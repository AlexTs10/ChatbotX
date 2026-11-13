//! `ecommerce-settings/service.ts` (get / update / dropPaymentMethod /
//! countProductsInOtherCurrency / resolveProFeatures) and the
//! `ecommerceSettingsSchema` checks of `database/partials/ecommerce.ts`.
//!
//! Settings are JSON objects with the four groups `general`, `payments`,
//! `shipping`, `checkout` (the TS `EcommerceSettings` shape, money in major
//! units); the row stores money x 10,000.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde_json::{Map, Value, json};

use crate::{BusinessError, Result, tx};

const MONEY_SCALE: f64 = 10_000.0;

const STORE_SOURCES: &[&str] = &["chatbotx", "shopify", "woocommerce"];
const PRODUCT_VIEWS: &[&str] = &[
    "list",
    "listWithoutImage",
    "card",
    "cardSquare",
    "image",
    "imageSquare",
];
const PRODUCT_BUTTONS: &[&str] = &["addToCart", "share", "similar", "save"];
const IMAGE_SIZES: &[&str] = &["bigSquare", "mediumRectangle"];
const PAYMENT_METHODS: &[&str] = &["cod", "stripe", "paypal"];
const CURRENCY_POSITIONS: &[&str] = &["left", "leftSpace", "right", "rightSpace"];
const SHIPPING_MODES: &[&str] = &["none", "delivery", "pickup", "deliveryAndPickup"];
const CHECKOUT_MODES: &[&str] = &["default", "externalLink", "flow", "productPage"];
const CONFIRMATION_MODES: &[&str] = &["default", "flow", "none"];
const EXTERNAL_CHECKOUT_METHODS: &[&str] = &["webCheckout", "inApp"];
const ABANDONED_CART_DELAY_MINUTES: &[i64] = &[
    5, 10, 15, 30, 45, 60, 120, 180, 240, 300, 360, 420, 480, 540, 600, 720, 840, 1080, 1200, 1440,
];
/// `PAYPAL_SUPPORTED_CURRENCIES`.
const PAYPAL_CURRENCIES: &[&str] = &[
    "AUD", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD", "HUF", "ILS", "JPY",
    "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "SEK", "SGD", "THB", "TWD", "USD",
];

/// One field check of the settings schema.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Enum(&'static [&'static str]),
    /// `z.string().regex(/^\d+$/).nullable()`.
    Id,
    /// `z.string().trim().max(n).nullable()`.
    Text(usize),
    Delay,
    Buttons,
    Methods,
    Currency,
    Country,
    Countries,
    Number(f64, Option<f64>),
    Link,
}

const GROUPS: &[(&str, &[(&str, Kind)])] = &[
    (
        "general",
        &[
            ("storeSource", Kind::Enum(STORE_SOURCES)),
            ("orderingAvailable", Kind::Bool),
            ("webshopProductView", Kind::Enum(PRODUCT_VIEWS)),
            ("openHoursId", Kind::Id),
            ("searchProducts", Kind::Bool),
            ("facebookPixelId", Kind::Text(64)),
            ("abandonedCartDelayMinutes", Kind::Delay),
            ("abandonedCartFlowId", Kind::Id),
            ("productButtons", Kind::Buttons),
            ("productImageSize", Kind::Enum(IMAGE_SIZES)),
            ("allowRateOrders", Kind::Bool),
            ("notifyOnCartStatusChange", Kind::Bool),
        ],
    ),
    (
        "payments",
        &[
            ("paymentMethods", Kind::Methods),
            ("currency", Kind::Currency),
            ("currencyPosition", Kind::Enum(CURRENCY_POSITIONS)),
            ("currencySymbol", Kind::Text(8)),
            ("serviceFeePercent", Kind::Number(0.0, Some(100.0))),
        ],
    ),
    (
        "shipping",
        &[
            ("shippingType", Kind::Enum(SHIPPING_MODES)),
            ("storeAddress", Kind::Text(1000)),
            ("shippingCountries", Kind::Countries),
            ("defaultShippingCountry", Kind::Country),
            ("deliveryCost", Kind::Number(0.0, None)),
        ],
    ),
    (
        "checkout",
        &[
            ("checkoutMode", Kind::Enum(CHECKOUT_MODES)),
            ("checkoutLink", Kind::Link),
            ("checkoutFlowId", Kind::Id),
            ("minimumOrderValue", Kind::Number(0.0, None)),
            ("orderConfirmationMode", Kind::Enum(CONFIRMATION_MODES)),
            ("orderConfirmationFlowId", Kind::Id),
            ("requireName", Kind::Bool),
            ("requireEmail", Kind::Bool),
            ("requirePhone", Kind::Bool),
            ("requireState", Kind::Bool),
            ("requireZip", Kind::Bool),
            (
                "externalCheckoutMethod",
                Kind::Enum(EXTERNAL_CHECKOUT_METHODS),
            ),
            ("externalAutoApplyDiscounts", Kind::Bool),
        ],
    ),
];

