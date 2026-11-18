//! `packages/business/src/woocommerce/{sync.service,webhooks}.ts`: the worker
//! side. A full sync mirrors the catalog, the coupons and the last 60 days of
//! orders; a webhook re-reads its one object from the store.

use chatbotx_core::utils::money::round_money;
use chatbotx_db::repo::external_store::{self as store_repo, ExternalVariantRow};
use chatbotx_db::repo::models::IntegrationWoocommerce;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::Value;

use super::client::{self, ApiError, Credentials};
use super::mappers::{self, numeric, text};
use super::{SOURCE, find, mark_invalid, refresh_store_currency, resolve_credentials};
use crate::errors::{BusinessError, Result};
use crate::external_store::{
    self, EXTERNAL_CONTACT_ATTRIBUTE, MirrorItem, MirrorOrder, find_mirrored,
    remove_ordered_from_cart, resolve_external_order_contact, sync_error_message, variant_label_of,
};
use crate::order::Deps;
use crate::shopify::sync::{save_mapped_discount, save_mapped_product};
use crate::tx;

/// The order meta our Web Checkout stamps on the pending order it creates.
pub const CHECKOUT_META: &str = "cbx_checkout";
pub const WEB_CHECKOUT: &str = "web";
/// Statuses of an unpaid order that has not gone anywhere yet.
const NOT_YET_PLACED: &[&str] = &["pending", "failed", "cancelled"];
/// Products read again for their stock after one order.
const STOCK_REFRESH_LIMIT: usize = 20;

/// A failed step: the store's refusal, or ours.
#[derive(Debug)]
pub enum SyncError {
    Api(ApiError),
    Business(BusinessError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Api(e) => f.write_str(&e.message),
            SyncError::Business(e) => write!(f, "{e}"),
        }
    }
}

impl From<ApiError> for SyncError {
    fn from(e: ApiError) -> Self {
        SyncError::Api(e)
    }
}

impl From<BusinessError> for SyncError {
    fn from(e: BusinessError) -> Self {
        SyncError::Business(e)
    }
}

pub type SyncResult<T> = std::result::Result<T, SyncError>;

struct Context {
    row: IntegrationWoocommerce,
    credentials: Credentials,
}

