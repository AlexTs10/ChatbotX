//! The two `discountCodeRepository` writes the catalog services run in their
//! delete transactions (private queries; they move to `crates/db` later).

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::errors::{BusinessError, Result};

fn id_list(ids: &[i64]) -> String {
    serde_json::Value::from(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>()).to_string()
}

/// `deactivateByCategoryIds`: switches off the `category` codes of these categories.
pub fn deactivate_by_category_ids(
    conn: &Connection,
    workspace_id: i64,
    category_ids: &[i64],
) -> Result<()> {
    if category_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        r#"UPDATE "DiscountCode" SET "isActive" = 0 WHERE "workspaceId" = ?1 AND "appliesTo" = 'category'
           AND "categoryId" IN (SELECT CAST(value AS INTEGER) FROM json_each(?2))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(category_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}

/// `removeProductIds`: drops the ids from `products` codes; a code left with
/// no product is switched off.
pub fn remove_product_ids(conn: &Connection, workspace_id: i64, product_ids: &[i64]) -> Result<()> {
    if product_ids.is_empty() {
        return Ok(());
    }
    // `productIds` holds the ids as JSON strings (or numbers); compare as text.
    conn.execute(
        r#"UPDATE "DiscountCode" SET
             "productIds" = coalesce((SELECT json_group_array(l.value) FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2))), '[]'),
             "isActive" = CASE WHEN EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2)))
                               THEN "isActive" ELSE 0 END
           WHERE "workspaceId" = ?1 AND "appliesTo" = 'products'
             AND EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                         WHERE CAST(l.value AS TEXT) IN (SELECT value FROM json_each(?2)))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(product_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}

// ---- discount-code/{service,pricing,errors}.ts: pricing a code on an order --

use chatbotx_core::utils::money::{percent_of, round_money, sum_money};
use chatbotx_db::repo::models::DiscountCode;
use chatbotx_db::repo::queries::contact_filter::{
    ContactWhereInput, build_contact_where, contact_filter_has_predicate,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{find_many, find_one, iso, new_id, now_ms};
use serde_json::json;

const MONEY_SCALE: f64 = 10_000.0;

fn major(scaled: i64) -> f64 {
    scaled as f64 / MONEY_SCALE
}

fn discount_error(message: impl Into<String>, code: &str) -> BusinessError {
    BusinessError::exception(message, code, 400)
}

pub fn not_found() -> BusinessError {
    BusinessError::exception(
        "This discount code does not exist.",
        "discountCodeNotFound",
        404,
    )
}

fn not_applicable() -> BusinessError {
    discount_error(
        "This discount code does not apply to anything in the order.",
        "discountCodeNotApplicable",
    )
}

fn used_up() -> BusinessError {
    discount_error(
        "This discount code has been used up.",
        "discountCodeUsageLimitReached",
    )
}

/// `CouponLine` (major units).
#[derive(Debug, Clone, PartialEq)]
pub struct CouponLine {
    pub product_id: Option<i64>,
    pub category_ids: Vec<i64>,
    pub amount: f64,
    pub quantity: i64,
}

/// `DiscountOrderContext`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderContext {
    pub workspace_id: i64,
    pub source: String,
    pub contact_id: Option<i64>,
    pub lines: Vec<CouponLine>,
    pub shipping_type: Option<String>,
    pub shipping_total: f64,
    pub currency: String,
}

/// `AppliedDiscount` (what an order keeps of the code).
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub id: i64,
    pub code: String,
    pub discount_type: String,
    pub discount_value: f64,
    pub ends_at: Option<i64>,
    pub amount: f64,
}

/// `assertCouponUsable`.
fn assert_usable(code: &DiscountCode, now: i64) -> Result<()> {
    if !code.is_active {
        return Err(discount_error(
            "This discount code is turned off.",
            "discountCodeInactive",
        ));
    }
    if code.starts_at > now {
        return Err(discount_error(
            "This discount code is not valid yet.",
            "discountCodeNotStarted",
        )
        .with_data(json!({ "startsAt": iso(code.starts_at) })));
    }
    if code.ends_at.is_some_and(|end| end <= now) {
        return Err(discount_error(
            "This discount code has expired.",
            "discountCodeExpired",
        ));
    }
    if code
        .usage_limit
        .is_some_and(|limit| code.used_count >= limit)
    {
        return Err(used_up());
    }
    Ok(())
}