fn issue(issues: &mut Vec<Value>, path: &[&str], code: &str, message: &str) {
    issues.push(json!({ "code": code, "path": path, "message": message }));
}

fn trimmed(value: &Value) -> Option<String> {
    value.as_str().map(|text| text.trim().to_string())
}

fn country(value: &Value) -> Option<String> {
    trimmed(value)
        .filter(|code| code.chars().count() == 2)
        .map(|code| code.to_uppercase())
}

/// One field: the normalized value, or an issue.
fn check(kind: Kind, value: &Value, path: &[&str], issues: &mut Vec<Value>) -> Option<Value> {
    let nullable = matches!(
        kind,
        Kind::Id | Kind::Text(_) | Kind::Delay | Kind::Country | Kind::Link
    );
    if value.is_null() {
        if nullable {
            return Some(Value::Null);
        }
        issue(
            issues,
            path,
            "invalid_type",
            "Invalid input: expected a value, received null",
        );
        return None;
    }
    let out = match kind {
        Kind::Bool => value.as_bool().map(Value::Bool),
        Kind::Enum(options) => value
            .as_str()
            .filter(|text| options.contains(text))
            .map(|text| json!(text)),
        Kind::Id => value
            .as_str()
            .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .map(|text| json!(text)),
        Kind::Text(max) => trimmed(value)
            .filter(|text| text.chars().count() <= max)
            .map(Value::String),
        Kind::Delay => value
            .as_f64()
            .filter(|minutes| minutes.fract() == 0.0)
            .map(|minutes| minutes as i64)
            .filter(|minutes| ABANDONED_CART_DELAY_MINUTES.contains(minutes))
            .map(|minutes| json!(minutes)),
        Kind::Buttons => value.as_array().and_then(|items| {
            let names: Option<Vec<&str>> = items
                .iter()
                .map(|item| item.as_str().filter(|t| PRODUCT_BUTTONS.contains(t)))
                .collect();
            let names = names?;
            (!names.is_empty() && names.len() <= PRODUCT_BUTTONS.len()).then(|| {
                json!(
                    PRODUCT_BUTTONS
                        .iter()
                        .filter(|button| names.contains(button))
                        .collect::<Vec<_>>()
                )
            })
        }),
        Kind::Methods => value.as_array().and_then(|items| {
            let names: Option<Vec<Value>> = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|t| PAYMENT_METHODS.contains(t))
                        .map(|t| json!(t))
                })
                .collect();
            names.filter(|names| !names.is_empty()).map(Value::Array)
        }),
        Kind::Currency => trimmed(value)
            .filter(|code| code.chars().count() == 3)
            .map(|code| json!(code.to_uppercase())),
        Kind::Country => country(value).map(Value::String),
        Kind::Countries => value.as_array().and_then(|items| {
            let codes: Option<Vec<Value>> = items
                .iter()
                .map(|item| country(item).map(Value::String))
                .collect();
            codes.filter(|codes| codes.len() <= 300).map(Value::Array)
        }),
        Kind::Number(min, max) => value
            .as_f64()
            .filter(|n| *n >= min && max.is_none_or(|max| *n <= max))
            .map(js_number),
        Kind::Link => trimmed(value)
            .filter(|text| text.len() <= 2048)
            .filter(|text| {
                url::Url::parse(text).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            })
            .map(Value::String),
    };
    if out.is_none() {
        issue(issues, path, "invalid_value", "Invalid input");
    }
    out
}