async fn update(db: &chatbotx_db::Db, ws: i64, values: Vec<(&'static str, Sql)>) -> Result<()> {
    external_store::update_connection(db, SOURCE, ws, values).await
}

/// `wooCommerceSyncService.run`: the worker's full sync.
pub async fn run(deps: Deps<'_>, workspace_id: i64, scope: &str) -> SyncResult<()> {
    let Some(row) = find(deps.db, workspace_id).await? else {
        return Ok(());
    };
    update(
        deps.db,
        workspace_id,
        vec![
            ("syncStatus", Sql::Text("running".into())),
            ("syncError", Sql::Null),
        ],
    )
    .await?;
    let order_count = row.order_count;
    match run_steps(deps, workspace_id, scope).await {
        Ok(orders) => {
            let products = tx::read(deps.db, move |conn| {
                Ok(store_repo::count_products(conn, workspace_id, SOURCE)?)
            })
            .await?;
            update(
                deps.db,
                workspace_id,
                vec![
                    ("syncStatus", Sql::Text("succeeded".into())),
                    ("syncError", Sql::Null),
                    ("lastSyncedAt", Sql::Integer(chatbotx_db::repo::now_ms())),
                    ("productCount", Sql::Integer(products)),
                    ("orderCount", Sql::Integer(orders.unwrap_or(order_count))),
                    ("status", Sql::Text("active".into())),
                    ("statusReason", Sql::Null),
                ],
            )
            .await?;
            Ok(())
        }
        Err(error) => {
            let message = sync_error_message(&error.to_string());
            update(
                deps.db,
                workspace_id,
                vec![
                    ("syncStatus", Sql::Text("failed".into())),
                    ("syncError", Sql::Text(message.clone())),
                ],
            )
            .await?;
            if let SyncError::Api(api) = &error
                && api.unauthorized()
            {
                mark_invalid(deps.db, workspace_id, &message).await?;
                return Ok(());
            }
            Err(error)
        }
    }
}

async fn run_steps(deps: Deps<'_>, workspace_id: i64, scope: &str) -> SyncResult<Option<i64>> {
    // The store's currency first: products are priced in it.
    refresh_store_currency(deps.db, workspace_id).await?;
    let Some(row) = find(deps.db, workspace_id).await? else {
        return Ok(None);
    };
    let credentials = resolve_credentials(&row)?;
    let context = Context { row, credentials };
    if scope != "orders" {
        sync_catalog(deps.db, &context).await?;
        sync_coupons(deps.db, &context).await?;
    }
    if scope != "products" {
        return Ok(Some(sync_orders(deps, &context).await?));
    }
    Ok(None)
}

/// `context`: an active connection with working credentials.
async fn context(db: &chatbotx_db::Db, workspace_id: i64) -> SyncResult<Option<Context>> {
    let Some(row) = find(db, workspace_id)
        .await?
        .filter(|r| r.status == "active")
    else {
        return Ok(None);
    };
    let credentials = resolve_credentials(&row)?;
    Ok(Some(Context { row, credentials }))
}

/// `syncProduct`: a variation reads its variable product; a product gone
/// (or no longer orderable) is removed here.
async fn sync_product(db: &chatbotx_db::Db, context: &Context, product_id: &str) -> SyncResult<()> {
    let mut product = client::get_product(&context.credentials, product_id).await?;
    if let Some(parent) = product
        .as_ref()
        .and_then(|p| p["parent_id"].as_i64())
        .filter(|id| *id > 0)
    {
        product = client::get_product(&context.credentials, &parent.to_string()).await?;
        if product
            .as_ref()
            .is_some_and(|p| p["parent_id"].as_i64().is_some_and(|id| id > 0))
        {
            return Ok(()); // ponytail: WooCommerce has one level of parents.
        }
        return match product {
            Some(p) if mappers::is_orderable(&p) => save_product(db, context, &p).await,
            _ => delete_product(db, context, &parent.to_string()).await,
        };
    }
    match product {
        Some(p) if mappers::is_orderable(&p) => save_product(db, context, &p).await,
        _ => delete_product(db, context, product_id).await,
    }
}

/// `deleteProduct`: a deleted variation re-reads its product instead.
async fn delete_product(
    db: &chatbotx_db::Db,
    context: &Context,
    product_id: &str,
) -> SyncResult<()> {
    let ws = context.row.workspace_id;
    let ids = vec![product_id.to_string()];
    let parent = tx::read(db, move |conn| {
        Ok(store_repo::find_variants_for_variations(
            conn, ws, SOURCE, &ids,
        )?)
    })
    .await?
    .into_iter()
    .next()
    .and_then(|v| v.product_external_id);
    if let Some(parent) = parent.filter(|p| p != product_id) {
        return Box::pin(sync_product(db, context, &parent)).await;
    }
    let id = product_id.to_string();
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_product_by_external_id(conn, ws, SOURCE, &id)?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn sync_coupon(db: &chatbotx_db::Db, context: &Context, coupon_id: &str) -> SyncResult<()> {
    let saved = match client::get_coupon(&context.credentials, coupon_id).await? {
        Some(coupon) => save_coupon(db, context, &coupon).await?,
        None => false,
    };
    if !saved {
        delete_coupon(db, context.row.workspace_id, coupon_id).await?;
    }
    Ok(())
}

async fn delete_coupon(db: &chatbotx_db::Db, ws: i64, coupon_id: &str) -> Result<()> {
    let id = coupon_id.to_string();
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_discount_by_external_id(conn, ws, SOURCE, &id)?;
        Ok(())
    })
    .await
}

/// `syncOrder`: the order, then its products (their stock moved with it).
async fn sync_order(deps: Deps<'_>, context: &Context, order_id: &str) -> SyncResult<()> {
    let Some(order) = client::get_order(&context.credentials, order_id).await? else {
        return Ok(());
    };
    // Only a store that sells through WooCommerce tells customers and fires
    // its triggers; one on another source just keeps the mirror.
    let announce = external_store::store_source(deps.db, context.row.workspace_id).await? == SOURCE;
    mirror_order(deps, &context.row, &order, announce).await?;
    let mut product_ids: Vec<String> = Vec::new();
    for line in order["line_items"].as_array().into_iter().flatten() {
        if let Some(id) = line["product_id"].as_i64().filter(|id| *id > 0) {
            let id = id.to_string();
            if !product_ids.contains(&id) {
                product_ids.push(id);
            }
        }
    }
    for product_id in product_ids.iter().take(STOCK_REFRESH_LIMIT) {
        if let Err(error) = sync_product(deps.db, context, product_id).await {
            tracing::warn!(%error, workspace_id = context.row.workspace_id, product_id, "[woocommerce] could not refresh a product's stock");
        }
    }
    Ok(())
}

