//! `integrations/meta-catalog/src/lib/{remote-product,mapper,retailer-id,
//! fingerprint}.ts` and `concurrencyForUsage` / `parseBucHeader`
//! (`integration-messenger/apis/usage.ts`).

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use ring::digest;
use serde_json::{Map, Value, json};

/// `CATALOG_BATCH_SIZE`.
pub const CATALOG_BATCH_SIZE: usize = 1000;
const MAX_IMAGES_PER_ITEM: usize = 21;
const MAX_TITLE_LENGTH: usize = 100;
const MAX_DESCRIPTION_LENGTH: usize = 5000;
const MAX_CUSTOM_LABELS: usize = 5;
const MAX_CUSTOM_LABEL_LENGTH: usize = 100;

/// `str.slice(0, n)` on UTF-16 units, approximated on chars.
fn take(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn opt_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// `Number(x)` for the parse helpers: a finite, non-negative number.
fn non_negative(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if s.is_empty() => return None,
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().ok()?
            }
        }
        _ => return None,
    };
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

static PRICE_NOISE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\d.,]").unwrap());
static PRICE_FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[.,](\d{1,2})$").unwrap());

/// `parsePriceAmount`.
fn parse_price(value: Option<&Value>) -> Option<f64> {
    let Some(Value::String(text)) = value else {
        return non_negative(value);
    };
    let digits = PRICE_NOISE.replace_all(text, "").into_owned();
    let fraction = PRICE_FRACTION.captures(&digits);
    let whole_part = match &fraction {
        Some(c) => &digits[..digits.len() - c[0].len()],
        None => digits.as_str(),
    };
    let whole: String = whole_part
        .chars()
        .filter(|c| *c != '.' && *c != ',')
        .collect();
    let joined = match &fraction {
        Some(c) => format!("{}.{}", if whole.is_empty() { "0" } else { &whole }, &c[1]),
        None => whole,
    };
    non_negative(Some(&Value::String(joined)))
}

/// `normalizeHttpUrl`: `new URL(value).toString()` for http(s) only.
fn normalize_http_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn collect_strings(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::String(s)) => out.push(s.clone()),
        Some(Value::Array(items)) => items.iter().for_each(|v| collect_strings(Some(v), out)),
        Some(Value::Object(map)) => map.values().for_each(|v| collect_strings(Some(v), out)),
        _ => {}
    }
}