/// A JS number (`10`, not `10.0`).
fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// `ecommerceSettingsSchema.parse` (`partial` = the `.partial()` groups of
/// the public request: every key optional).
fn parse_groups(input: &Value, partial: bool) -> std::result::Result<Value, Vec<Value>> {
    let mut issues = Vec::new();
    let mut out = Map::new();
    for (group, fields) in GROUPS {
        let Some(value) = input.get(*group) else {
            if !partial {
                issue(
                    &mut issues,
                    &[group],
                    "invalid_type",
                    "Invalid input: expected object",
                );
            }
            continue;
        };
        let Some(object) = value.as_object() else {
            issue(
                &mut issues,
                &[group],
                "invalid_type",
                "Invalid input: expected object",
            );
            continue;
        };
        let mut parsed = Map::new();
        for (name, kind) in *fields {
            match object.get(*name) {
                Some(value) => {
                    if let Some(value) = check(*kind, value, &[group, name], &mut issues) {
                        parsed.insert((*name).to_string(), value);
                    }
                }
                None if partial => {}
                None => issue(
                    &mut issues,
                    &[group, name],
                    "invalid_type",
                    "Invalid input: expected a value, received undefined",
                ),
            }
        }
        if partial && *group == "checkout" {
            // `shopifyCheckoutMethod` / `shopifyAutoApplyDiscounts` (deprecated
            // aliases; the new keys win).
            for (alias, target, kind) in [
                (
                    "shopifyCheckoutMethod",
                    "externalCheckoutMethod",
                    Kind::Enum(EXTERNAL_CHECKOUT_METHODS),
                ),
                (
                    "shopifyAutoApplyDiscounts",
                    "externalAutoApplyDiscounts",
                    Kind::Bool,
                ),
            ] {
                if let Some(value) = object.get(alias)
                    && let Some(value) = check(kind, value, &[group, alias], &mut issues)
                    && !parsed.contains_key(target)
                {
                    parsed.insert(target.to_string(), value);
                }
            }
        }
        out.insert((*group).to_string(), Value::Object(parsed));
    }
    if issues.is_empty() {
        Ok(Value::Object(out))
    } else {
        Err(issues)
    }
}

fn money(scaled: i64) -> Value {
    js_number(scaled as f64 / MONEY_SCALE)
}

fn to_money(value: &Value) -> i64 {
    (value.as_f64().unwrap_or_default() * MONEY_SCALE).round() as i64
}

fn opt_id(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |id| json!(id.to_string()))
}

fn id_of(value: &Value) -> Option<i64> {
    value.as_str().and_then(|text| text.parse().ok())
}

fn json_list(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!([]))
}

const COLUMNS: &str = r#""storeSource", "orderingAvailable", "webshopProductView", "openHoursId",
    "searchProducts", "facebookPixelId", "abandonedCartDelayMinutes", "abandonedCartFlowId",
    "productButtons", "productImageSize", "allowRateOrders", "notifyOnCartStatusChange",
    "paymentMethods", "currency", "currencyPosition", "currencySymbol", "serviceFeePercent",
    "shippingType", "storeAddress", "shippingCountries", "defaultShippingCountry", "deliveryCost",
    "checkoutMode", "checkoutLink", "checkoutFlowId", "minimumOrderValue",
    "orderConfirmationMode", "orderConfirmationFlowId", "requireName", "requireEmail",
    "requirePhone", "requireState", "requireZip", "externalCheckoutMethod",
    "externalAutoApplyDiscounts""#;