fn product_ids(code: &DiscountCode) -> Vec<i64> {
    code.product_ids
        .0
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.as_i64())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `computeCouponDiscount`.
pub fn compute(code: &DiscountCode, order: &OrderContext) -> Result<f64> {
    let currency = order.currency.as_str();
    let lines: Vec<&CouponLine> =
        if code.discount_type == "freeShipping" || code.applies_to == "entireOrder" {
            order.lines.iter().collect()
        } else if code.applies_to == "category" {
            match code.category_id {
                Some(category) => order
                    .lines
                    .iter()
                    .filter(|l| l.category_ids.contains(&category))
                    .collect(),
                None => Vec::new(),
            }
        } else {
            let ids = product_ids(code);
            order
                .lines
                .iter()
                .filter(|l| l.product_id.is_some_and(|p| ids.contains(&p)))
                .collect()
        };
    if lines.is_empty() {
        return Err(not_applicable());
    }
    let amount = sum_money(
        &lines.iter().map(|l| l.amount).collect::<Vec<_>>(),
        currency,
    );
    let quantity: i64 = lines.iter().map(|l| l.quantity).sum();
    let minimum = major(code.minimum_value);
    let js = crate::payments::js_number;
    if code.minimum_type == "amount" && amount < minimum {
        return Err(discount_error(
            format!("This discount code needs a purchase of at least {}.", js(minimum)),
            "discountCodeMinimumNotMet",
        )
        .with_data(json!({ "minimumType": "amount", "minimum": minimum, "missing": round_money(minimum - amount, currency) })));
    }
    if code.minimum_type == "quantity" && (quantity as f64) < minimum {
        return Err(discount_error(
            format!("This discount code needs at least {} items.", js(minimum)),
            "discountCodeMinimumNotMet",
        )
        .with_data(json!({ "minimumType": "quantity", "minimum": minimum, "missing": minimum - quantity as f64 })));
    }
    let value = major(code.discount_value);
    match code.discount_type.as_str() {
        "percentage" => Ok(percent_of(amount, value, currency)),
        "fixedAmount" => Ok(round_money(value.min(amount), currency)),
        "freeShipping"
            if order.shipping_type.as_deref() == Some("delivery") && order.shipping_total > 0.0 =>
        {
            Ok(order.shipping_total)
        }
        _ => Err(not_applicable()),
    }
}

/// `assertEligible`.
fn assert_eligible(conn: &Connection, code: &DiscountCode, order: &OrderContext) -> Result<()> {
    if code.eligibility != "specificCustomers" && !code.once_per_customer {
        return Ok(());
    }
    let not_eligible = || {
        discount_error(
            "This discount code is not available to this customer.",
            "discountCodeNotEligible",
        )
    };
    let Some(contact_id) = order.contact_id else {
        return Err(not_eligible());
    };
    if code.eligibility == "specificCustomers" {
        // ponytail: `withWorkingHoursTimezone` (working-hours conditions in
        // the workspace zone) is not applied here; they read as UTC.
        let criteria = code
            .customer_filter
            .as_ref()
            .map(|f| crate::contact_filter::to_filter_criteria(&f.0));
        let matches = match criteria {
            Some(c)
                if c.conditions.is_empty()
                    || contact_filter_has_predicate(&c, Some(order.workspace_id)) =>
            {
                let mut sql = Sql::raw(r#"SELECT 1 FROM "Contact" WHERE "Contact"."id" = "#);
                sql.push_bind(contact_id);
                sql.push(" AND ");
                sql.append(build_contact_where(&ContactWhereInput {
                    workspace_id: order.workspace_id,
                    keyword: None,
                    contact_filter: Some(c),
                    now: now_ms(),
                }));
                sql.push(" LIMIT 1");
                !sql.query_map(conn, |r| r.get::<_, i64>(0))?.is_empty()
            }
            _ => false,
        };
        if !matches {
            return Err(not_eligible());
        }
    }
    if code.once_per_customer {
        let used: i64 = conn.query_row(
            r#"SELECT COUNT(*) FROM "DiscountCodeRedemption" WHERE "discountCodeId" = ?1 AND "contactId" = ?2"#,
            params![code.id, contact_id],
            |r| r.get(0),
        )?;
        if used > 0 {
            return Err(discount_error(
                "This discount code can be used once per customer.",
                "discountCodeAlreadyUsed",
            ));
        }
    }
    Ok(())
}

/// `price`: the code re-read (the one writer is the lock), usable, eligible.
fn price(
    conn: &Connection,
    code: &DiscountCode,
    order: &OrderContext,
    now: i64,
) -> Result<Applied> {
    let current: DiscountCode = find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&code.id, &order.workspace_id],
    )?
    .ok_or_else(not_found)?;
    assert_usable(&current, now)?;
    assert_eligible(conn, &current, order)?;
    let amount = compute(&current, order)?;
    Ok(Applied {
        id: current.id,
        code: current.code.clone(),
        discount_type: current.discount_type.clone(),
        discount_value: major(current.discount_value),
        ends_at: current.ends_at,
        amount,
    })
}

