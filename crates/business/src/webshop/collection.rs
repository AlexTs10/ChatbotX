//! `packages/business/src/webshop/collection.ts`: the product cards a
//! `sendCollection` step sends to one contact.

use chatbotx_core::encryption::{SHOP_TOKEN_TTL_MS, ShopCustomerClaims};
use chatbotx_core::flow_config::steps::COLLECTION_MAX_PRODUCTS;
use chatbotx_core::utils::id::create_id;
use chatbotx_core::utils::money::{LinePriceInput, compute_line_price, stack_discount_percents};
use chatbotx_core::utils::web_url::{join_app_url, to_safe_web_url};
use chatbotx_db::Db;
use chatbotx_db::repo::product::{self as product_repo, ShopListInput, ShopProduct};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::order::messages;
use crate::store_source::resolve_store_source;
use crate::{ecommerce_settings, tx};

const CARD_TEXT_MAX: usize = 80;

/// `truncate` (UTF-16 length, as the original).
fn truncate(text: &str) -> String {
    if text.encode_utf16().count() <= CARD_TEXT_MAX {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut units = 0;
    for c in text.chars() {
        units += c.len_utf16();
        if units > CARD_TEXT_MAX - 1 {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn id(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) if !s.is_empty() => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// `listCollectionProducts`: what the shop shows, at most 10.
pub fn list_collection_products(
    conn: &Connection,
    workspace_id: i64,
    step: &Value,
) -> Result<Vec<ShopProduct>> {
    let source = Some(resolve_store_source(conn, workspace_id)?);
    if step["collectionType"] == "manual" {
        let ids: Vec<i64> = step["productIds"]
            .as_array()
            .map(|a| a.iter().filter_map(id).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let page = product_repo::list_for_shop(
            conn,
            &ShopListInput {
                workspace_id,
                source,
                product_ids: Some(ids.clone()),
                ..ShopListInput::default()
            },
        )
        .map_err(BusinessError::Repo)?;
        // In the order they were chosen.
        return Ok(ids
            .iter()
            .filter_map(|id| page.data.iter().find(|p| p.product.id == *id).cloned())
            .take(COLLECTION_MAX_PRODUCTS)
            .collect());
    }
    let conditions = &step["conditions"];
    let keyword = conditions["name"]
        .as_str()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned);
    let page = product_repo::list_for_shop(
        conn,
        &ShopListInput {
            workspace_id,
            source,
            keyword,
            category_id: id(&conditions["subcategoryId"]).or_else(|| id(&conditions["categoryId"])),
            vendor_id: id(&conditions["vendorId"]),
            page: Some(1),
            per_page: Some(COLLECTION_MAX_PRODUCTS as i64),
            ..ShopListInput::default()
        },
    )
    .map_err(BusinessError::Repo)?;
    Ok(page.data)
}

/// `buildWebshopUrl` for one product / category.
fn shop_url(
    app_url: &str,
    ws: i64,
    token: &str,
    product: Option<i64>,
    category: Option<i64>,
) -> String {
    let page = product
        .map(|id| format!("/products/{id}"))
        .unwrap_or_default();
    let mut query = format!("token={token}");
    if let Some(id) = category {
        query.push_str(&format!("&category={id}"));
    }
    join_app_url(app_url, &format!("/shop/{ws}{page}?{query}"))
}

fn link_button(label: &str, url: &str) -> Value {
    json!({
        "id": create_id(),
        "label": label,
        "buttonType": "openWebsite",
        "beforeStep": { "id": create_id(), "stepType": "openWebsite", "url": url, "browserSize": 100 },
        "steps": [],
    })
}

/// `priceText`: the product's and vendor's discounts stacked.
fn price_text(product: &ShopProduct, settings: &Value) -> String {
    let payments = messages::Payments::of(&settings["payments"]);
    let price = compute_line_price(&LinePriceInput {
        unit_price: product.product.price,
        quantity: 1.0,
        discount_percent: Some(stack_discount_percents(
            product.product.discount,
            product.vendor_discount_percent as f64,
        )),
        tax_percent: None,
        currency: payments.currency,
    });
    messages::money(
        price.gross - price.discount_total,
        payments.currency,
        &payments,
    )
}

/// `buildCollectionCards`.
pub fn build_collection_cards(
    products: &[ShopProduct],
    settings: &Value,
    workspace_id: i64,
    app_url: &str,
    shop_token: &str,
    language: &str,
) -> Vec<Value> {
    let labels = &messages::strings(language).collection;
    let label = |key: &str| labels.get(key).cloned().unwrap_or_default();
    let similar = settings["general"]["productButtons"]
        .as_array()
        .is_some_and(|b| b.iter().any(|v| v == "similar"));
    let external_page = settings["checkout"]["checkoutMode"] == "productPage";
    products
        .iter()
        .map(|product| {
            let p = &product.product;
            let external = external_page
                .then(|| to_safe_web_url(p.product_url.as_deref()))
                .flatten();
            let url = external
                .clone()
                .unwrap_or_else(|| shop_url(app_url, workspace_id, shop_token, Some(p.id), None));
            let mut buttons = vec![link_button(
                &label(if external.is_some() { "buy" } else { "view" }),
                &url,
            )];
            if similar && let Some(category) = p.category_id {
                buttons.push(link_button(
                    &label("similar"),
                    &shop_url(app_url, workspace_id, shop_token, None, Some(category)),
                ));
            }
            let subtitle = [Some(price_text(product, settings)), product.vendor.clone()]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let mut card = json!({
                "id": create_id(),
                "stepType": "sendCard",
                "title": truncate(p.name.trim()),
                "subtitle": truncate(&subtitle),
                "buttons": buttons,
            });
            let image = p.images.0.get(0).and_then(|i| i["url"].as_str());
            if let Some(image) = to_safe_web_url(image) {
                card["image"] = json!({
                    "id": create_id(), "stepType": "sendImage", "mode": "url", "url": image, "buttons": [],
                });
            }
            card
        })
        .collect()
}

/// `resolveCollectionStep`: the step with its `cards` for this contact, or
/// `None` when no product matches (the step is then skipped).
pub async fn resolve_collection_step(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
    app_url: &str,
    step: &Value,
) -> Result<Option<Value>> {
    let shop_token = crate::meta_catalog::process_encryption()
        .sign_shop_token(
            &ShopCustomerClaims {
                workspace_id: workspace_id.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: contact_inbox_id.map(|id| id.to_string()),
                conversation_id: None,
            },
            SHOP_TOKEN_TTL_MS,
        )
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?;
    let step_in = step.clone();
    let (products, settings, language) = tx::read(db, move |conn| {
        let language: Option<String> = conn
            .query_row(
                r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok((
            list_collection_products(conn, workspace_id, &step_in)?,
            ecommerce_settings::get_in(conn, workspace_id)?,
            language,
        ))
    })
    .await?;
    let language = messages::language(language.as_deref());
    let cards = build_collection_cards(
        &products,
        &settings,
        workspace_id,
        app_url,
        &shop_token,
        language,
    );
    if cards.is_empty() {
        return Ok(None);
    }
    let mut step = step.clone();
    step["cards"] = Value::Array(cards);
    Ok(Some(step))
}
