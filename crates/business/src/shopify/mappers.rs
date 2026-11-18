//! `packages/business/src/shopify/mappers.ts`: Shopify's products, code
//! discounts and orders in our terms.

use chatbotx_db::repo::external_store::{ExternalProductOption, ExternalVariantValues};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::{Value, json};

use crate::contact::phone::normalize_phone_to_e164;

const SHORT_DESCRIPTION_LENGTH: usize = 300;
const MONEY_SCALE: f64 = 10_000.0;

pub(crate) fn s(value: &Value) -> Option<&str> {
    value.as_str()
}

/// Shopify sends money as a decimal string.
pub(crate) fn money(value: &Value) -> f64 {
    match value {
        Value::String(text) => text.trim().parse().unwrap_or(0.0),
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn arr(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn opt_text(value: Option<&str>) -> Sql {
    value.map_or(Sql::Null, |v| Sql::Text(v.to_string()))
}

/// Shopify's placeholder option for a product sold without options.
fn is_default_only_options(product: &Value) -> bool {
    let options = arr(&product["options"]);
    options.len() == 1
        && options[0]["name"] == "Title"
        && arr(&options[0]["optionValues"]).len() == 1
        && options[0]["optionValues"][0]["name"] == "Default Title"
}

/// `mapShopifyProductStatus`.
pub fn product_status(status: &str) -> &'static str {
    match status {
        "ACTIVE" => "active",
        "UNLISTED" => "unlisted",
        _ => "draft",
    }
}

/// `MappedShopifyProduct`.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedProduct {
    pub external_id: String,
    /// Product columns besides `categoryId` / `vendorId`.
    pub values: Patch,
    pub options: Vec<ExternalProductOption>,
    pub variants: Vec<ExternalVariantValues>,
    pub category_name: Option<String>,
    pub vendor_name: Option<String>,
}

fn trimmed(value: &Value) -> Option<String> {
    s(value)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// `"…"` after the first 299 characters, as the original.
pub(crate) fn short_description(description: &str) -> Option<String> {
    if description.is_empty() {
        return None;
    }
    if description.chars().count() > SHORT_DESCRIPTION_LENGTH {
        let head: String = description
            .chars()
            .take(SHORT_DESCRIPTION_LENGTH - 1)
            .collect();
        return Some(format!("{}…", head.trim_end()));
    }
    Some(description.to_string())
}

/// `mapShopifyProduct`.
pub fn map_product(product: &Value, shop_domain: &str, currency: &str) -> MappedProduct {
    let variants = arr(&product["variants"]["nodes"]);
    let first = variants.first();
    let without_options = is_default_only_options(product);
    let description = s(&product["description"]).unwrap_or_default().trim();
    let handle = s(&product["handle"]).unwrap_or_default();
    let product_url = s(&product["onlineStoreUrl"]).map_or_else(
        || format!("https://{shop_domain}/products/{handle}"),
        str::to_string,
    );
    let images: Vec<Value> = arr(&product["media"]["nodes"])
        .iter()
        .filter_map(|m| s(&m["image"]["url"]).filter(|u| !u.is_empty()))
        .map(|url| json!({ "url": url, "type": "link" }))
        .collect();
    let sku = first.and_then(|v| s(&v["sku"])).filter(|x| !x.is_empty());
    let values: Patch = vec![
        (
            "name",
            Sql::Text(s(&product["title"]).unwrap_or_default().to_string()),
        ),
        (
            "shortDescription",
            opt_text(short_description(description).as_deref()),
        ),
        (
            "longDescription",
            opt_text(Some(description).filter(|d| !d.is_empty())),
        ),
        (
            "price",
            Sql::Real(first.map_or(0.0, |v| money(&v["price"]))),
        ),
        ("currency", Sql::Text(currency.to_string())),
        ("productUrl", Sql::Text(product_url)),
        ("sku", opt_text(sku)),
        (
            "inventoryPolicy",
            Sql::Text(
                if product["tracksInventory"] == true {
                    "track"
                } else {
                    "dont_track"
                }
                .into(),
            ),
        ),
        (
            "inventoryQuantity",
            Sql::Integer(product["totalInventory"].as_i64().unwrap_or(0).max(0)),
        ),
        ("allowOutOfStockPurchase", Sql::Integer(0)),
        ("images", Sql::Text(Value::Array(images).to_string())),
        (
            "tags",
            Sql::Text(Value::Array(arr(&product["tags"]).to_vec()).to_string()),
        ),
        (
            "status",
            Sql::Text(product_status(s(&product["status"]).unwrap_or_default()).into()),
        ),
        ("externalHandle", Sql::Text(handle.to_string())),
        (
            "externalVariantId",
            opt_text(
                without_options
                    .then(|| first.and_then(|v| s(&v["id"])))
                    .flatten(),
            ),
        ),
    ];
    let options = if without_options {
        Vec::new()
    } else {
        arr(&product["options"])
            .iter()
            .map(|o| ExternalProductOption {
                name: s(&o["name"]).unwrap_or_default().to_string(),
                values: arr(&o["optionValues"])
                    .iter()
                    .filter_map(|v| s(&v["name"]).map(str::to_string))
                    .collect(),
                position: o["position"].as_i64().unwrap_or(0),
            })
            .collect()
    };
    let variants = if without_options {
        Vec::new()
    } else {
        variants
            .iter()
            .map(|v| {
                let combination: serde_json::Map<String, Value> = arr(&v["selectedOptions"])
                    .iter()
                    .map(|o| {
                        (
                            s(&o["name"]).unwrap_or_default().to_string(),
                            o["value"].clone(),
                        )
                    })
                    .collect();
                ExternalVariantValues {
                    external_id: s(&v["id"]).unwrap_or_default().to_string(),
                    external_inventory_item_id: s(&v["inventoryItem"]["id"]).map(str::to_string),
                    combination: Value::Object(combination),
                    price: money(&v["price"]),
                    sku: s(&v["sku"]).filter(|x| !x.is_empty()).map(str::to_string),
                    inventory_quantity: (v["inventoryItem"]["tracked"] != false)
                        .then(|| v["inventoryQuantity"].as_i64().unwrap_or(0).max(0)),
                    is_enabled: true,
                }
            })
            .collect()
    };
    MappedProduct {
        external_id: s(&product["id"]).unwrap_or_default().to_string(),
        values,
        options,
        variants,
        category_name: trimmed(&product["productType"]),
        vendor_name: trimmed(&product["vendor"]),
    }
}

/// `MappedShopifyDiscount`.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedDiscount {
    pub external_id: String,
    pub applies_to_products: bool,
    /// Shopify product GIDs, with `appliesTo: products`.
    pub product_external_ids: Vec<String>,
    /// The discount columns besides `productIds`.
    pub values: Patch,
}

fn iso_ms(value: &Value) -> Option<i64> {
    s(value)
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp_millis())
}