/// `findByCode` (case-insensitive, the store's source).
pub fn find_by_code(
    conn: &Connection,
    workspace_id: i64,
    code: &str,
    source: &str,
) -> Result<Option<DiscountCode>> {
    Ok(find_one(
        conn,
        r#""workspaceId" = ?1 AND "source" = ?2 AND lower("code") = lower(?3)"#,
        &[&workspace_id, &source, &code.trim()],
    )?)
}

/// `resolve`: the typed code (refused when unusable), else the automatic
/// code that takes off the most.
pub fn resolve(
    conn: &Connection,
    order: &OrderContext,
    code: Option<&str>,
    auto_apply: bool,
    now: i64,
) -> Result<Option<Applied>> {
    if let Some(typed) = code.map(str::trim).filter(|c| !c.is_empty()) {
        let found =
            find_by_code(conn, order.workspace_id, typed, &order.source)?.ok_or_else(not_found)?;
        return price(conn, &found, order, now).map(Some);
    }
    if !auto_apply {
        return Ok(None);
    }
    let candidates: Vec<DiscountCode> = find_many(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "source" = ?2 AND "autoApply" = 1 AND "isActive" = 1
           AND "startsAt" <= ?3 AND ("endsAt" IS NULL OR "endsAt" > ?3)
           AND ("usageLimit" IS NULL OR "usedCount" < "usageLimit")
           ORDER BY "createdAt" ASC, "id" ASC"#,
        &[&order.workspace_id, &order.source, &now],
    )?;
    let mut best: Option<Applied> = None;
    for candidate in &candidates {
        match price(conn, candidate, order, now) {
            Ok(applied)
                if applied.amount > 0.0
                    && best.as_ref().is_none_or(|b| applied.amount > b.amount) =>
            {
                best = Some(applied);
            }
            Ok(_) | Err(BusinessError::Exception { .. }) => {}
            Err(other) => return Err(other),
        }
    }
    Ok(best)
}

/// `redeem`: counted against the limit, one redemption row.
pub fn redeem(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    contact_id: Option<i64>,
    applied: &Applied,
) -> Result<()> {
    let counted = conn.execute(
        r#"UPDATE "DiscountCode" SET "usedCount" = "usedCount" + 1, "updatedAt" = ?2
           WHERE "id" = ?1 AND ("usageLimit" IS NULL OR "usedCount" < "usageLimit")"#,
        params![applied.id, now_ms()],
    )?;
    if counted == 0 {
        return Err(used_up());
    }
    conn.execute(
        r#"INSERT INTO "DiscountCodeRedemption" ("id","workspaceId","discountCodeId","orderId","contactId","amount")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
        params![new_id(), workspace_id, applied.id, order_id, contact_id, (applied.amount * MONEY_SCALE).round() as i64],
    )?;
    Ok(())
}