/// `processWooCommerceWebhook`.
pub async fn process_webhook(
    deps: Deps<'_>,
    workspace_id: i64,
    topic: &str,
    resource_id: &str,
) -> SyncResult<()> {
    let Some(context) = context(deps.db, workspace_id).await? else {
        return Ok(());
    };
    match topic {
        "product.created" | "product.updated" | "product.restored" => {
            sync_product(deps.db, &context, resource_id).await
        }
        "product.deleted" => delete_product(deps.db, &context, resource_id).await,
        "coupon.created" | "coupon.updated" | "coupon.restored" => {
            sync_coupon(deps.db, &context, resource_id).await
        }
        "coupon.deleted" => Ok(delete_coupon(deps.db, workspace_id, resource_id).await?),
        // A deleted order keeps its mirror: the customer was already told.
        t if t.starts_with("order.") && t != "order.deleted" => {
            sync_order(deps, &context, resource_id).await
        }
        _ => Ok(()),
    }
}

/// `mirrorOrder`: `None` for an order not mirrored (yet): a draft, or a Web
/// Checkout order still waiting for payment.
pub async fn mirror_order(
    deps: Deps<'_>,
    row: &IntegrationWoocommerce,
    order: &Value,
    announce: bool,
) -> Result<Option<(chatbotx_db::repo::models::Order, bool)>> {
    if !mappers::is_mirrorable(order) {
        return Ok(None);
    }
    let ws = row.workspace_id;
    let order_id = match &order["id"] {
        Value::Number(n) => n.to_string(),
        other => text(other).to_string(),
    };
    let external_id = mappers::order_external_id(&row.site_url, &order_id);
    if mappers::order_meta(order, CHECKOUT_META) == Some(WEB_CHECKOUT)
        && NOT_YET_PLACED.contains(&text(&order["status"]))
    {
        let id = external_id.clone();
        if tx::read(deps.db, move |conn| find_mirrored(conn, ws, &id))
            .await?
            .is_none()
        {
            return Ok(None);
        }
    }
    let state = mappers::order_state(order);
    let contact = resolve_external_order_contact(
        deps.db,
        ws,
        mappers::order_meta(order, EXTERNAL_CONTACT_ATTRIBUTE),
        &mappers::order_contact_identifiers(order),
    )
    .await;
    let currency = Some(text(&order["currency"]).to_uppercase())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| row.currency.clone());
    let items = map_order_items(deps.db, ws, order, &currency).await?;
    let money = |v: f64| round_money(v, &currency);
    let lines = order["line_items"].as_array().cloned().unwrap_or_default();
    let subtotal = money(lines.iter().map(|l| numeric(&l["subtotal"])).sum());
    let shipping_address = mappers::address(&order["shipping"]);
    let optional = |v: &Value| Some(text(v)).filter(|t| !t.is_empty()).map(str::to_string);
    let number = optional(&order["number"]).unwrap_or_else(|| order_id.clone());
    let input = MirrorOrder {
        external_id,
        external_name: Some(format!("#{number}")),
        status: state.status.into(),
        contact: contact.clone(),
        payment_method: "cod".into(),
        payment_status: state.payment_status.into(),
        paid_at: mappers::gmt_date(&order["date_paid_gmt"]),
        fulfillment_status: Some(state.fulfillment_status.into()),
        tracking_company: state.tracking_company,
        tracking_number: state.tracking_number,
        tracking_url: state.tracking_url,
        currency: currency.clone(),
        subtotal,
        discount_total: money(numeric(&order["discount_total"])),
        shipping_total: money(numeric(&order["shipping_total"])),
        tax_total: money(numeric(&order["total_tax"])),
        total: money(numeric(&order["total"])),
        discount_code: optional(&order["coupon_lines"][0]["code"]),
        customer_name: mappers::customer_name(order),
        customer_email: optional(&order["billing"]["email"]),
        customer_phone: optional(&order["billing"]["phone"])
            .or_else(|| optional(&order["shipping"]["phone"])),
        shipping_type: shipping_address.is_some().then(|| "delivery".into()),
        shipping_address,
        note: optional(&order["customer_note"]),
        cancel_reason: None,
        created_at: mappers::gmt_date(&order["date_created_gmt"]),
    };
    let (saved, created) =
        external_store::sync_external(deps, ws, SOURCE, input, items.clone(), announce).await?;
    if created && let Some(contact) = contact.filter(|c| c.from_shop_link) {
        remove_ordered_from_cart(deps.db, ws, &contact, &items).await;
    }
    Ok(Some((saved, created)))
}

