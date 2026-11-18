//! `packages/business/src/woocommerce/mappers.ts`: WooCommerce's products,
//! coupons and orders in our terms. Values follow the zod schemas' defaults
//! (`numeric` = 0, `text` = "").

use chatbotx_db::repo::external_store::{ExternalProductOption, ExternalVariantValues};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::{Map, Value, json};

use crate::contact::phone::normalize_phone_to_e164;
use crate::shopify::mappers::short_description;

const MONEY_SCALE: f64 = 10_000.0;
/// The separator between a variation id and its "Any …" expansion.
pub const ANY_VARIANT_SEPARATOR: char = '~';

/// `text`: a string, `""` otherwise.
pub(crate) fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// `numeric`: a number or numeric string, 0 otherwise.
pub(crate) fn numeric(value: &Value) -> f64 {
    optional_number(value).unwrap_or(0.0)
}

/// `optionalNumber`.
pub(crate) fn optional_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            // `Number.parseFloat`: the leading number.
            let t = s.trim_start();
            let end = t
                .char_indices()
                .take_while(|(i, c)| {
                    c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))
                })
                .count();
            t[..end].parse().ok()
        }
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

fn arr(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn opt(value: &str) -> Sql {
    if value.is_empty() {
        Sql::Null
    } else {
        Sql::Text(value.to_string())
    }
}

fn scaled(major: f64) -> i64 {
    (major * MONEY_SCALE).round() as i64
}

/// `htmlToPlainText`: WooCommerce descriptions are HTML; ours are plain text.
pub fn html_to_plain_text(html: &str) -> String {
    static BREAKS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)</(p|div|li|h[1-6])>|<br\s*/?>").expect("regex")
    });
    static TAGS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]*>").expect("regex"));
    static ENTITY: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)&(?:[a-z]+|#\d+);").expect("regex"));
    static SPACES: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[ \t]+").expect("regex"));
    static BLANK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\n{3,}").expect("regex"));
    let text = BREAKS.replace_all(html, "\n");
    let text = TAGS.replace_all(&text, "");
    let text = ENTITY.replace_all(&text, |c: &regex::Captures<'_>| {
        let entity = c[0].to_lowercase();
        match entity.as_str() {
            "&amp;" => "&",
            "&lt;" => "<",
            "&gt;" => ">",
            "&quot;" => "\"",
            "&#039;" | "&#39;" => "'",
            "&nbsp;" => " ",
            "&#8211;" => "–",
            "&#8212;" => "—",
            "&#8217;" => "’",
            "&#8220;" => "“",
            "&#8221;" => "”",
            _ => return c[0].to_string(),
        }
        .to_string()
    });
    let text = SPACES.replace_all(&text, " ");
    let lines: Vec<&str> = text.split('\n').map(str::trim).collect();
    BLANK
        .replace_all(&lines.join("\n"), "\n\n")
        .trim()
        .to_string()
}

/// `mapWooCommerceProductStatus`.
pub fn product_status(product: &Value) -> &'static str {
    match text(&product["status"]) {
        "publish" if product["catalog_visibility"] == "hidden" => "unlisted",
        "publish" => "active",
        "private" => "unlisted",
        _ => "draft",
    }
}

/// `isOrderableWooCommerceProduct`.
pub fn is_orderable(product: &Value) -> bool {
    matches!(text(&product["type"]), "simple" | "variable")
}

/// `stockOf`: `None` = not counted; out of stock without counting = 0.
fn stock_of(item: &Value) -> Option<i64> {
    if item["manage_stock"] == true {
        return Some(
            optional_number(&item["stock_quantity"])
                .unwrap_or(0.0)
                .floor()
                .max(0.0) as i64,
        );
    }
    (item["stock_status"] == "outofstock").then_some(0)
}

/// `wooCommerceVariationIdOf`.
pub fn variation_id_of(external_id: &str) -> &str {
    external_id
        .split(ANY_VARIANT_SEPARATOR)
        .next()
        .unwrap_or(external_id)
}

/// `wooCommerceAnyAttributesOf`: `55~Size=M` -> `[("Size", "M")]`.
pub fn any_attributes_of(external_id: &str) -> Vec<(String, String)> {
    match external_id.split_once(ANY_VARIANT_SEPARATOR) {
        Some((_, query)) => url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
        None => Vec::new(),
    }
}