fn scaled(major: f64) -> i64 {
    (major * MONEY_SCALE).round() as i64
}

/// `mapShopifyDiscount`: `None` when our cart could not price it the way
/// Shopify will (collections, buy X get Y, apps).
pub fn map_discount(node: &Value) -> Option<MappedDiscount> {
    let discount = &node["codeDiscount"];
    let code = s(&discount["codes"]["nodes"][0]["code"])?;
    let mut discount_value = 0.0;
    let mut applies_to = "entireOrder";
    let mut product_external_ids = Vec::new();
    let discount_type = match s(&discount["__typename"]) {
        Some("DiscountCodeFreeShipping") => "freeShipping",
        Some("DiscountCodeBasic") => {
            let value = &discount["customerGets"]["value"];
            let kind = match s(&value["__typename"]) {
                Some("DiscountPercentage") => {
                    // Shopify keeps a fraction, 0.1 for 10%.
                    discount_value =
                        (value["percentage"].as_f64().unwrap_or(0.0) * 10_000.0).round() / 100.0;
                    "percentage"
                }
                Some("DiscountAmount") => {
                    discount_value = money(&value["amount"]["amount"]);
                    "fixedAmount"
                }
                _ => return None,
            };
            let items = &discount["customerGets"]["items"];
            match s(&items["__typename"]) {
                Some("DiscountProducts") => {
                    applies_to = "products";
                    product_external_ids = arr(&items["products"]["nodes"])
                        .iter()
                        .filter_map(|p| s(&p["id"]).map(str::to_string))
                        .collect();
                    if product_external_ids.is_empty() {
                        return None;
                    }
                }
                Some("AllDiscountItems") if items["allItems"] == true => {}
                _ => return None,
            }
            kind
        }
        _ => return None,
    };
    let minimum = &discount["minimumRequirement"];
    let (minimum_type, minimum_value) = match s(&minimum["__typename"]) {
        Some("DiscountMinimumSubtotal") => (
            "amount",
            money(&minimum["greaterThanOrEqualToSubtotal"]["amount"]),
        ),
        Some("DiscountMinimumQuantity") => (
            "quantity",
            s(&minimum["greaterThanOrEqualToQuantity"])
                .and_then(|q| q.trim().parse::<i64>().ok())
                .unwrap_or(0) as f64,
        ),
        _ => ("none", 0.0),
    };
    let usage_limit = discount["usageLimit"].as_i64();
    let used = discount["asyncUsageCount"].as_i64().unwrap_or(0);
    let values: Patch = vec![
        ("code", Sql::Text(code.to_string())),
        ("discountType", Sql::Text(discount_type.into())),
        ("discountValue", Sql::Integer(scaled(discount_value))),
        ("appliesTo", Sql::Text(applies_to.into())),
        ("minimumType", Sql::Text(minimum_type.into())),
        ("minimumValue", Sql::Integer(scaled(minimum_value))),
        ("usageLimit", usage_limit.map_or(Sql::Null, Sql::Integer)),
        // Our check constraint keeps the count within the limit.
        (
            "usedCount",
            Sql::Integer(usage_limit.map_or(used, |limit| used.min(limit))),
        ),
        (
            "oncePerCustomer",
            Sql::Integer(i64::from(discount["appliesOncePerCustomer"] == true)),
        ),
        (
            "startsAt",
            Sql::Integer(iso_ms(&discount["startsAt"]).unwrap_or(0)),
        ),
        (
            "endsAt",
            iso_ms(&discount["endsAt"]).map_or(Sql::Null, Sql::Integer),
        ),
        (
            "isActive",
            Sql::Integer(i64::from(discount["status"] != "EXPIRED")),
        ),
        ("autoApply", Sql::Integer(0)),
        ("eligibility", Sql::Text("everyone".into())),
        ("customerFilter", Sql::Null),
        ("categoryId", Sql::Null),
    ];
    Some(MappedDiscount {
        external_id: s(&node["id"]).unwrap_or_default().to_string(),
        applies_to_products: applies_to == "products",
        product_external_ids,
        values,
    })
}