async fn sync_catalog(db: &chatbotx_db::Db, context: &Context) -> SyncResult<()> {
    let mut keep = Vec::new();
    for product in client::list_products(&context.credentials).await? {
        if !mappers::is_orderable(&product) {
            continue;
        }
        save_product(db, context, &product).await?;
        if let Some(id) = product["id"].as_i64() {
            keep.push(id.to_string());
        }
    }
    // Only after every page was read, so a failed page deletes nothing.
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_products_not_in(conn, ws, SOURCE, &keep)?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn sync_coupons(db: &chatbotx_db::Db, context: &Context) -> SyncResult<()> {
    let mut keep = Vec::new();
    for coupon in client::list_coupons(&context.credentials).await? {
        if save_coupon(db, context, &coupon).await?
            && let Some(id) = coupon["id"].as_i64()
        {
            keep.push(id.to_string());
        }
    }
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_discounts_not_in(conn, ws, SOURCE, &keep)?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn sync_orders(deps: Deps<'_>, context: &Context) -> SyncResult<i64> {
    let since = chrono::Utc::now() - chrono::Duration::days(client::ORDER_SYNC_DAYS);
    let since = since.format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut count = 0;
    for order in client::list_orders(&context.credentials, &since).await? {
        match mirror_order(deps, &context.row, &order, false).await {
            Ok(Some(_)) => count += 1,
            Ok(None) => {}
            // One bad order must not stop the rest of the history.
            Err(error) => tracing::warn!(
                %error,
                workspace_id = context.row.workspace_id,
                order_id = order["id"].as_i64(),
                "[woocommerce] could not mirror an order"
            ),
        }
    }
    Ok(count)
}

/// `saveProduct`: a variable product with its variations.
async fn save_product(db: &chatbotx_db::Db, context: &Context, product: &Value) -> SyncResult<()> {
    let variations = match (product["type"].as_str(), product["id"].as_i64()) {
        (Some("variable"), Some(id)) => client::list_variations(&context.credentials, id).await?,
        _ => Vec::new(),
    };
    let mapped = mappers::map_product(product, &variations, &context.row.currency);
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        save_mapped_product(conn, ws, SOURCE, mapped)
    })
    .await?;
    Ok(())
}

/// `saveCoupon`: false when the coupon cannot be priced in our cart.
async fn save_coupon(db: &chatbotx_db::Db, context: &Context, coupon: &Value) -> Result<bool> {
    let Some(mapped) = mappers::map_coupon(coupon) else {
        return Ok(false);
    };
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        save_mapped_discount(conn, ws, SOURCE, mapped)
    })
    .await
}

fn same_text(left: &Value, right: &str) -> bool {
    left.as_str()
        .is_some_and(|l| l.trim().to_lowercase() == right.trim().to_lowercase())
}

/// `matchLineVariant`: the variation's own variant, or — for "Any …"
/// attributes — the one whose picked values the line's meta shows.
fn match_line_variant<'a>(
    variants: &'a [ExternalVariantRow],
    variation_id: &str,
    meta: &[Value],
) -> Option<&'a ExternalVariantRow> {
    let candidates: Vec<&ExternalVariantRow> = variants
        .iter()
        .filter(|v| {
            v.external_id
                .as_deref()
                .is_some_and(|id| mappers::variation_id_of(id) == variation_id)
        })
        .collect();
    if let Some(exact) = candidates
        .iter()
        .find(|v| v.external_id.as_deref() == Some(variation_id))
    {
        return Some(exact);
    }
    candidates.into_iter().find(|v| {
        mappers::any_attributes_of(v.external_id.as_deref().unwrap_or_default())
            .iter()
            .all(|(name, value)| {
                meta.iter().any(|entry| {
                    (same_text(&entry["display_key"], name) || same_text(&entry["key"], name))
                        && (same_text(&entry["display_value"], value)
                            || same_text(&entry["value"], value))
                })
            })
    })
}