/// `anyVariantExternalId`: keyed by the sorted choices.
fn any_variant_external_id(variation_id: i64, choices: &[(String, String)]) -> String {
    let mut sorted = choices.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in &sorted {
        serializer.append_pair(k, v);
    }
    format!(
        "{variation_id}{ANY_VARIANT_SEPARATOR}{}",
        serializer.finish()
    )
}

/// `expandVariation`: one variant per combination a variation sells.
fn expand_variation(
    variation: &Value,
    options: &[ExternalProductOption],
) -> Vec<(String, Map<String, Value>)> {
    let set: Vec<(&str, &str)> = arr(&variation["attributes"])
        .iter()
        .map(|a| (text(&a["name"]), text(&a["option"])))
        .collect();
    let chosen = |name: &str| {
        set.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, o)| *o)
            .filter(|o| !o.is_empty())
    };
    let any: Vec<&ExternalProductOption> = options
        .iter()
        .filter(|o| chosen(&o.name).is_none())
        .collect();
    let mut combinations: Vec<Map<String, Value>> = vec![Map::new()];
    for option in options {
        let values: Vec<String> = match chosen(&option.name) {
            Some(v) => vec![v.to_string()],
            None => option.values.clone(),
        };
        combinations = combinations
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut next = c.clone();
                    next.insert(option.name.clone(), Value::String(v.clone()));
                    next
                })
            })
            .collect();
    }
    let id = variation["id"].as_i64().unwrap_or_default();
    combinations
        .into_iter()
        .map(|c| {
            let external_id = if any.is_empty() {
                id.to_string()
            } else {
                let choices: Vec<(String, String)> = any
                    .iter()
                    .map(|o| (o.name.clone(), text(&c[&o.name]).to_string()))
                    .collect();
                any_variant_external_id(id, &choices)
            };
            (external_id, c)
        })
        .collect()
}

pub use crate::shopify::mappers::MappedProduct;