/// `mapShopifyOrderState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderState {
    pub status: &'static str,
    pub payment_status: &'static str,
    pub fulfillment_status: &'static str,
    pub tracking_company: Option<String>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
}

pub fn order_state(order: &Value) -> OrderState {
    let financial = s(&order["displayFinancialStatus"]).unwrap_or("PENDING");
    let payment_status = match financial {
        "REFUNDED" => "refunded",
        "PAID" | "PARTIALLY_REFUNDED" => "paid",
        _ => "unpaid",
    };
    let fulfillment_status = match s(&order["displayFulfillmentStatus"]) {
        Some("FULFILLED") => "fulfilled",
        Some("PARTIALLY_FULFILLED") => "partial",
        _ => "unfulfilled",
    };
    let fulfillments = arr(&order["fulfillments"]);
    let delivered = fulfillments
        .iter()
        .any(|f| f["displayStatus"] == "DELIVERED");
    let status = if !order["cancelledAt"].is_null() {
        "cancelled"
    } else if delivered && fulfillment_status == "fulfilled" {
        "concluded"
    } else if fulfillment_status == "fulfilled" {
        "shipped"
    } else if payment_status != "unpaid" {
        "accepted"
    } else {
        "pending"
    };
    let tracking = fulfillments
        .iter()
        .flat_map(|f| arr(&f["trackingInfo"]))
        .next();
    let field = |key: &str| tracking.and_then(|t| s(&t[key])).map(str::to_string);
    OrderState {
        status,
        payment_status,
        fulfillment_status,
        tracking_company: field("company"),
        tracking_number: field("number"),
        tracking_url: field("url"),
    }
}

/// `mapShopifyAddress`.
pub fn address(address: &Value) -> Option<Value> {
    address.is_object().then(|| {
        json!({
            "line1": address["address1"],
            "line2": address["address2"],
            "city": address["city"],
            "state": address["province"],
            "zip": address["zip"],
            "country": address["countryCodeV2"],
        })
    })
}