/// The `Size: M, Color: Red` label of a line's visible meta.
fn variant_meta(meta: &[Value]) -> String {
    meta.iter()
        .filter(|m| !text(&m["key"]).starts_with('_'))
        .filter_map(|m| {
            let value = if m["display_value"].is_null() {
                &m["value"]
            } else {
                &m["display_value"]
            };
            let key = if m["display_key"].is_null() {
                &m["key"]
            } else {
                &m["display_key"]
            };
            value.as_str().map(|v| format!("{}: {v}", text(key)))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn map_order_items(
    db: &chatbotx_db::Db,
    ws: i64,
    order: &Value,
    currency: &str,
) -> Result<Vec<MirrorItem>> {
    let lines = order["line_items"].as_array().cloned().unwrap_or_default();
    let positive = |v: &Value| v.as_i64().filter(|id| *id > 0).map(|id| id.to_string());
    let variation_ids: Vec<String> = lines
        .iter()
        .filter_map(|l| positive(&l["variation_id"]))
        .collect();
    let product_ids: Vec<String> = lines
        .iter()
        .filter_map(|l| positive(&l["product_id"]))
        .collect();
    let (variants, products) = tx::read(db, move |conn| {
        Ok((
            store_repo::find_variants_for_variations(conn, ws, SOURCE, &variation_ids)?,
            store_repo::find_products_by_external_ids(conn, ws, SOURCE, &product_ids)?,
        ))
    })
    .await?;
    Ok(lines
        .iter()
        .map(|line| {
            let meta = line["meta_data"].as_array().map_or(&[][..], Vec::as_slice);
            let variant = positive(&line["variation_id"])
                .and_then(|id| match_line_variant(&variants, &id, meta));
            let product = positive(&line["product_id"]).and_then(|id| {
                products
                    .iter()
                    .find(|p| p.external_id.as_deref() == Some(id.as_str()))
            });
            let quantity = line["quantity"].as_i64().unwrap_or(0);
            let line_subtotal = numeric(&line["subtotal"]);
            let unit_price = round_money(line_subtotal / quantity.max(1) as f64, currency);
            let gross = round_money(line_subtotal, currency);
            let line_discount =
                round_money((line_subtotal - numeric(&line["total"])).max(0.0), currency);
            let combination = variant
                .map(|v| v.combination.0.clone())
                .filter(|c| !c.is_null());
            let label = variant_meta(meta);
            MirrorItem {
                product_id: variant.map(|v| v.product_id).or(product.map(|p| p.id)),
                variant_id: variant.map(|v| v.variant_id),
                vendor_id: variant
                    .and_then(|v| v.vendor_id)
                    .or(product.and_then(|p| p.vendor_id)),
                category_id: variant
                    .and_then(|v| v.category_id)
                    .or(product.and_then(|p| p.category_id)),
                name: text(&line["name"]).to_string(),
                variant_label: variant_label_of(combination.as_ref())
                    .or_else(|| (!label.is_empty()).then_some(label)),
                combination,
                sku: Some(text(&line["sku"]).to_string()).filter(|s| !s.is_empty()),
                image_url: Some(text(&line["image"]["src"]).to_string()).filter(|s| !s.is_empty()),
                unit_price,
                quantity,
                discount_percent: if gross > 0.0 {
                    round_money(line_discount / gross * 100.0, "USD")
                } else {
                    0.0
                },
                discount_total: line_discount,
                tax_total: 0.0,
                line_total: round_money(gross - line_discount, currency),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn variant(id: &str) -> ExternalVariantRow {
        ExternalVariantRow {
            external_id: Some(id.into()),
            variant_id: 1,
            combination: chatbotx_db::repo::Json(Value::Null),
            product_id: 1,
            product_external_id: None,
            category_id: None,
            vendor_id: None,
        }
    }

    #[test]
    fn matches_any_variants_by_meta() {
        let variants = [variant("55~Size=M"), variant("55~Size=L"), variant("56")];
        let meta = [
            json!({ "key": "pa_size", "display_key": "Size", "value": "l", "display_value": "L" }),
        ];
        assert_eq!(
            match_line_variant(&variants, "55", &meta).and_then(|v| v.external_id.as_deref()),
            Some("55~Size=L")
        );
        assert_eq!(
            match_line_variant(&variants, "56", &[]).and_then(|v| v.external_id.as_deref()),
            Some("56")
        );
        assert_eq!(variant_meta(&meta), "Size: L");
    }
}