/// `mapWooCommerceProduct`.
pub fn map_product(product: &Value, variations: &[Value], currency: &str) -> MappedProduct {
    let is_variable = product["type"] == "variable";
    let mut attributes: Vec<&Value> = arr(&product["attributes"])
        .iter()
        .filter(|a| a["variation"] == true)
        .collect();
    attributes.sort_by_key(|a| a["position"].as_i64().unwrap_or(0));
    let options: Vec<ExternalProductOption> = if is_variable {
        attributes
            .iter()
            .enumerate()
            .map(|(i, a)| ExternalProductOption {
                name: text(&a["name"]).to_string(),
                values: arr(&a["options"])
                    .iter()
                    .filter_map(|o| o.as_str().map(str::to_string))
                    .collect(),
                position: i as i64,
            })
            .collect()
    } else {
        Vec::new()
    };
    let variants: Vec<ExternalVariantValues> = if is_variable {
        variations
            .iter()
            .flat_map(|variation| {
                expand_variation(variation, &options).into_iter().map(
                    |(external_id, combination)| ExternalVariantValues {
                        external_id,
                        external_inventory_item_id: None,
                        combination: Value::Object(combination),
                        price: numeric(&variation["price"]),
                        sku: Some(text(&variation["sku"]))
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                        inventory_quantity: stock_of(variation),
                        is_enabled: variation["status"] == "publish",
                    },
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let enabled_prices: Vec<f64> = variants
        .iter()
        .filter(|v| v.is_enabled)
        .map(|v| v.price)
        .collect();
    let description = html_to_plain_text(text(&product["description"]));
    let short = html_to_plain_text(text(&product["short_description"]));
    let short = if short.is_empty() {
        description.clone()
    } else {
        short
    };
    let stock = stock_of(product);
    let category = arr(&product["categories"])
        .iter()
        .find(|t| t["slug"] != "uncategorized");
    let trimmed = |v: Option<&Value>| {
        v.map(|t| text(&t["name"]).trim().to_string())
            .filter(|n| !n.is_empty())
    };
    let id = product["id"].as_i64().unwrap_or_default().to_string();
    let images: Vec<Value> = arr(&product["images"])
        .iter()
        .filter_map(|i| i["src"].as_str())
        .map(|src| json!({ "url": src, "type": "link" }))
        .collect();
    let tags: Vec<Value> = arr(&product["tags"])
        .iter()
        .map(|t| text(&t["name"]))
        .filter(|n| !n.is_empty())
        .map(|n| Value::String(n.to_string()))
        .collect();
    let price = if enabled_prices.is_empty() {
        numeric(&product["price"])
    } else {
        enabled_prices.iter().copied().fold(f64::INFINITY, f64::min)
    };
    let values: Patch = vec![
        ("name", Sql::Text(text(&product["name"]).to_string())),
        (
            "shortDescription",
            short_description(&short).map_or(Sql::Null, Sql::Text),
        ),
        ("longDescription", opt(&description)),
        ("price", Sql::Real(price)),
        ("currency", Sql::Text(currency.to_string())),
        ("productUrl", opt(text(&product["permalink"]))),
        ("sku", opt(text(&product["sku"]))),
        (
            "inventoryPolicy",
            Sql::Text(
                if stock.is_none() {
                    "dont_track"
                } else {
                    "track"
                }
                .into(),
            ),
        ),
        ("inventoryQuantity", Sql::Integer(stock.unwrap_or(0))),
        (
            "allowOutOfStockPurchase",
            Sql::Integer(i64::from(
                product["manage_stock"] == true && text(&product["backorders"]) != "no",
            )),
        ),
        ("images", Sql::Text(Value::Array(images).to_string())),
        ("tags", Sql::Text(Value::Array(tags).to_string())),
        ("status", Sql::Text(product_status(product).into())),
        ("externalHandle", opt(text(&product["slug"]))),
        (
            "externalVariantId",
            if is_variable {
                Sql::Null
            } else {
                Sql::Text(id.clone())
            },
        ),
    ];
    MappedProduct {
        external_id: id,
        values,
        options,
        variants,
        category_name: trimmed(category),
        vendor_name: trimmed(arr(&product["brands"]).first()),
    }
}

/// `gmtDate`: a GMT date sent without a zone.
pub fn gmt_date(value: &Value) -> Option<i64> {
    let text = value.as_str().filter(|t| !t.is_empty())?;
    let zoned = if text.ends_with('Z') {
        text.to_string()
    } else {
        format!("{text}Z")
    };
    chrono::DateTime::parse_from_rfc3339(&zoned)
        .ok()
        .map(|d| d.timestamp_millis())
}

pub use crate::shopify::mappers::MappedDiscount;

/// `mapWooCommerceCoupon`: `None` when our cart could not price it.
pub fn map_coupon(coupon: &Value) -> Option<MappedDiscount> {
    let code = text(&coupon["code"]).trim();
    let status = coupon["status"].as_str().unwrap_or("publish");
    if code.is_empty() || status != "publish" {
        return None;
    }
    let kind = text(&coupon["discount_type"]);
    if kind == "fixed_product"
        || !arr(&coupon["product_categories"]).is_empty()
        || !arr(&coupon["excluded_product_categories"]).is_empty()
        || !arr(&coupon["excluded_product_ids"]).is_empty()
        || !arr(&coupon["email_restrictions"]).is_empty()
        || coupon["exclude_sale_items"] == true
        || numeric(&coupon["maximum_amount"]) > 0.0
    {
        return None;
    }
    let amount = numeric(&coupon["amount"]);
    let (discount_type, value) = if amount <= 0.0 && coupon["free_shipping"] == true {
        ("freeShipping", 0.0)
    } else if kind == "percent" {
        ("percentage", amount)
    } else if kind == "fixed_cart" {
        ("fixedAmount", amount)
    } else {
        return None;
    };
    if discount_type != "freeShipping" && value <= 0.0 {
        return None;
    }
    let usage_limit = optional_number(&coupon["usage_limit"])
        .filter(|l| *l > 0.0)
        .map(|l| l.floor() as i64);
    let used = optional_number(&coupon["usage_count"])
        .unwrap_or(0.0)
        .floor()
        .max(0.0) as i64;
    let product_external_ids: Vec<String> = arr(&coupon["product_ids"])
        .iter()
        .filter_map(Value::as_i64)
        .map(|id| id.to_string())
        .collect();
    let minimum = numeric(&coupon["minimum_amount"]);
    let products = !product_external_ids.is_empty();
    let values: Patch = vec![
        ("code", Sql::Text(code.to_string())),
        ("discountType", Sql::Text(discount_type.into())),
        ("discountValue", Sql::Integer(scaled(value))),
        (
            "appliesTo",
            Sql::Text(if products { "products" } else { "entireOrder" }.into()),
        ),
        (
            "minimumType",
            Sql::Text(if minimum > 0.0 { "amount" } else { "none" }.into()),
        ),
        ("minimumValue", Sql::Integer(scaled(minimum))),
        ("usageLimit", usage_limit.map_or(Sql::Null, Sql::Integer)),
        (
            "usedCount",
            Sql::Integer(usage_limit.map_or(used, |l| used.min(l))),
        ),
        (
            "oncePerCustomer",
            Sql::Integer(i64::from(
                optional_number(&coupon["usage_limit_per_user"]) == Some(1.0),
            )),
        ),
        (
            "startsAt",
            Sql::Integer(gmt_date(&coupon["date_created_gmt"]).unwrap_or(0)),
        ),
        (
            "endsAt",
            gmt_date(&coupon["date_expires_gmt"]).map_or(Sql::Null, Sql::Integer),
        ),
        ("isActive", Sql::Integer(1)),
        ("autoApply", Sql::Integer(0)),
        ("eligibility", Sql::Text("everyone".into())),
        ("customerFilter", Sql::Null),
        ("categoryId", Sql::Null),
    ];
    Some(MappedDiscount {
        external_id: coupon["id"].as_i64().unwrap_or_default().to_string(),
        applies_to_products: products,
        product_external_ids,
        values,
    })
}

pub use crate::shopify::mappers::OrderState;

/// `mapWooCommerceOrderState`.
pub fn order_state(order: &Value) -> OrderState {
    let status_text = text(&order["status"]);
    let status = match status_text {
        "processing" | "refunded" => "accepted",
        "completed" => "concluded",
        "cancelled" | "failed" => "cancelled",
        _ => "pending",
    };
    let payment_status = if status_text == "refunded" {
        "refunded"
    } else if order["date_paid_gmt"]
        .as_str()
        .is_some_and(|d| !d.is_empty())
        || status_text == "completed"
    {
        "paid"
    } else {
        "unpaid"
    };
    let tracking = arr(&order["meta_data"])
        .iter()
        .find(|m| m["key"] == "_wc_shipment_tracking_items")
        .and_then(|m| m["value"].as_array())
        .and_then(|items| items.first());
    let read = |key: &str| {
        tracking
            .and_then(|t| t[key].as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    OrderState {
        status,
        payment_status,
        fulfillment_status: if status_text == "completed" {
            "fulfilled"
        } else {
            "unfulfilled"
        },
        tracking_company: read("tracking_provider").or_else(|| read("custom_tracking_provider")),
        tracking_number: read("tracking_number"),
        tracking_url: read("custom_tracking_link"),
    }
}

/// `wooCommerceOrderExternalId`: `example.com#1234`.
pub fn order_external_id(site_url: &str, order_id: impl std::fmt::Display) -> String {
    format!(
        "{}#{order_id}",
        site_url.strip_prefix("https://").unwrap_or(site_url)
    )
}

/// `isMirrorableWooCommerceOrder`.
pub fn is_mirrorable(order: &Value) -> bool {
    !matches!(text(&order["status"]), "checkout-draft" | "trash")
}

/// `mapWooCommerceAddress`.
pub fn address(address: &Value) -> Option<Value> {
    let field = |key: &str| {
        Some(text(&address[key]))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    (field("address_1").is_some() || field("city").is_some()).then(|| {
        json!({
            "line1": field("address_1"),
            "line2": field("address_2"),
            "city": field("city"),
            "state": field("state"),
            "zip": field("postcode"),
            "country": field("country"),
        })
    })
}

/// `wooCommerceCustomerName`.
pub fn customer_name(order: &Value) -> Option<String> {
    [&order["billing"], &order["shipping"]]
        .iter()
        .find_map(|a| {
            let name = [text(&a["first_name"]).trim(), text(&a["last_name"]).trim()]
                .iter()
                .filter(|p| !p.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            (!name.is_empty()).then_some(name)
        })
}

/// `wooCommerceOrderMeta`: a text meta value.
pub fn order_meta<'a>(order: &'a Value, key: &str) -> Option<&'a str> {
    arr(&order["meta_data"])
        .iter()
        .find(|m| m["key"] == key)
        .and_then(|m| m["value"].as_str())
        .filter(|v| !v.is_empty())
}

/// `wooCommerceOrderContactIdentifiers`.
pub fn order_contact_identifiers(order: &Value) -> Vec<String> {
    let billing = &order["billing"];
    let shipping = &order["shipping"];
    let email = text(&billing["email"]).trim();
    let raw_phone = Some(text(&billing["phone"]).trim())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| text(&shipping["phone"]).trim());
    let country = Some(text(&billing["country"]))
        .filter(|c| !c.is_empty())
        .or_else(|| Some(text(&shipping["country"])).filter(|c| !c.is_empty()));
    let phone = (!raw_phone.is_empty())
        .then(|| normalize_phone_to_e164(raw_phone, country))
        .flatten();
    let mut out = Vec::new();
    if !email.is_empty() {
        out.push(format!("email:{}", email.to_lowercase()));
    }
    if let Some(phone) = phone {
        out.push(format!("phone:{phone}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_from_html() {
        assert_eq!(
            html_to_plain_text("<p>Hello &amp; <b>welcome</b></p><p>Line&nbsp;2</p>\n\n\n\n<br/>x"),
            "Hello & welcome\nLine 2\n\nx"
        );
    }

    #[test]
    fn any_variants_expand_per_value() {
        let options = vec![
            ExternalProductOption {
                name: "Size".into(),
                values: vec!["S".into(), "M".into()],
                position: 0,
            },
            ExternalProductOption {
                name: "Color".into(),
                values: vec!["Red".into()],
                position: 1,
            },
        ];
        let variation =
            json!({ "id": 55, "attributes": [{ "id": 1, "name": "Color", "option": "Red" }] });
        let expanded = expand_variation(&variation, &options);
        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[1].0, "55~Size=M");
        assert_eq!(variation_id_of(&expanded[1].0), "55");
        assert_eq!(
            any_attributes_of("55~Size=M"),
            vec![("Size".to_string(), "M".to_string())]
        );
    }

    #[test]
    fn coupons_and_orders() {
        let coupon = json!({ "id": 3, "code": " SAVE ", "amount": "10.00", "discount_type": "percent",
            "usage_limit": 5, "usage_count": 9, "usage_limit_per_user": 1, "product_ids": [],
            "minimum_amount": "0", "maximum_amount": "", "date_created_gmt": "2026-01-02T03:04:05" });
        let mapped = map_coupon(&coupon).unwrap();
        assert!(!mapped.applies_to_products);
        let value = |k: &str| {
            mapped
                .values
                .iter()
                .find(|(n, _)| *n == k)
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(value("usedCount"), Sql::Integer(5));
        assert_eq!(value("discountValue"), Sql::Integer(100_000));
        assert_eq!(value("oncePerCustomer"), Sql::Integer(1));
        let mut fixed = coupon.clone();
        fixed["discount_type"] = json!("fixed_product");
        assert!(map_coupon(&fixed).is_none());

        let order = json!({ "status": "completed", "meta_data": [{ "key": "_wc_shipment_tracking_items",
            "value": [{ "custom_tracking_provider": "DHL", "tracking_number": " 7 " }] }] });
        let state = order_state(&order);
        assert_eq!((state.status, state.payment_status), ("concluded", "paid"));
        assert_eq!(state.tracking_company.as_deref(), Some("DHL"));
        assert_eq!(state.tracking_number.as_deref(), Some("7"));
        assert_eq!(
            order_external_id("https://shop.example", 12),
            "shop.example#12"
        );
        assert_eq!(numeric(&json!("12.5abc")), 12.5);
    }
}