/// `release`: an order's redemption goes, and the code's count with it.
pub fn release(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<()> {
    let code: Option<i64> = conn
        .query_row(
            r#"DELETE FROM "DiscountCodeRedemption" WHERE "workspaceId" = ?1 AND "orderId" = ?2 RETURNING "discountCodeId""#,
            params![workspace_id, order_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(code) = code {
        conn.execute(
            r#"UPDATE "DiscountCode" SET "usedCount" = max("usedCount" - 1, 0), "updatedAt" = ?2 WHERE "id" = ?1"#,
            params![code, now_ms()],
        )?;
    }
    Ok(())
}

// ---- discount-code/service.ts: list / get / generateCode / create / update /
// setActive / delete -----------------------------------------------------------

use chatbotx_db::repo::Model;
use chatbotx_db::repo::utils::{PaginationInput, like_contains, parse_pagination};
use serde_json::{Map, Value};

use crate::store_source::{external_store_managed, is_external_store_source, resolve_store_source};
use crate::tx;

/// `CODE_PATTERN`.
pub fn is_code(code: &str) -> bool {
    (3..=64).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// No 0/O or 1/I/l, so a code read aloud or off a screen is typed right.
const GENERATED_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

fn record_not_found() -> BusinessError {
    BusinessError::not_found("Discount code does not exist.")
}

fn duplicated() -> BusinessError {
    BusinessError::exception(
        "A discount code with this name already exists.",
        "discountCodeDuplicated",
        409,
    )
}

fn scaled(major: f64) -> i64 {
    (major * MONEY_SCALE).round() as i64
}

fn number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, |n| {
        if value.fract() == 0.0 && value.abs() < 9e15 {
            json!(value as i64)
        } else {
            Value::Number(n)
        }
    })
}

/// `DiscountCodeInput` as camelCase JSON (major units, ms dates):
/// `toInput(row)` / what the API sends, merged field by field.
pub fn to_input(row: &DiscountCode) -> Map<String, Value> {
    let Value::Object(map) = json!({
        "code": row.code,
        "autoApply": row.auto_apply,
        "discountType": row.discount_type,
        "discountValue": major(row.discount_value),
        "appliesTo": row.applies_to,
        "categoryId": row.category_id,
        "productIds": product_ids(row),
        "minimumType": row.minimum_type,
        "minimumValue": major(row.minimum_value),
        "usageLimit": row.usage_limit,
        "oncePerCustomer": row.once_per_customer,
        "eligibility": row.eligibility,
        "customerFilter": row.customer_filter.as_ref().map(|j| j.0.clone()),
        "startsAt": row.starts_at,
        "endsAt": row.ends_at,
        "isActive": row.is_active,
    }) else {
        unreachable!()
    };
    map
}

/// The public resource (`publicDiscountCodeResource`).
pub fn resource_json(row: &DiscountCode) -> Value {
    json!({
        "id": row.id.to_string(),
        "code": row.code,
        "autoApply": row.auto_apply,
        "discountType": row.discount_type,
        "discountValue": number(major(row.discount_value)),
        "appliesTo": row.applies_to,
        "categoryId": row.category_id.map(|id| id.to_string()),
        "productIds": product_ids(row).iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "minimumType": row.minimum_type,
        "minimumValue": number(major(row.minimum_value)),
        "usageLimit": row.usage_limit,
        "usedCount": row.used_count,
        "oncePerCustomer": row.once_per_customer,
        "eligibility": row.eligibility,
        "customerFilter": row.customer_filter.as_ref().map(|j| j.0.clone()),
        "startsAt": iso(row.starts_at),
        "endsAt": row.ends_at.map(iso),
        "isActive": row.is_active,
        "createdAt": iso(row.created_at),
    })
}

/// `normalize`: the write values of an input, unused fields cleared.
struct Values {
    code: String,
    auto_apply: bool,
    discount_type: String,
    discount_value: f64,
    applies_to: String,
    category_id: Option<i64>,
    product_ids: Vec<i64>,
    minimum_type: String,
    minimum_value: f64,
    usage_limit: Option<i64>,
    once_per_customer: bool,
    eligibility: String,
    customer_filter: Option<Value>,
    starts_at: i64,
    ends_at: Option<i64>,
    is_active: bool,
}

fn normalize(input: &Map<String, Value>, now: i64) -> Values {
    let s = |k: &str, d: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or(d)
            .to_string()
    };
    let f = |k: &str| input.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let b = |k: &str, d: bool| input.get(k).and_then(Value::as_bool).unwrap_or(d);
    let discount_type = s("discountType", "percentage");
    let free_shipping = discount_type == "freeShipping";
    let applies_to = if free_shipping {
        "entireOrder".to_string()
    } else {
        s("appliesTo", "entireOrder")
    };
    let minimum_type = s("minimumType", "none");
    let eligibility = s("eligibility", "everyone");
    let mut ids: Vec<i64> = Vec::new();
    if applies_to == "products" {
        for v in input
            .get("productIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = v
                .as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    Values {
        code: s("code", "").trim().to_string(),
        auto_apply: b("autoApply", false),
        discount_value: if free_shipping {
            0.0
        } else {
            f("discountValue")
        },
        category_id: (applies_to == "category")
            .then(|| {
                input
                    .get("categoryId")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
            })
            .flatten(),
        product_ids: ids,
        minimum_value: if minimum_type == "none" {
            0.0
        } else {
            f("minimumValue")
        },
        usage_limit: input.get("usageLimit").and_then(Value::as_i64),
        once_per_customer: b("oncePerCustomer", false),
        customer_filter: (eligibility == "specificCustomers")
            .then(|| {
                input
                    .get("customerFilter")
                    .cloned()
                    .filter(|v| !v.is_null())
            })
            .flatten(),
        starts_at: input.get("startsAt").and_then(Value::as_i64).unwrap_or(now),
        ends_at: input.get("endsAt").and_then(Value::as_i64),
        is_active: b("isActive", true),
        discount_type,
        applies_to,
        minimum_type,
        eligibility,
    }
}

/// `assertValid`.
fn assert_valid(conn: &Connection, workspace_id: i64, v: &Values, used_count: i64) -> Result<()> {
    if !is_code(&v.code) {
        return Err(BusinessError::validation(
            "code",
            "A code is 3–64 letters, digits, - or _.",
        ));
    }
    if v.discount_type == "percentage" && !(v.discount_value > 0.0 && v.discount_value <= 100.0) {
        return Err(BusinessError::validation(
            "discountValue",
            "A percentage is more than 0 and at most 100.",
        ));
    }
    if v.discount_type == "fixedAmount" && v.discount_value <= 0.0 {
        return Err(BusinessError::validation(
            "discountValue",
            "An amount is more than 0.",
        ));
    }
    if v.applies_to == "category" {
        let found = match v.category_id {
            Some(id) => conn
                .query_row(
                    r#"SELECT 1 FROM "ProductCategory" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, workspace_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some(),
            None => false,
        };
        if !found {
            return Err(BusinessError::validation(
                "categoryId",
                "Choose a category of this workspace.",
            ));
        }
    }
    if v.applies_to == "products" {
        let found =
            chatbotx_db::repo::product::find_for_order(conn, workspace_id, &v.product_ids, None)?;
        if v.product_ids.is_empty() || found.len() != v.product_ids.len() {
            return Err(BusinessError::validation(
                "productIds",
                "Choose products of this workspace.",
            ));
        }
    }
    if v.minimum_type != "none" && v.minimum_value <= 0.0 {
        return Err(BusinessError::validation(
            "minimumValue",
            "A minimum is more than 0.",
        ));
    }
    if v.minimum_type == "quantity" && v.minimum_value.fract() != 0.0 {
        return Err(BusinessError::validation(
            "minimumValue",
            "A minimum number of items is a whole number.",
        ));
    }
    if let Some(limit) = v.usage_limit {
        if limit < 1 {
            return Err(BusinessError::validation(
                "usageLimit",
                "A usage limit is a whole number from 1.",
            ));
        }
        if limit < used_count {
            return Err(BusinessError::validation(
                "usageLimit",
                format!("This code was already used {used_count} times."),
            ));
        }
    }
    if v.eligibility == "specificCustomers"
        && v.customer_filter
            .as_ref()
            .and_then(|f| f["conditions"].as_array())
            .is_none_or(Vec::is_empty)
    {
        return Err(BusinessError::validation(
            "customerFilter",
            "Add at least one condition.",
        ));
    }
    if let Some(end) = v.ends_at
        && end <= v.starts_at
    {
        return Err(BusinessError::validation(
            "endsAt",
            "The end date is after the start date.",
        ));
    }
    Ok(())
}

fn find_row(conn: &Connection, workspace_id: i64, id: i64) -> Result<DiscountCode> {
    find_one::<DiscountCode>(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )?
    .ok_or_else(record_not_found)
}

fn code_taken(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    code: &str,
    except: Option<i64>,
) -> Result<bool> {
    Ok(find_by_code(conn, workspace_id, code, source)?.is_some_and(|row| Some(row.id) != except))
}

fn write_params(v: &Values) -> Vec<chatbotx_db::rusqlite::types::Value> {
    use chatbotx_db::rusqlite::types::Value as Sql;
    let opt = |v: Option<i64>| v.map_or(Sql::Null, Sql::Integer);
    vec![
        Sql::Text(v.code.clone()),
        Sql::Integer(v.auto_apply.into()),
        Sql::Text(v.discount_type.clone()),
        Sql::Integer(scaled(v.discount_value)),
        Sql::Text(v.applies_to.clone()),
        opt(v.category_id),
        Sql::Text(
            Value::from(
                v.product_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>(),
            )
            .to_string(),
        ),
        Sql::Text(v.minimum_type.clone()),
        Sql::Integer(scaled(v.minimum_value)),
        opt(v.usage_limit),
        Sql::Integer(v.once_per_customer.into()),
        Sql::Text(v.eligibility.clone()),
        v.customer_filter
            .as_ref()
            .map_or(Sql::Null, |f| Sql::Text(f.to_string())),
        Sql::Integer(v.starts_at),
        opt(v.ends_at),
        Sql::Integer(v.is_active.into()),
    ]
}

const WRITE_COLUMNS: &str = r#""code", "autoApply", "discountType", "discountValue", "appliesTo", "categoryId", "productIds",
  "minimumType", "minimumValue", "usageLimit", "oncePerCustomer", "eligibility", "customerFilter", "startsAt", "endsAt", "isActive""#;

/// `list`: the store's codes (or the mirrored store's), newest first.
pub async fn list(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    code: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<(Vec<DiscountCode>, i64)> {
    tx::read(db, move |conn| {
        let source = resolve_store_source(conn, workspace_id)?;
        let mut where_sql = Sql::raw(r#"WHERE "workspaceId" = "#);
        where_sql.push_bind(workspace_id);
        where_sql.push(r#" AND "source" = "#);
        where_sql.push_bind(source);
        if let Some(code) = code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            where_sql.push(r#" AND ilike("code", "#);
            where_sql.push_bind(like_contains(code));
            where_sql.push(")");
        }
        let total: i64 = {
            let mut count = Sql::raw(r#"SELECT count(*) FROM "DiscountCode" "#);
            count.append(where_sql.clone());
            count
                .query_map(conn, |r| r.get(0))?
                .first()
                .copied()
                .unwrap_or(0)
        };
        let pagination = parse_pagination(PaginationInput { page, per_page });
        let mut select = Sql::raw(format!("{} ", DiscountCode::select_sql()));
        select.append(where_sql);
        select.push(r#" ORDER BY "createdAt" DESC, "id" DESC"#);
        if let Some(p) = pagination {
            select.push(&format!(" LIMIT {} OFFSET {}", p.limit, p.offset));
        }
        let rows = select.models::<DiscountCode>(conn)?;
        let page_count = pagination.map_or(1, |p| {
            if p.limit > 0 {
                (total + p.limit - 1) / p.limit
            } else {
                1
            }
        });
        Ok((rows, page_count))
    })
    .await
}

/// `get`.
pub async fn get(db: &chatbotx_db::Db, workspace_id: i64, id: i64) -> Result<DiscountCode> {
    tx::read(db, move |conn| find_row(conn, workspace_id, id)).await
}

/// `generateCode`: a random code not taken in this workspace yet.
pub async fn generate_code(db: &chatbotx_db::Db, workspace_id: i64) -> Result<String> {
    tx::read(db, move |conn| {
        for _ in 0..10 {
            let mut bytes = [0u8; 8];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
                .map_err(|_| BusinessError::Repo(chatbotx_db::repo::Error::msg("system random")))?;
            // ponytail: modulo bias (256 % 57) is fine for a shop code.
            let code: String = bytes
                .iter()
                .map(|b| GENERATED_ALPHABET[usize::from(*b) % GENERATED_ALPHABET.len()] as char)
                .collect();
            let taken: bool = conn
                .query_row(
                    r#"SELECT 1 FROM "DiscountCode" WHERE "workspaceId" = ?1 AND lower("code") = lower(?2)"#,
                    params![workspace_id, code],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !taken {
                return Ok(code);
            }
        }
        Err(BusinessError::exception("Could not generate a free code.", "systemError", 500))
    })
    .await
}

/// `create`: refused while the store mirrors Shopify / WooCommerce.
pub async fn create(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    input: Map<String, Value>,
) -> Result<DiscountCode> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let source = resolve_store_source(conn, workspace_id)?;
        if is_external_store_source(&source) {
            return Err(external_store_managed(&source));
        }
        let now = now_ms();
        let values = normalize(&input, now);
        assert_valid(conn, workspace_id, &values, 0)?;
        if code_taken(conn, workspace_id, "chatbotx", &values.code, None)? {
            return Err(duplicated());
        }
        let mut sql = Sql::raw(format!(
            "INSERT INTO \"DiscountCode\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", {WRITE_COLUMNS}) VALUES ("
        ));
        let id = new_id();
        for (i, v) in [id, now, now, workspace_id].into_iter().enumerate() {
            if i > 0 {
                sql.push(", ");
            }
            sql.push_bind(v);
        }
        for v in write_params(&values) {
            sql.push(", ");
            sql.push_bind(v);
        }
        sql.push(")");
        sql.execute(conn)?;
        find_row(conn, workspace_id, id)
    })
    .await
}

/// `update`: a partial change, checked as the whole code it produces.
/// Products deleted since the last save are dropped when no list is sent.
pub async fn update(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    id: i64,
    data: Map<String, Value>,
) -> Result<DiscountCode> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let current = find_row(conn, workspace_id, id)?;
        if is_external_store_source(&current.source) {
            return Err(external_store_managed(&current.source));
        }
        let mut merged = to_input(&current);
        let sends_products = data.contains_key("productIds");
        merged.extend(data);
        if !sends_products && merged["appliesTo"] == "products" {
            let ids = product_ids(&current);
            let found = chatbotx_db::repo::product::find_for_order(conn, workspace_id, &ids, None)?;
            let kept: Vec<i64> = ids
                .into_iter()
                .filter(|id| found.iter().any(|p| p.product.id == *id))
                .collect();
            merged.insert("productIds".into(), json!(kept));
        }
        let values = normalize(&merged, current.starts_at);
        assert_valid(conn, workspace_id, &values, current.used_count)?;
        if code_taken(conn, workspace_id, &current.source, &values.code, Some(id))? {
            return Err(duplicated());
        }
        let mut sql = Sql::raw(r#"UPDATE "DiscountCode" SET "updatedAt" = "#);
        sql.push_bind(now_ms());
        let columns: Vec<&str> = WRITE_COLUMNS.split(',').map(|c| c.trim()).collect();
        for (column, value) in columns.iter().zip(write_params(&values)) {
            sql.push(&format!(", {column} = "));
            sql.push_bind(value);
        }
        sql.push(r#" WHERE "id" = "#);
        sql.push_bind(id);
        sql.push(r#" AND "workspaceId" = "#);
        sql.push_bind(workspace_id);
        sql.execute(conn)?;
        find_row(conn, workspace_id, id)
    })
    .await
}

/// `assertManagedHere`: a code mirrored from the connected store changes there.
fn assert_managed_here(conn: &Connection, workspace_id: i64, row: &DiscountCode) -> Result<()> {
    let source = resolve_store_source(conn, workspace_id)?;
    if is_external_store_source(&source) && row.source == source {
        return Err(external_store_managed(&source));
    }
    Ok(())
}

/// `setActive`: a code switched off by a deleted product or category is
/// fixed through `update` before it can be on.
pub async fn set_active(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    id: i64,
    is_active: bool,
) -> Result<DiscountCode> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let current = find_row(conn, workspace_id, id)?;
        assert_managed_here(conn, workspace_id, &current)?;
        if is_active {
            assert_valid(conn, workspace_id, &normalize(&to_input(&current), current.starts_at), current.used_count)?;
        }
        conn.execute(
            r#"UPDATE "DiscountCode" SET "isActive" = ?3, "updatedAt" = ?4 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, is_active, now_ms()],
        )?;
        find_row(conn, workspace_id, id)
    })
    .await
}

/// `delete`: placed orders keep the code they used, as text.
pub async fn delete(db: &chatbotx_db::Db, workspace_id: i64, id: i64) -> Result<DiscountCode> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let current = find_row(conn, workspace_id, id)?;
        assert_managed_here(conn, workspace_id, &current)?;
        conn.execute(
            r#"DELETE FROM "DiscountCode" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )?;
        Ok(current)
    })
    .await
}