/// `toSettings`.
fn row(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<Value> {
    Ok(json!({
        "general": {
            "storeSource": r.get::<_, String>(0)?,
            "orderingAvailable": r.get::<_, bool>(1)?,
            "webshopProductView": r.get::<_, String>(2)?,
            "openHoursId": opt_id(r.get(3)?),
            "searchProducts": r.get::<_, bool>(4)?,
            "facebookPixelId": r.get::<_, Option<String>>(5)?,
            "abandonedCartDelayMinutes": r.get::<_, Option<i64>>(6)?,
            "abandonedCartFlowId": opt_id(r.get(7)?),
            "productButtons": json_list(&r.get::<_, String>(8)?),
            "productImageSize": r.get::<_, String>(9)?,
            "allowRateOrders": r.get::<_, bool>(10)?,
            "notifyOnCartStatusChange": r.get::<_, bool>(11)?,
        },
        "payments": {
            "paymentMethods": json_list(&r.get::<_, String>(12)?),
            "currency": r.get::<_, String>(13)?,
            "currencyPosition": r.get::<_, String>(14)?,
            "currencySymbol": r.get::<_, Option<String>>(15)?,
            "serviceFeePercent": money(r.get(16)?),
        },
        "shipping": {
            "shippingType": r.get::<_, String>(17)?,
            "storeAddress": r.get::<_, Option<String>>(18)?,
            "shippingCountries": json_list(&r.get::<_, String>(19)?),
            "defaultShippingCountry": r.get::<_, Option<String>>(20)?,
            "deliveryCost": money(r.get(21)?),
        },
        "checkout": {
            "checkoutMode": r.get::<_, String>(22)?,
            "checkoutLink": r.get::<_, Option<String>>(23)?,
            "checkoutFlowId": opt_id(r.get(24)?),
            "minimumOrderValue": money(r.get(25)?),
            "orderConfirmationMode": r.get::<_, String>(26)?,
            "orderConfirmationFlowId": opt_id(r.get(27)?),
            "requireName": r.get::<_, bool>(28)?,
            "requireEmail": r.get::<_, bool>(29)?,
            "requirePhone": r.get::<_, bool>(30)?,
            "requireState": r.get::<_, bool>(31)?,
            "requireZip": r.get::<_, bool>(32)?,
            "externalCheckoutMethod": r.get::<_, String>(33)?,
            "externalAutoApplyDiscounts": r.get::<_, bool>(34)?,
        },
    }))
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn find(conn: &Connection, workspace_id: i64) -> Result<Option<Value>> {
    conn.query_row(
        &format!(r#"SELECT {COLUMNS} FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#),
        [workspace_id],
        row,
    )
    .optional()
    .map_err(repo_err)
}

/// `ecommerceSettingsRepository.upsert` (every column).
fn upsert(conn: &Connection, workspace_id: i64, s: &Value) -> Result<()> {
    let (g, p, sh, c) = (
        &s["general"],
        &s["payments"],
        &s["shipping"],
        &s["checkout"],
    );
    let text = |v: &Value| -> SqlValue {
        v.as_str()
            .map_or(SqlValue::Null, |t| SqlValue::Text(t.to_string()))
    };
    let id = |v: &Value| -> SqlValue { id_of(v).map_or(SqlValue::Null, SqlValue::Integer) };
    let flag =
        |v: &Value| -> SqlValue { SqlValue::Integer(i64::from(v.as_bool().unwrap_or_default())) };
    let list = |v: &Value| -> SqlValue { SqlValue::Text(v.to_string()) };
    let values: Vec<SqlValue> = vec![
        text(&g["storeSource"]),
        flag(&g["orderingAvailable"]),
        text(&g["webshopProductView"]),
        id(&g["openHoursId"]),
        flag(&g["searchProducts"]),
        text(&g["facebookPixelId"]),
        g["abandonedCartDelayMinutes"]
            .as_i64()
            .map_or(SqlValue::Null, SqlValue::Integer),
        id(&g["abandonedCartFlowId"]),
        list(&g["productButtons"]),
        text(&g["productImageSize"]),
        flag(&g["allowRateOrders"]),
        flag(&g["notifyOnCartStatusChange"]),
        list(&p["paymentMethods"]),
        text(&p["currency"]),
        text(&p["currencyPosition"]),
        text(&p["currencySymbol"]),
        SqlValue::Integer(to_money(&p["serviceFeePercent"])),
        text(&sh["shippingType"]),
        text(&sh["storeAddress"]),
        list(&sh["shippingCountries"]),
        text(&sh["defaultShippingCountry"]),
        SqlValue::Integer(to_money(&sh["deliveryCost"])),
        text(&c["checkoutMode"]),
        text(&c["checkoutLink"]),
        id(&c["checkoutFlowId"]),
        SqlValue::Integer(to_money(&c["minimumOrderValue"])),
        text(&c["orderConfirmationMode"]),
        id(&c["orderConfirmationFlowId"]),
        flag(&c["requireName"]),
        flag(&c["requireEmail"]),
        flag(&c["requirePhone"]),
        flag(&c["requireState"]),
        flag(&c["requireZip"]),
        text(&c["externalCheckoutMethod"]),
        flag(&c["externalAutoApplyDiscounts"]),
    ];
    let columns: Vec<&str> = COLUMNS.split(',').map(str::trim).collect();
    let placeholders = (0..columns.len())
        .map(|i| format!("?{}", i + 3))
        .collect::<Vec<_>>()
        .join(", ");
    let updates = columns
        .iter()
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut binds: Vec<SqlValue> = vec![
        SqlValue::Integer(
            chatbotx_core::utils::id::create_id()
                .parse()
                .unwrap_or_default(),
        ),
        SqlValue::Integer(workspace_id),
    ];
    binds.extend(values);
    conn.execute(
        &format!(
            r#"INSERT INTO "EcommerceSettings" ("id", "workspaceId", {COLUMNS})
               VALUES (?1, ?2, {placeholders})
               ON CONFLICT ("workspaceId") DO UPDATE SET {updates},
                   "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)"#
        ),
        chatbotx_db::rusqlite::params_from_iter(binds.iter()),
    )
    .map_err(repo_err)?;
    Ok(())
}

fn exists(conn: &Connection, sql: &str, id: i64, workspace_id: i64) -> Result<bool> {
    conn.query_row(sql, params![id, workspace_id], |_| Ok(()))
        .optional()
        .map(|found| found.is_some())
        .map_err(repo_err)
}

/// `flowService.exists`.
fn flow_exists(conn: &Connection, workspace_id: i64, flow_id: &Value) -> Result<bool> {
    match id_of(flow_id) {
        Some(id) => exists(
            conn,
            r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            id,
            workspace_id,
        ),
        None => Ok(false),
    }
}

fn invalid(field: &str, message: &str) -> BusinessError {
    BusinessError::validation(field, message)
}

fn with_data(message: String, code: &str, data: Value) -> BusinessError {
    match BusinessError::exception(message, code, 400) {
        BusinessError::Exception {
            message,
            code,
            status,
            field,
            ..
        } => BusinessError::Exception {
            message,
            code,
            status,
            field,
            data: Some(data),
        },
        other => other,
    }
}

/// `assertCoherent`.
fn assert_coherent(conn: &Connection, workspace_id: i64, s: &Value, patch: &Value) -> Result<()> {
    let sent = |group: &str, key: &str| patch.get(group).and_then(|g| g.get(key)).is_some();
    let (general, shipping, checkout) = (&s["general"], &s["shipping"], &s["checkout"]);
    let touches_target = ["checkoutMode", "checkoutLink", "checkoutFlowId"]
        .iter()
        .any(|k| sent("checkout", k));
    let touches_confirmation = ["orderConfirmationMode", "orderConfirmationFlowId"]
        .iter()
        .any(|k| sent("checkout", k));
    let touches_shipping = patch
        .get("shipping")
        .and_then(Value::as_object)
        .is_some_and(|g| !g.is_empty());
    if sent("general", "openHoursId")
        && let Some(id) = id_of(&general["openHoursId"])
        && !exists(
            conn,
            r#"SELECT 1 FROM "EcommerceOpenHours" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            id,
            workspace_id,
        )?
    {
        return Err(BusinessError::not_found(
            "Open-hours schedule does not exist.",
        ));
    }
    if touches_target
        && checkout["checkoutMode"] == "externalLink"
        && checkout["checkoutLink"].is_null()
    {
        return Err(invalid(
            "checkoutLink",
            "A checkout link is required for the external link mode.",
        ));
    }
    if touches_target && checkout["checkoutMode"] == "flow" {
        if checkout["checkoutFlowId"].is_null() {
            return Err(invalid(
                "checkoutFlowId",
                "A checkout flow is required for the flow mode.",
            ));
        }
        if !flow_exists(conn, workspace_id, &checkout["checkoutFlowId"])? {
            return Err(invalid(
                "checkoutFlowId",
                "The checkout flow does not exist.",
            ));
        }
    }
    if touches_confirmation && checkout["orderConfirmationMode"] == "flow" {
        if checkout["orderConfirmationFlowId"].is_null() {
            return Err(invalid(
                "orderConfirmationFlowId",
                "An order confirmation flow is required for the flow mode.",
            ));
        }
        if !flow_exists(conn, workspace_id, &checkout["orderConfirmationFlowId"])? {
            return Err(invalid(
                "orderConfirmationFlowId",
                "The order confirmation flow does not exist.",
            ));
        }
    }
    if sent("general", "abandonedCartFlowId")
        && !general["abandonedCartFlowId"].is_null()
        && !flow_exists(conn, workspace_id, &general["abandonedCartFlowId"])?
    {
        return Err(invalid(
            "abandonedCartFlowId",
            "The abandoned cart flow does not exist.",
        ));
    }
    let countries = shipping["shippingCountries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if touches_shipping
        && !shipping["defaultShippingCountry"].is_null()
        && !countries.is_empty()
        && !countries.contains(&shipping["defaultShippingCountry"])
    {
        return Err(invalid(
            "defaultShippingCountry",
            "The default shipping country must be one of the shipping countries.",
        ));
    }
    let pickup = matches!(
        shipping["shippingType"].as_str(),
        Some("pickup" | "deliveryAndPickup")
    );
    if touches_shipping && pickup && shipping["storeAddress"].is_null() {
        return Err(invalid(
            "storeAddress",
            "A store address is required for pickup.",
        ));
    }
    Ok(())
}

/// `assertPaymentMethodsUsable`.
fn assert_payment_methods_usable(conn: &Connection, workspace_id: i64, s: &Value) -> Result<()> {
    let online: Vec<&str> = s["payments"]["paymentMethods"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|m| *m != "cod")
                .collect()
        })
        .unwrap_or_default();
    if online.is_empty() {
        return Ok(());
    }
    let mut statement = conn
        .prepare(r#"SELECT "provider" FROM "EcommercePaymentProvider" WHERE "workspaceId" = ?1"#)
        .map_err(repo_err)?;
    let connected = statement
        .query_map([workspace_id], |r| r.get::<_, String>(0))
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    if let Some(missing) = online.iter().find(|p| !connected.iter().any(|c| c == *p)) {
        return Err(with_data(
            format!("Connect {missing} in Settings → Integrations first."),
            "paymentProviderNotConnected",
            json!({ "provider": missing }),
        ));
    }
    let currency = s["payments"]["currency"]
        .as_str()
        .unwrap_or_default()
        .to_uppercase();
    if online.contains(&"paypal") && !PAYPAL_CURRENCIES.contains(&currency.as_str()) {
        return Err(with_data(
            format!("paypal does not take payments in {currency}."),
            "paymentCurrencyUnsupported",
            json!({ "provider": "paypal", "currency": currency }),
        ));
    }
    Ok(())
}

/// `assertExternalStoreConnected` (the store presence; its currency read is
/// not ported).
fn assert_external_store_connected(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
) -> Result<()> {
    let (table, name) = if source == "shopify" {
        ("IntegrationShopify", "Shopify")
    } else {
        ("IntegrationWoocommerce", "WooCommerce")
    };
    let found = conn
        .query_row(
            &format!(r#"SELECT 1 FROM "{table}" WHERE "workspaceId" = ?1"#),
            [workspace_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(repo_err)?;
    if found.is_none() {
        return Err(with_data(
            format!(
                "Connect a {name} store in Ecommerce → Settings → {name} before selling from it."
            ),
            "externalStoreNotConnected",
            json!({ "store": source }),
        ));
    }
    Ok(())
}

/// `ECOMMERCE_SETTINGS_DEFAULTS`.
pub fn defaults() -> Value {
    json!({
        "general": {
            "storeSource": "chatbotx",
            "orderingAvailable": true,
            "webshopProductView": "list",
            "openHoursId": null,
            "searchProducts": false,
            "facebookPixelId": null,
            "abandonedCartDelayMinutes": null,
            "abandonedCartFlowId": null,
            "productButtons": ["addToCart", "share", "similar"],
            "productImageSize": "bigSquare",
            "allowRateOrders": false,
            "notifyOnCartStatusChange": true,
        },
        "payments": {
            "paymentMethods": ["cod"],
            "currency": "USD",
            "currencyPosition": "left",
            "currencySymbol": "$",
            "serviceFeePercent": 0,
        },
        "shipping": {
            "shippingType": "delivery",
            "storeAddress": null,
            "shippingCountries": [],
            "defaultShippingCountry": null,
            "deliveryCost": 0,
        },
        "checkout": {
            "checkoutMode": "default",
            "checkoutLink": null,
            "checkoutFlowId": null,
            "minimumOrderValue": 0,
            "orderConfirmationMode": "default",
            "orderConfirmationFlowId": null,
            "requireName": true,
            "requireEmail": false,
            "requirePhone": true,
            "requireState": false,
            "requireZip": false,
            "externalCheckoutMethod": "webCheckout",
            "externalAutoApplyDiscounts": true,
        },
    })
}

/// `resolveProFeatures` (community edition: every feature on).
pub fn pro_features() -> Value {
    json!({ "openHours": true, "searchProducts": true, "facebookPixel": true, "abandonedCart": true })
}

/// `updateEcommerceSettingsPublicRequest` + `toEcommerceSettingsPatch`: the
/// partial groups, normalized (codes upper-case, buttons ordered); `Err` =
/// the zod issues (paths like `["payments", "currency"]`).
pub fn parse_patch(input: &Value) -> std::result::Result<Value, Vec<Value>> {
    parse_groups(input, true)
}

/// `ecommerceSettingsService.get` (no cache): the stored settings or the
/// defaults.
pub async fn get(db: &Db, workspace_id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        Ok(find(conn, workspace_id)?.unwrap_or_else(defaults))
    })
    .await
}

/// `ecommerceSettingsService.update`: in one transaction, insert the default
/// row when missing, merge the patch groups, check the merged settings
/// (schema, external store, coherence, online payment methods) and write
/// every column. Returns the merged settings.
pub async fn update(db: &Db, workspace_id: i64, patch: Value) -> Result<Value> {
    tx::write(db, move |tx| {
        // `assertReferencable`: every flow the patch names belongs to the
        // workspace (`404 Flow not found`), before any other check.
        for (group, key) in [
            ("general", "abandonedCartFlowId"),
            ("checkout", "checkoutFlowId"),
            ("checkout", "orderConfirmationFlowId"),
        ] {
            let value = &patch[group][key];
            if value.is_string() && !flow_exists(tx, workspace_id, value)? {
                return Err(BusinessError::not_found("Flow not found"));
            }
        }
        let current = match find(tx, workspace_id)? {
            Some(current) => current,
            None => {
                let defaults = defaults();
                upsert(tx, workspace_id, &defaults)?;
                defaults
            }
        };
        let mut merged = current.clone();
        for (group, _) in GROUPS {
            if let Some(Value::Object(fields)) = patch.get(*group) {
                for (key, value) in fields {
                    merged[*group][key] = value.clone();
                }
            }
        }
        let next = parse_groups(&merged, false).map_err(|issues| BusinessError::Exception {
            message: "Invalid ecommerce settings".to_string(),
            code: "invalidRequestData".to_string(),
            status: 422,
            field: None,
            data: Some(json!({ "issues": issues })),
        })?;
        let source = next["general"]["storeSource"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if patch
            .get("general")
            .and_then(|g| g.get("storeSource"))
            .is_some()
            && source != "chatbotx"
        {
            assert_external_store_connected(tx, workspace_id, &source)?;
        }
        assert_coherent(tx, workspace_id, &next, &patch)?;
        if patch
            .get("payments")
            .and_then(Value::as_object)
            .is_some_and(|g| !g.is_empty())
        {
            assert_payment_methods_usable(tx, workspace_id, &next)?;
        }
        upsert(tx, workspace_id, &next)?;
        Ok(next)
    })
    .await
}

/// `ecommerceSettingsService.dropPaymentMethod`: remove `method`; an empty
/// list becomes `["cod"]`.
pub async fn drop_payment_method(db: &Db, workspace_id: i64, method: &str) -> Result<()> {
    let method = method.to_string();
    tx::write(db, move |tx| {
        let Some(mut current) = find(tx, workspace_id)? else {
            return Ok(());
        };
        let methods: Vec<Value> = current["payments"]["paymentMethods"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !methods.iter().any(|m| m == method.as_str()) {
            return Ok(());
        }
        let remaining: Vec<Value> = methods
            .into_iter()
            .filter(|m| m != method.as_str())
            .collect();
        current["payments"]["paymentMethods"] = if remaining.is_empty() {
            json!(["cod"])
        } else {
            Value::Array(remaining)
        };
        upsert(tx, workspace_id, &current)
    })
    .await
}

/// `ecommerceSettingsRepository.countProductsInOtherCurrency`.
pub async fn count_products_in_other_currency(
    db: &Db,
    workspace_id: i64,
    currency: &str,
) -> Result<i64> {
    let currency = currency.to_string();
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT COUNT(*) FROM "Product" WHERE "workspaceId" = ?1 AND upper("currency") <> upper(?2)"#,
            params![workspace_id, currency],
            |r| r.get(0),
        )
        .map_err(repo_err)
    })
    .await
}