/// `shopifyOrderContactIdentifiers`: email first, the phone in E.164.
pub fn order_contact_identifiers(order: &Value) -> Vec<String> {
    let email = s(&order["email"]).or_else(|| s(&order["customer"]["email"]));
    let phone = s(&order["phone"])
        .or_else(|| s(&order["customer"]["phone"]))
        .and_then(|raw| {
            normalize_phone_to_e164(raw, s(&order["shippingAddress"]["countryCodeV2"]))
        });
    let mut out = Vec::new();
    if let Some(email) = email.filter(|e| !e.is_empty()) {
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

    fn product() -> Value {
        json!({
            "id": "gid://shopify/Product/1", "title": "Tote", "handle": "tote",
            "description": "  Nice  ", "status": "ACTIVE", "productType": " Bags ", "vendor": "",
            "tags": ["a"], "onlineStoreUrl": null, "tracksInventory": true, "totalInventory": -2,
            "options": [{ "name": "Title", "position": 1, "optionValues": [{ "name": "Default Title" }] }],
            "media": { "nodes": [{ "image": { "url": "https://cdn/x.png" } }, {}] },
            "variants": { "nodes": [{ "id": "gid://shopify/ProductVariant/9", "title": "Default Title",
                "sku": "", "price": "12.50", "inventoryQuantity": 3, "availableForSale": true,
                "selectedOptions": [{ "name": "Title", "value": "Default Title" }],
                "inventoryItem": { "id": "gid://shopify/InventoryItem/5", "tracked": true } }] }
        })
    }

    fn value<'a>(patch: &'a Patch, key: &str) -> &'a Sql {
        &patch.iter().find(|(k, _)| *k == key).unwrap().1
    }

    #[test]
    fn a_product_without_options_keeps_its_one_variant() {
        let mapped = map_product(&product(), "shop.myshopify.com", "USD");
        assert!(mapped.variants.is_empty() && mapped.options.is_empty());
        assert_eq!(mapped.category_name.as_deref(), Some("Bags"));
        assert_eq!(mapped.vendor_name, None);
        assert_eq!(value(&mapped.values, "price"), &Sql::Real(12.5));
        assert_eq!(value(&mapped.values, "inventoryQuantity"), &Sql::Integer(0));
        assert_eq!(
            value(&mapped.values, "productUrl"),
            &Sql::Text("https://shop.myshopify.com/products/tote".into())
        );
        assert_eq!(
            value(&mapped.values, "externalVariantId"),
            &Sql::Text("gid://shopify/ProductVariant/9".into())
        );
        assert_eq!(value(&mapped.values, "sku"), &Sql::Null);
    }

    #[test]
    fn discounts() {
        let node = json!({ "id": "gid://shopify/DiscountCodeNode/1", "codeDiscount": {
            "__typename": "DiscountCodeBasic", "status": "ACTIVE", "usageLimit": 2, "asyncUsageCount": 5,
            "codes": { "nodes": [{ "code": "TEN" }] },
            "minimumRequirement": { "__typename": "DiscountMinimumQuantity", "greaterThanOrEqualToQuantity": "3" },
            "customerGets": { "value": { "__typename": "DiscountPercentage", "percentage": 0.1 },
                "items": { "__typename": "AllDiscountItems", "allItems": true } } } });
        let mapped = map_discount(&node).unwrap();
        assert_eq!(
            value(&mapped.values, "discountValue"),
            &Sql::Integer(100_000)
        );
        assert_eq!(value(&mapped.values, "usedCount"), &Sql::Integer(2));
        assert_eq!(value(&mapped.values, "minimumValue"), &Sql::Integer(30_000));
        let mut collection = node.clone();
        collection["codeDiscount"]["customerGets"]["items"] =
            json!({ "__typename": "DiscountCollections" });
        assert!(map_discount(&collection).is_none());
    }

    #[test]
    fn order_states() {
        let mut order = json!({ "cancelledAt": null, "displayFinancialStatus": "PAID",
            "displayFulfillmentStatus": "UNFULFILLED", "fulfillments": [] });
        assert_eq!(order_state(&order).status, "accepted");
        order["displayFulfillmentStatus"] = json!("FULFILLED");
        order["fulfillments"] = json!([{ "displayStatus": "DELIVERED", "trackingInfo": [{ "company": "UPS", "number": "1", "url": null }] }]);
        let state = order_state(&order);
        assert_eq!(
            (state.status, state.tracking_company.as_deref()),
            ("concluded", Some("UPS"))
        );
        order["cancelledAt"] = json!("2026-01-01T00:00:00Z");
        assert_eq!(order_state(&order).status, "cancelled");
    }
}