fn unique_urls(values: &[Option<&Value>]) -> Vec<String> {
    let mut strings = Vec::new();
    for value in values {
        collect_strings(*value, &mut strings);
    }
    let mut out: Vec<String> = Vec::new();
    for url in strings.iter().filter_map(|s| normalize_http_url(s)) {
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// `toImportedMetaProduct`: `Ok(product)` (the `ImportedMetaProduct` JSON)
/// or `Err(reason)`.
pub fn to_imported_meta_product(remote: &Value) -> Result<Value, String> {
    let has = |key: &str| remote.get(key).is_some();
    let Some(retailer_id) = opt_text(remote.get("retailer_id")) else {
        return Err("Meta product is missing retailer_id".into());
    };
    let Some(name) = opt_text(remote.get("name")) else {
        return Err("Meta product is missing name".into());
    };
    let price = parse_price(remote.get("price"));
    if has("price") && price.is_none() {
        return Err("Meta product price is invalid".into());
    }
    let sale_price = parse_price(remote.get("sale_price"));
    let has_sale_price = match remote.get("sale_price") {
        None => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(_) => true,
    };
    if has_sale_price && sale_price.is_none() {
        return Err("Meta product sale price is invalid".into());
    }
    let mut out = Map::new();
    out.insert("retailerId".into(), json!(retailer_id));
    out.insert("name".into(), json!(name));
    out.insert("sku".into(), json!(retailer_id));
    if let Some(price) = price {
        let discount = match sale_price {
            Some(sale) if price > 0.0 && sale < price => {
                ((price - sale) / price * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        out.insert("price".into(), json!(price));
        out.insert("discount".into(), json!(discount));
    }
    if let Some(currency) = opt_text(remote.get("currency")) {
        out.insert("currency".into(), json!(currency.to_uppercase()));
    }
    if has("url") {
        let url = opt_text(remote.get("url"));
        // `value ? normalizeHttpUrl(value) : null`: an invalid URL is
        // `undefined` (the key stays out).
        match url {
            None => {
                out.insert("productUrl".into(), Value::Null);
            }
            Some(url) => {
                if let Some(url) = normalize_http_url(&url) {
                    out.insert("productUrl".into(), json!(url));
                }
            }
        }
    }
    if has("short_description") {
        out.insert(
            "shortDescription".into(),
            json!(opt_text(remote.get("short_description"))),
        );
    }
    if has("description") {
        out.insert(
            "longDescription".into(),
            json!(opt_text(remote.get("description"))),
        );
    }
    if has("product_type") {
        let parts: Vec<String> = remote
            .get("product_type")
            .and_then(Value::as_str)
            .map(|t| {
                t.split('>')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        out.insert("categoryName".into(), json!(parts.first()));
        out.insert(
            "subcategoryName".into(),
            json!((parts.len() > 1).then(|| parts[1..].join(" > "))),
        );
    }
    let labels: Vec<Option<&Value>> = (0..MAX_CUSTOM_LABELS)
        .map(|i| remote.get(format!("custom_label_{i}")))
        .collect();
    if labels.iter().any(Option::is_some) {
        let mut tags: Vec<String> = Vec::new();
        for label in labels.iter().filter_map(|l| opt_text(*l)) {
            if !tags.contains(&label) {
                tags.push(label);
            }
        }
        out.insert("tags".into(), json!(tags));
    }
    if has("brand") {
        out.insert("vendor".into(), json!(opt_text(remote.get("brand"))));
    }
    let availability = remote
        .get("availability")
        .and_then(Value::as_str)
        .map(|a| a.trim().to_lowercase());
    if let Some(quantity) = non_negative(remote.get("quantity_to_sell_on_facebook")) {
        let keeps_policy = quantity == 0.0
            && matches!(
                availability.as_deref(),
                Some("available for order" | "discontinued")
            );
        out.insert("inventoryQuantity".into(), json!(quantity.floor() as i64));
        if !keeps_policy {
            out.insert("inventoryPolicy".into(), json!("track"));
            out.insert("allowOutOfStockPurchase".into(), json!(false));
        }
    }
    let originals = unique_urls(&[
        remote.get("image_url"),
        remote.get("additional_image_urls"),
        remote.get("images"),
    ]);
    let cdn = unique_urls(&[
        remote.get("image_cdn_urls"),
        remote.get("additional_image_cdn_urls"),
    ]);
    let images = if originals.is_empty() { cdn } else { originals };
    if !images.is_empty() {
        out.insert(
            "images".into(),
            Value::Array(
                images
                    .into_iter()
                    .map(|url| json!({ "url": url, "type": "link" }))
                    .collect(),
            ),
        );
    }
    if let Some(availability) = availability.filter(|a| !a.is_empty()) {
        let status = if availability == "discontinued" {
            "draft"
        } else {
            "active"
        };
        out.insert("status".into(), json!(status));
    }
    Ok(Value::Object(out))
}

/// The `CatalogProduct` fields `toMetaItem` reads.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogProduct {
    pub id: String,
    pub sku: Option<String>,
    pub name: String,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
    pub price: f64,
    pub discount: f64,
    pub currency: Option<String>,
    pub product_url: Option<String>,
    pub inventory_policy: String,
    pub inventory_quantity: i64,
    pub allow_out_of_stock_purchase: bool,
    pub status: String,
    pub image_urls: Vec<String>,
    pub tags: Vec<String>,
    pub vendor: Option<String>,
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub has_variants: bool,
}

/// `CatalogSettings`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogSettings {
    pub currency: String,
    pub store_url: Option<String>,
    pub workspace_name: String,
}

/// `MetaItemMapping`.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaItem {
    Skipped {
        product_id: String,
        reason: &'static str,
    },
    Mapped {
        product_id: String,
        retailer_id: String,
        data: Value,
    },
}

static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

fn strip_html(value: &str) -> String {
    let no_tags = HTML_TAG.replace_all(value, " ");
    SPACES.replace_all(&no_tags, " ").trim().to_owned()
}

fn description(product: &CatalogProduct) -> String {
    let long = strip_html(product.long_description.as_deref().unwrap_or(""));
    if long.is_empty() {
        strip_html(product.short_description.as_deref().unwrap_or(""))
    } else {
        long
    }
}

/// `formatAmount`: `Number(x.toFixed(d)).toString()` plus the currency.
fn format_amount(amount: f64, currency: &str) -> String {
    let currency = currency.to_uppercase();
    let digits = if currency == "VND" { 0 } else { 2 };
    let fixed: f64 = format!("{amount:.digits$}").parse().unwrap_or(0.0);
    format!("{} {currency}", chatbotx_core::js::number_to_string(fixed))
}

/// `resolveMetaAvailability`.
pub fn meta_availability(product: &CatalogProduct) -> &'static str {
    if product.status != "active" {
        "discontinued"
    } else if product.inventory_policy == "dont_track" {
        "available for order"
    } else if product.inventory_quantity > 0 {
        "in stock"
    } else if product.allow_out_of_stock_purchase {
        "available for order"
    } else {
        "out of stock"
    }
}

/// `toMetaItem(product, settings, retailerId = product.id)`.
pub fn to_meta_item(
    product: &CatalogProduct,
    settings: &CatalogSettings,
    retailer_id: Option<&str>,
) -> MetaItem {
    let product_url = product
        .product_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let skip = if !product.image_urls.iter().any(|u| !u.trim().is_empty()) {
        Some("missingImage")
    } else if description(product).is_empty() {
        Some("missingDescription")
    } else if product_url.is_none() && settings.store_url.as_deref().is_none_or(str::is_empty) {
        Some("missingStoreUrl")
    } else if product.has_variants {
        Some("hasVariants")
    } else {
        None
    };
    if let Some(reason) = skip {
        return MetaItem::Skipped {
            product_id: product.id.clone(),
            reason,
        };
    }
    let currency = product
        .currency
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(&settings.currency)
        .to_owned();
    let sale_price = (product.discount > 0.0)
        .then(|| (product.price * (1.0 - product.discount / 100.0)).max(0.0));
    let product_type = [&product.category, &product.subcategory]
        .into_iter()
        .filter_map(|n| n.as_deref().map(str::trim).filter(|n| !n.is_empty()))
        .collect::<Vec<_>>()
        .join(" > ");
    let tags: Vec<String> = product
        .tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .take(MAX_CUSTOM_LABELS)
        .map(|t| take(t, MAX_CUSTOM_LABEL_LENGTH))
        .collect();
    let label = |i: usize| tags.get(i).cloned().unwrap_or_default();
    let link = match product_url {
        Some(url) => url.to_owned(),
        None => format!(
            "{}/{}",
            settings
                .store_url
                .as_deref()
                .unwrap_or("")
                .trim_end_matches('/'),
            product.id
        ),
    };
    let brand = product
        .vendor
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(&settings.workspace_name);
    MetaItem::Mapped {
        product_id: product.id.clone(),
        retailer_id: retailer_id.unwrap_or(&product.id).to_owned(),
        data: json!({
            "title": take(&product.name, MAX_TITLE_LENGTH),
            "description": take(&description(product), MAX_DESCRIPTION_LENGTH),
            "short_description": take(&strip_html(product.short_description.as_deref().unwrap_or("")), MAX_DESCRIPTION_LENGTH),
            "availability": meta_availability(product),
            "condition": "new",
            "image": product.image_urls.iter().filter(|u| !u.trim().is_empty()).take(MAX_IMAGES_PER_ITEM)
                .map(|url| json!({ "url": url, "tag": [] })).collect::<Vec<_>>(),
            "link": link,
            "price": format_amount(product.price, &currency),
            "sale_price": sale_price.map(|s| format_amount(s, &currency)).unwrap_or_default(),
            "brand": brand,
            "product_type": product_type,
            "quantity_to_sell_on_facebook": if product.inventory_policy == "track" { product.inventory_quantity } else { 0 },
            "custom_label_0": label(0),
            "custom_label_1": label(1),
            "custom_label_2": label(2),
            "custom_label_3": label(3),
            "custom_label_4": label(4),
        }),
    }
}

/// `sortObject` keys sorted (serde_json `Map` is a `BTreeMap`; the
/// `localeCompare` order equals byte order for these ASCII keys).
fn sort_object(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sort_object).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), sort_object(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `fingerprintMetaItem`: hex SHA-256 of the key-sorted JSON.
pub fn fingerprint_meta_item(item: &Value) -> String {
    let hash = digest::digest(&digest::SHA256, sort_object(item).to_string().as_bytes());
    hash.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// `resolveRetailerIds`: product id → retailer id.
pub fn resolve_retailer_ids(
    products: &[(String, Option<String>)],
    linked_by_product_id: &HashMap<String, String>,
    owner_by_retailer_id: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut owner_of: HashMap<String, String> = products
        .iter()
        .map(|(id, _)| (id.clone(), id.clone()))
        .collect();
    for (retailer, product) in owner_by_retailer_id {
        owner_of.insert(retailer.clone(), product.clone());
    }
    for (id, _) in products {
        if let Some(linked) = linked_by_product_id.get(id) {
            owner_of.insert(linked.clone(), id.clone());
        }
    }
    let mut resolved = HashMap::new();
    for (id, sku) in products {
        if let Some(linked) = linked_by_product_id.get(id) {
            resolved.insert(id.clone(), linked.clone());
            continue;
        }
        let sku = sku.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(sku) = sku
            && owner_of.get(sku).is_none_or(|owner| owner == id)
        {
            owner_of.insert(sku.to_owned(), id.clone());
            resolved.insert(id.clone(), sku.to_owned());
            continue;
        }
        resolved.insert(id.clone(), id.clone());
    }
    resolved
}

/// `BucUsage`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BucUsage {
    pub call_count: f64,
    pub total_cpu_time: f64,
    pub total_time: f64,
    pub estimated_time_to_regain_access: f64,
}

/// `parseBucHeader`: the first entry of the first bucket.
pub fn parse_buc_header(header: Option<&str>) -> Option<BucUsage> {
    let parsed: Value = serde_json::from_str(header?).ok()?;
    let entry = parsed.as_object()?.values().next()?.as_array()?.first()?;
    let num = |key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    Some(BucUsage {
        call_count: num("call_count"),
        total_cpu_time: num("total_cputime"),
        total_time: num("total_time"),
        estimated_time_to_regain_access: num("estimated_time_to_regain_access"),
    })
}

/// `concurrencyForUsage`.
pub fn concurrency_for_usage(usage: Option<&BucUsage>) -> u32 {
    let Some(usage) = usage else {
        return 5;
    };
    if usage.estimated_time_to_regain_access > 0.0 {
        return 0;
    }
    let peak = usage
        .call_count
        .max(usage.total_cpu_time)
        .max(usage.total_time);
    match peak {
        p if p < 50.0 => 5,
        p if p < 75.0 => 3,
        p if p < 90.0 => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    //! `integrations/meta-catalog/__tests__/{remote-product,mapper,retailer-id}.test.ts`.
    use super::*;

    // ---- remote-product.test.ts ----

    fn imported(remote: Value) -> Value {
        to_imported_meta_product(&remote).expect("ok")
    }

    /// `expect.objectContaining(expected)`.
    fn contains(product: &Value, expected: Value) {
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(product.get(key), Some(value), "{key}");
        }
    }

    /// `expect.not.objectContaining({ k: expect.anything(), .. })`: not every
    /// key is set (to a non-null value).
    fn not_contains(product: &Value, keys: &[&str]) {
        assert!(
            !keys
                .iter()
                .all(|k| product.get(*k).is_some_and(|v| !v.is_null()))
        );
    }

    #[test]
    fn maps_a_valid_remote_product_and_preserves_retailer_identity() {
        let product = imported(json!({
            "id": "graph-product-1", "retailer_id": "merchant-sku-1", "name": "Imported product",
            "description": "Description", "short_description": "Short description",
            "price": "100", "sale_price": "80", "image_url": "https://cdn.example.com/main.jpg",
            "additional_image_urls": ["https://cdn.example.com/main.jpg", "https://cdn.example.com/second.jpg"],
            "product_type": "Shoes", "custom_label_0": "featured", "custom_label_1": "summer",
            "quantity_to_sell_on_facebook": "4", "availability": "in stock",
        }));
        contains(
            &product,
            json!({
                "retailerId": "merchant-sku-1", "sku": "merchant-sku-1", "price": 100.0, "discount": 20.0,
                "categoryName": "Shoes", "longDescription": "Description", "shortDescription": "Short description",
                "tags": ["featured", "summer"], "inventoryQuantity": 4, "inventoryPolicy": "track",
                "allowOutOfStockPurchase": false,
                "images": [
                    { "type": "link", "url": "https://cdn.example.com/main.jpg" },
                    { "type": "link", "url": "https://cdn.example.com/second.jpg" },
                ],
            }),
        );
    }

    #[test]
    fn rejects_incomplete_remote_product_data() {
        for (remote, reason) in [
            (json!({ "name": "Missing retailer" }), "missing retailer_id"),
            (json!({ "retailer_id": "remote-1" }), "missing name"),
            (
                json!({ "retailer_id": "remote-1", "name": "Bad price", "price": "not-a-number" }),
                "price is invalid",
            ),
        ] {
            assert!(
                to_imported_meta_product(&remote)
                    .unwrap_err()
                    .contains(reason)
            );
        }
    }

    #[test]
    fn parses_display_prices() {
        for (price, expected) in [
            ("100", 100.0),
            ("$9.99", 9.99),
            ("9.99 USD", 9.99),
            ("199.000₫", 199_000.0),
            ("199,000 VND", 199_000.0),
            ("1,234.56", 1234.56),
            ("14,50 EUR", 14.5),
        ] {
            let product = imported(
                json!({ "retailer_id": "remote-1", "name": "Priced product", "price": price }),
            );
            assert_eq!(product["price"], json!(expected), "{price}");
        }
    }

    #[test]
    fn carries_the_currency_and_storefront_link_back_to_the_local_product() {
        let product = imported(json!({
            "retailer_id": "remote-1", "name": "Imported product", "price": "199.000₫",
            "currency": "vnd", "url": "https://brand.example.com/p/abc",
        }));
        contains(
            &product,
            json!({ "currency": "VND", "productUrl": "https://brand.example.com/p/abc" }),
        );
    }

    #[test]
    fn drops_an_unsafe_storefront_link_instead_of_storing_it() {
        let product = imported(
            json!({ "retailer_id": "remote-1", "name": "Imported product", "url": "javascript:alert(1)" }),
        );
        not_contains(&product, &["productUrl"]);
    }

    #[test]
    fn drops_unsafe_image_protocols_without_rejecting_the_product() {
        let product = imported(json!({
            "retailer_id": "remote-1", "name": "Safe product", "image_url": "file:///etc/passwd",
            "additional_image_urls": ["javascript:alert(1)"],
        }));
        not_contains(&product, &["images"]);
    }

    #[test]
    fn prefers_original_localized_images_over_meta_cdn_copies() {
        let product = imported(json!({
            "retailer_id": "remote-images", "name": "Product with images", "image_url": "",
            "additional_image_urls": [],
            "images": { "en_US": [{ "url": "https://origin.example.com/main.jpg", "tag": ["en_US"] }] },
            "image_cdn_urls": { "en_US": "https://cdn.example.com/main.jpg" },
            "additional_image_cdn_urls": ["https://cdn.example.com/second.jpg", "https://origin.example.com/main.jpg"],
        }));
        contains(
            &product,
            json!({ "images": [{ "type": "link", "url": "https://origin.example.com/main.jpg" }] }),
        );
    }

    #[test]
    fn falls_back_to_meta_cdn_images_when_original_image_fields_are_empty() {
        let product = imported(json!({
            "retailer_id": "remote-cdn-images", "name": "Product with CDN images", "image_url": "",
            "additional_image_urls": [],
            "image_cdn_urls": { "en_US": "https://cdn.example.com/main.jpg" },
            "additional_image_cdn_urls": ["https://cdn.example.com/second.jpg", "https://cdn.example.com/main.jpg"],
        }));
        contains(
            &product,
            json!({ "images": [
                { "type": "link", "url": "https://cdn.example.com/main.jpg" },
                { "type": "link", "url": "https://cdn.example.com/second.jpg" },
            ] }),
        );
    }

    #[test]
    fn preserves_ambiguous_available_for_order_inventory_policy() {
        let product = imported(json!({
            "retailer_id": "remote-inventory", "name": "Backordered product",
            "quantity_to_sell_on_facebook": "0", "availability": "available for order",
            "product_type": "Home > Robot vacuums",
        }));
        contains(
            &product,
            json!({ "categoryName": "Home", "subcategoryName": "Robot vacuums", "inventoryQuantity": 0 }),
        );
        not_contains(&product, &["inventoryPolicy", "allowOutOfStockPurchase"]);
    }

    #[test]
    fn preserves_inventory_policy_when_a_zero_quantity_product_is_discontinued() {
        let product = imported(json!({
            "retailer_id": "remote-inactive-untracked", "name": "Inactive untracked product",
            "quantity_to_sell_on_facebook": "0", "availability": "discontinued",
        }));
        contains(
            &product,
            json!({ "inventoryQuantity": 0, "status": "draft" }),
        );
        not_contains(&product, &["inventoryPolicy", "allowOutOfStockPurchase"]);
    }

    #[test]
    fn rejects_malformed_sale_price_instead_of_clearing_local_discount() {
        let error = to_imported_meta_product(&json!({
            "retailer_id": "remote-invalid-sale", "name": "Invalid sale", "price": "100 USD", "sale_price": "not-a-price",
        }))
        .unwrap_err();
        assert!(error.contains("sale price is invalid"));
    }

    #[test]
    fn does_not_invent_inventory_settings_when_meta_omits_quantity() {
        let product = imported(
            json!({ "retailer_id": "remote-no-inventory", "name": "Ads-only product", "availability": "in stock" }),
        );
        not_contains(
            &product,
            &[
                "price",
                "discount",
                "inventoryQuantity",
                "inventoryPolicy",
                "allowOutOfStockPurchase",
                "status",
            ],
        );
    }

    #[test]
    fn preserves_explicit_clears_for_nullable_local_product_fields() {
        let product = imported(json!({
            "retailer_id": "remote-cleared-fields", "name": "Cleared product", "description": "",
            "short_description": "", "brand": "", "product_type": "", "url": "",
        }));
        contains(
            &product,
            json!({
                "productUrl": null, "shortDescription": null, "longDescription": null,
                "vendor": null, "categoryName": null, "subcategoryName": null,
            }),
        );
    }

    // ---- mapper.test.ts ----

    fn product() -> CatalogProduct {
        CatalogProduct {
            id: "123456789".into(),
            name: "Product".into(),
            short_description: Some("Description".into()),
            price: 9.99,
            inventory_policy: "track".into(),
            inventory_quantity: 5,
            status: "active".into(),
            image_urls: vec!["https://example.com/one.jpg".into()],
            vendor: Some("ChatbotX".into()),
            ..CatalogProduct::default()
        }
    }

    fn settings() -> CatalogSettings {
        CatalogSettings {
            currency: "USD".into(),
            store_url: Some("https://shop.example.com".into()),
            workspace_name: "Example Store".into(),
        }
    }

    fn data(product: &CatalogProduct, settings: &CatalogSettings) -> Value {
        match to_meta_item(product, settings, None) {
            MetaItem::Mapped { data, .. } => data,
            MetaItem::Skipped { reason, .. } => panic!("skipped: {reason}"),
        }
    }

    #[test]
    fn formats_price_using_catalog_settings() {
        for (price, currency, expected) in [
            (9.99, "USD", "9.99 USD"),
            (14.0, "GBP", "14 GBP"),
            (199_000.0, "VND", "199000 VND"),
        ] {
            let p = CatalogProduct { price, ..product() };
            let s = CatalogSettings {
                currency: currency.into(),
                ..settings()
            };
            // The fixture has no product currency (`CatalogProduct.currency` unset).
            assert_eq!(data(&p, &s)["price"], expected);
        }
    }

    #[test]
    fn uses_immutable_product_id_and_preserves_all_mapped_limits() {
        let p = CatalogProduct {
            name: "x".repeat(120),
            short_description: Some(format!("<p>{}</p>", "d".repeat(5100))),
            image_urls: (0..25)
                .map(|i| format!("https://example.com/{i}.jpg"))
                .collect(),
            ..product()
        };
        let MetaItem::Mapped {
            retailer_id, data, ..
        } = to_meta_item(&p, &settings(), None)
        else {
            panic!("skipped");
        };
        assert_eq!(retailer_id, "123456789");
        assert_eq!(data["title"].as_str().unwrap().chars().count(), 100);
        assert_eq!(data["description"].as_str().unwrap().chars().count(), 5000);
        assert!(!data["description"].as_str().unwrap().contains("<p>"));
        assert_eq!(data["image"].as_array().unwrap().len(), 21);
    }

    #[test]
    fn resolves_availability_from_the_rule_table() {
        let cases: [(CatalogProduct, &str); 6] = [
            (
                CatalogProduct {
                    status: "draft".into(),
                    ..product()
                },
                "discontinued",
            ),
            (
                CatalogProduct {
                    status: "unlisted".into(),
                    ..product()
                },
                "discontinued",
            ),
            (
                CatalogProduct {
                    inventory_policy: "dont_track".into(),
                    ..product()
                },
                "available for order",
            ),
            (
                CatalogProduct {
                    inventory_quantity: 1,
                    ..product()
                },
                "in stock",
            ),
            (
                CatalogProduct {
                    inventory_quantity: 0,
                    allow_out_of_stock_purchase: true,
                    ..product()
                },
                "available for order",
            ),
            (
                CatalogProduct {
                    inventory_quantity: 0,
                    ..product()
                },
                "out of stock",
            ),
        ];
        for (p, expected) in cases {
            assert_eq!(meta_availability(&p), expected);
        }
    }

    #[test]
    fn sends_zero_stock_to_clear_stale_counters_for_untracked_products() {
        let p = CatalogProduct {
            inventory_policy: "dont_track".into(),
            inventory_quantity: 42,
            ..product()
        };
        assert_eq!(data(&p, &settings())["quantity_to_sell_on_facebook"], 0);
    }

    #[test]
    fn keeps_inactive_untracked_products_distinguishable_on_round_trip() {
        let p = CatalogProduct {
            inventory_policy: "dont_track".into(),
            inventory_quantity: 42,
            status: "draft".into(),
            ..product()
        };
        let data = data(&p, &settings());
        assert_eq!(data["availability"], "discontinued");
        assert_eq!(data["quantity_to_sell_on_facebook"], 0);
    }

    #[test]
    fn reports_each_explicit_skip_reason() {
        let cases = [
            (
                CatalogProduct {
                    image_urls: vec![],
                    ..product()
                },
                settings(),
                "missingImage",
            ),
            (
                CatalogProduct {
                    short_description: None,
                    long_description: None,
                    ..product()
                },
                settings(),
                "missingDescription",
            ),
            (
                product(),
                CatalogSettings {
                    store_url: None,
                    ..settings()
                },
                "missingStoreUrl",
            ),
            (
                CatalogProduct {
                    has_variants: true,
                    ..product()
                },
                settings(),
                "hasVariants",
            ),
        ];
        for (p, s, reason) in cases {
            assert_eq!(
                to_meta_item(&p, &s, None),
                MetaItem::Skipped {
                    product_id: "123456789".into(),
                    reason
                }
            );
        }
    }

    #[test]
    fn prefers_the_product_currency_over_the_catalog_wide_one() {
        let p = CatalogProduct {
            currency: Some("VND".into()),
            price: 199_000.0,
            ..product()
        };
        assert_eq!(data(&p, &settings())["price"], "199000 VND");
    }

    #[test]
    fn prefers_the_product_url_over_the_derived_store_link() {
        let p = CatalogProduct {
            product_url: Some("https://brand.example.com/p/abc".into()),
            ..product()
        };
        assert_eq!(
            data(&p, &settings())["link"],
            "https://brand.example.com/p/abc"
        );
    }

    #[test]
    fn keeps_short_and_long_descriptions_distinct_and_exports_product_tags() {
        let p = CatalogProduct {
            short_description: Some("Short summary".into()),
            long_description: Some("<p>Full product description</p>".into()),
            tags: [" Summer ", "", "featured", "sale", "vip", "last"]
                .map(String::from)
                .to_vec(),
            ..product()
        };
        let data = data(&p, &settings());
        assert_eq!(data["description"], "Full product description");
        assert_eq!(data["short_description"], "Short summary");
        for (i, label) in ["Summer", "featured", "sale", "vip", "last"]
            .iter()
            .enumerate()
        {
            assert_eq!(data[format!("custom_label_{i}")], *label);
        }
    }

    #[test]
    fn clears_unused_custom_labels_and_short_description_explicitly() {
        let p = CatalogProduct {
            short_description: None,
            long_description: Some("Description".into()),
            tags: vec![],
            ..product()
        };
        let data = data(&p, &settings());
        assert_eq!(data["short_description"], "");
        for i in 0..5 {
            assert_eq!(data[format!("custom_label_{i}")], "");
        }
    }

    #[test]
    fn falls_back_to_short_description_when_long_description_has_no_content() {
        let p = CatalogProduct {
            short_description: Some("Useful summary".into()),
            long_description: Some("<p> </p>".into()),
            ..product()
        };
        assert_eq!(data(&p, &settings())["description"], "Useful summary");
    }

    fn with_category(category: Option<&str>, subcategory: Option<&str>) -> CatalogProduct {
        CatalogProduct {
            category: category.map(str::to_owned),
            subcategory: subcategory.map(str::to_owned),
            ..product()
        }
    }

    #[test]
    fn sends_the_category_path_as_the_product_type() {
        for (category, subcategory, expected) in [
            (Some("Men"), Some("Shirts"), "Men > Shirts"),
            (Some("Men"), None, "Men"),
            (None, Some("Shirts"), "Shirts"),
            (Some("  Men  "), Some("  Shirts  "), "Men > Shirts"),
            (Some("Men"), Some("   "), "Men"),
        ] {
            assert_eq!(
                data(&with_category(category, subcategory), &settings())["product_type"],
                expected
            );
        }
    }

    #[test]
    fn clears_the_product_type_when_no_category_name_survives() {
        for (category, subcategory) in [(None, None), (None, None), (Some("   "), Some(""))] {
            assert_eq!(
                data(&with_category(category, subcategory), &settings())["product_type"],
                ""
            );
        }
    }

    #[test]
    fn clears_the_remote_sale_price_when_the_local_discount_is_removed() {
        assert_eq!(
            data(
                &CatalogProduct {
                    discount: 0.0,
                    ..product()
                },
                &settings()
            )["sale_price"],
            ""
        );
    }

    #[test]
    fn keeps_a_product_with_its_own_url_when_the_store_url_is_unset() {
        let p = CatalogProduct {
            product_url: Some("https://brand.example.com/p/abc".into()),
            ..product()
        };
        let s = CatalogSettings {
            store_url: None,
            ..settings()
        };
        assert!(matches!(
            to_meta_item(&p, &s, None),
            MetaItem::Mapped { .. }
        ));
    }

    // ---- retailer-id.test.ts ----

    fn resolve(
        products: &[(&str, Option<&str>)],
        linked: &[(&str, &str)],
        owners: &[(&str, &str)],
    ) -> HashMap<String, String> {
        let products: Vec<(String, Option<String>)> = products
            .iter()
            .map(|(id, sku)| ((*id).to_owned(), sku.map(str::to_owned)))
            .collect();
        let pairs = |list: &[(&str, &str)]| {
            list.iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect()
        };
        resolve_retailer_ids(&products, &pairs(linked), &pairs(owners))
    }

    #[test]
    fn sends_the_sku_as_the_content_id_when_one_is_set() {
        assert_eq!(
            resolve(&[("product-1", Some("TSHIRT-BLK-M"))], &[], &[])["product-1"],
            "TSHIRT-BLK-M"
        );
    }

    #[test]
    fn falls_back_to_the_product_id_when_the_sku_is_missing_null_or_blank() {
        // `undefined` and `null` are both `None` in Rust.
        for sku in [None, None, Some("   ")] {
            assert_eq!(
                resolve(&[("product-1", sku)], &[], &[])["product-1"],
                "product-1"
            );
        }
    }

    #[test]
    fn trims_the_sku_so_trailing_spaces_cannot_fork_the_content_id() {
        assert_eq!(
            resolve(&[("product-1", Some(" SKU-1 "))], &[], &[])["product-1"],
            "SKU-1"
        );
    }

    #[test]
    fn keeps_the_id_an_already_pushed_item_carries_sku_or_not() {
        let resolved = resolve(
            &[("product-1", Some("SKU-1"))],
            &[("product-1", "legacy-retailer-id")],
            &[],
        );
        assert_eq!(resolved["product-1"], "legacy-retailer-id");
    }

    #[test]
    fn gives_a_duplicated_sku_to_the_first_product_only() {
        let resolved = resolve(
            &[
                ("product-1", Some("SKU-DUP")),
                ("product-2", Some("SKU-DUP")),
            ],
            &[],
            &[],
        );
        assert_eq!(resolved["product-1"], "SKU-DUP");
        assert_eq!(resolved["product-2"], "product-2");
    }

    #[test]
    fn leaves_a_sku_alone_when_another_product_already_owns_it_in_the_catalog() {
        let resolved = resolve(
            &[("product-2", Some("SKU-IMPORTED"))],
            &[],
            &[("SKU-IMPORTED", "product-1")],
        );
        assert_eq!(resolved["product-2"], "product-2");
    }

    #[test]
    fn reuses_a_sku_the_product_itself_already_owns() {
        let resolved = resolve(
            &[("product-1", Some("SKU-1"))],
            &[],
            &[("SKU-1", "product-1")],
        );
        assert_eq!(resolved["product-1"], "SKU-1");
    }

    #[test]
    fn refuses_a_sku_that_would_steal_another_products_fallback_id() {
        let resolved = resolve(
            &[("product-1", Some("product-2")), ("product-2", None)],
            &[],
            &[],
        );
        assert_eq!(resolved["product-1"], "product-1");
        assert_eq!(resolved["product-2"], "product-2");
    }

    #[test]
    fn assigns_every_product_exactly_one_unique_content_id() {
        let products = [
            ("product-1", Some("SKU-DUP")),
            ("product-2", Some("SKU-DUP")),
            ("product-3", None),
            ("product-4", Some("SKU-TAKEN")),
            ("product-5", Some("SKU-5")),
        ];
        let resolved = resolve(
            &products,
            &[("product-3", "legacy-3")],
            &[("SKU-TAKEN", "other-product")],
        );
        let ids: Vec<&str> = products
            .iter()
            .map(|(id, _)| resolved[*id].as_str())
            .collect();
        assert_eq!(
            ids,
            ["SKU-DUP", "product-2", "legacy-3", "product-4", "SKU-5"]
        );
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            products.len()
        );
    }

    // ---- fingerprint / usage (no TS test file) ----

    #[test]
    fn fingerprints_ignore_key_order() {
        assert_eq!(
            fingerprint_meta_item(&json!({ "a": 1, "b": 2 })),
            fingerprint_meta_item(&json!({ "b": 2, "a": 1 }))
        );
    }

    #[test]
    fn concurrency_follows_the_business_usage() {
        assert_eq!(concurrency_for_usage(None), 5);
        let usage = parse_buc_header(Some(
            r#"{"1":[{"call_count":80,"total_cputime":1,"total_time":1}]}"#,
        ));
        assert_eq!(concurrency_for_usage(usage.as_ref()), 1);
        let blocked = parse_buc_header(Some(r#"{"1":[{"estimated_time_to_regain_access":3}]}"#));
        assert_eq!(concurrency_for_usage(blocked.as_ref()), 0);
    }
}
