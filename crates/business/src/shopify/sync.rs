//! `packages/business/src/shopify/{sync.service,webhooks}.ts`: the worker
//! side. A full sync mirrors the catalog, the code discounts and the last 60
//! days of orders; a webhook re-reads its one object from Shopify.

use chatbotx_core::utils::money::round_money;
use chatbotx_db::repo::external_store::{self as store_repo, UpsertProductInput};
use chatbotx_db::repo::models::IntegrationShopify;
use chatbotx_db::repo::{product_category, store_integration, vendor};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::Value;

use super::client::{self, ApiError, Credentials};
use super::mappers::{self, money, s};
use super::{SOURCE, mark_invalid, resolve_credentials};
use crate::errors::{BusinessError, Result};
use crate::external_store::{
    self, EXTERNAL_CONTACT_ATTRIBUTE, MirrorItem, MirrorOrder, remove_ordered_from_cart,
    resolve_external_order_contact, sync_error_message, variant_label_of,
};
use crate::order::Deps;
use crate::tx;

/// A failed step: Shopify's refusal, or ours.
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
    row: IntegrationShopify,
    credentials: Credentials,
}

async fn find(db: &chatbotx_db::Db, workspace_id: i64) -> Result<Option<IntegrationShopify>> {
    tx::read(db, move |conn| {
        Ok(store_integration::find_shopify(conn, workspace_id)?)
    })
    .await
}

async fn update(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    values: Vec<(&'static str, Sql)>,
) -> Result<()> {
    external_store::update_connection(db, SOURCE, workspace_id, values).await
}

/// `shopifySyncService.run`: the worker's full sync.
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
    match run_steps(deps, row, scope).await {
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

/// The steps of a full sync: the mirrored order count when orders ran.
async fn run_steps(
    deps: Deps<'_>,
    row: IntegrationShopify,
    scope: &str,
) -> SyncResult<Option<i64>> {
    let credentials = resolve_credentials(deps.db, &row).await?;
    let row = refresh_store_currency(deps.db, row, &credentials).await?;
    let context = Context { row, credentials };
    if scope != "orders" {
        sync_catalog(deps.db, &context).await?;
        sync_discounts(deps.db, &context).await?;
    }
    if scope != "products" {
        return Ok(Some(sync_orders(deps, &context).await?));
    }
    Ok(None)
}

/// `refreshStoreCurrency`: products are priced in the store's currency.
async fn refresh_store_currency(
    db: &chatbotx_db::Db,
    row: IntegrationShopify,
    credentials: &Credentials,
) -> SyncResult<IntegrationShopify> {
    let shop = client::get_shop(credentials).await?;
    let currency = s(&shop["currencyCode"]).unwrap_or_default().to_uppercase();
    let name = s(&shop["name"]).unwrap_or_default().to_string();
    let mut current = row;
    if currency != current.currency || Some(&name) != current.shop_name.as_ref() {
        update(
            db,
            current.workspace_id,
            vec![
                ("currency", Sql::Text(currency.clone())),
                ("shopName", Sql::Text(name.clone())),
            ],
        )
        .await?;
        current.currency.clone_from(&currency);
        current.shop_name = Some(name);
    }
    external_store::adopt_external_store_currency(
        db,
        current.workspace_id,
        SOURCE,
        &currency,
        None,
        None,
    )
    .await?;
    Ok(current)
}

/// `context`: an active connection with working credentials.
async fn context(db: &chatbotx_db::Db, workspace_id: i64) -> SyncResult<Option<Context>> {
    let Some(row) = find(db, workspace_id)
        .await?
        .filter(|r| r.status == "active")
    else {
        return Ok(None);
    };
    let credentials = resolve_credentials(db, &row).await?;
    Ok(Some(Context { row, credentials }))
}

async fn delete_product_row(db: &chatbotx_db::Db, workspace_id: i64, id: &str) -> Result<()> {
    let id = id.to_string();
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_product_by_external_id(conn, workspace_id, SOURCE, &id)?;
        Ok(())
    })
    .await
}

async fn delete_discount_row(db: &chatbotx_db::Db, workspace_id: i64, id: &str) -> Result<()> {
    let id = id.to_string();
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        store_repo::delete_discount_by_external_id(conn, workspace_id, SOURCE, &id)?;
        Ok(())
    })
    .await
}

async fn sync_product(db: &chatbotx_db::Db, workspace_id: i64, product_id: &str) -> SyncResult<()> {
    let Some(context) = context(db, workspace_id).await? else {
        return Ok(());
    };
    match client::get_product(&context.credentials, product_id).await? {
        Some(product) => save_product(db, &context, &product).await?,
        None => delete_product_row(db, workspace_id, product_id).await?,
    }
    Ok(())
}

async fn sync_discount(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    discount_id: &str,
) -> SyncResult<()> {
    let Some(context) = context(db, workspace_id).await? else {
        return Ok(());
    };
    let saved = match client::get_code_discount(&context.credentials, discount_id).await? {
        Some(node) => save_discount(db, &context, &node).await?,
        None => false,
    };
    if !saved {
        delete_discount_row(db, workspace_id, discount_id).await?;
    }
    Ok(())
}

async fn sync_order(deps: Deps<'_>, workspace_id: i64, order_id: &str) -> SyncResult<()> {
    let Some(context) = context(deps.db, workspace_id).await? else {
        return Ok(());
    };
    if let Some(order) = client::get_order(&context.credentials, order_id).await? {
        // Only a store that sells through Shopify tells customers and fires
        // its triggers; one still on its own catalog just keeps the mirror.
        let announce = external_store::store_source(deps.db, workspace_id).await? == SOURCE;
        mirror_order(deps, &context.row, &order, announce).await?;
    }
    Ok(())
}

/// `processShopifyWebhook`.
pub async fn process_webhook(
    deps: Deps<'_>,
    workspace_id: i64,
    topic: &str,
    object_id: Option<&str>,
) -> SyncResult<()> {
    if topic == "app/uninstalled" {
        mark_invalid(
            deps.db,
            workspace_id,
            "The Shopify app was uninstalled from the store.",
        )
        .await?;
        return Ok(());
    }
    let Some(id) = object_id else {
        return Ok(());
    };
    match topic {
        "products/create" | "products/update" => sync_product(deps.db, workspace_id, id).await,
        "products/delete" => Ok(delete_product_row(deps.db, workspace_id, id).await?),
        "inventory_levels/update" => {
            let Some(context) = context(deps.db, workspace_id).await? else {
                return Ok(());
            };
            match client::get_inventory_item_product_id(&context.credentials, id).await? {
                Some(product_id) => sync_product(deps.db, workspace_id, &product_id).await,
                None => Ok(()),
            }
        }
        "discounts/create" | "discounts/update" => sync_discount(deps.db, workspace_id, id).await,
        "discounts/delete" => Ok(delete_discount_row(deps.db, workspace_id, id).await?),
        t if t.starts_with("orders/") || t.starts_with("fulfillments/") => {
            sync_order(deps, workspace_id, id).await
        }
        _ => Ok(()),
    }
}

/// `mirrorOrder`: one Shopify order through `syncExternal`.
pub async fn mirror_order(
    deps: Deps<'_>,
    row: &IntegrationShopify,
    order: &Value,
    announce: bool,
) -> Result<(chatbotx_db::repo::models::Order, bool)> {
    let ws = row.workspace_id;
    let state = mappers::order_state(order);
    let reference = order["customAttributes"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["key"] == EXTERNAL_CONTACT_ATTRIBUTE))
        .and_then(|x| s(&x["value"]));
    let contact = resolve_external_order_contact(
        deps.db,
        ws,
        reference,
        &mappers::order_contact_identifiers(order),
    )
    .await;
    let currency = s(&order["currencyCode"]).unwrap_or_default().to_string();
    let items = map_order_items(deps.db, ws, order, &currency).await?;
    let set = |key: &str| round_money(money(&order[key]["shopMoney"]["amount"]), &currency);
    let customer = &order["customer"];
    let shipping = &order["shippingAddress"];
    let first = |values: [&Value; 3]| values.iter().find_map(|v| s(v)).map(str::to_string);
    let input = MirrorOrder {
        external_id: s(&order["id"]).unwrap_or_default().to_string(),
        external_name: s(&order["name"]).map(str::to_string),
        status: state.status.into(),
        contact: contact.clone(),
        payment_method: "cod".into(),
        payment_status: state.payment_status.into(),
        paid_at: (state.payment_status == "paid").then(chatbotx_db::repo::now_ms),
        fulfillment_status: Some(state.fulfillment_status.into()),
        tracking_company: state.tracking_company,
        tracking_number: state.tracking_number,
        tracking_url: state.tracking_url,
        currency: currency.clone(),
        subtotal: set("subtotalPriceSet"),
        discount_total: set("totalDiscountsSet"),
        shipping_total: set("totalShippingPriceSet"),
        tax_total: set("totalTaxSet"),
        total: set("totalPriceSet"),
        discount_code: s(&order["discountCodes"][0]).map(str::to_string),
        customer_name: first([&customer["displayName"], &shipping["name"], &Value::Null]),
        customer_email: first([&order["email"], &customer["email"], &Value::Null]),
        customer_phone: first([&order["phone"], &customer["phone"], &shipping["phone"]]),
        shipping_type: shipping.is_object().then(|| "delivery".into()),
        shipping_address: mappers::address(shipping),
        note: s(&order["note"]).map(str::to_string),
        cancel_reason: s(&order["cancelReason"]).map(str::to_string),
        created_at: s(&order["createdAt"])
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|d| d.timestamp_millis()),
    };
    let (saved, created) =
        external_store::sync_external(deps, ws, SOURCE, input, items.clone(), announce).await?;
    if created && let Some(contact) = contact.filter(|c| c.from_shop_link) {
        remove_ordered_from_cart(deps.db, ws, &contact, &items).await;
    }
    Ok((saved, created))
}

async fn sync_catalog(db: &chatbotx_db::Db, context: &Context) -> SyncResult<()> {
    let mut keep = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let (nodes, next) = client::list_products(&context.credentials, after.as_deref()).await?;
        for product in &nodes {
            save_product(db, context, product).await?;
            keep.extend(s(&product["id"]).map(str::to_string));
        }
        match next {
            Some(cursor) => after = Some(cursor),
            None => break,
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

async fn sync_discounts(db: &chatbotx_db::Db, context: &Context) -> SyncResult<()> {
    let mut keep = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let (nodes, next) =
            client::list_code_discounts(&context.credentials, after.as_deref()).await?;
        for node in &nodes {
            if save_discount(db, context, node).await? {
                keep.extend(s(&node["id"]).map(str::to_string));
            }
        }
        match next {
            Some(cursor) => after = Some(cursor),
            None => break,
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
    let since = since.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut count = 0;
    let mut after: Option<String> = None;
    loop {
        let (nodes, next) =
            client::list_orders(&context.credentials, after.as_deref(), &since).await?;
        for order in &nodes {
            match mirror_order(deps, &context.row, order, false).await {
                Ok(_) => count += 1,
                // One bad order must not stop the rest of the history.
                Err(error) => tracing::warn!(
                    %error,
                    workspace_id = context.row.workspace_id,
                    order_id = s(&order["id"]),
                    "[shopify] could not mirror an order"
                ),
            }
        }
        match next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(count),
        }
    }
}

/// `saveProduct`: the category is the product type, the vendor the vendor.
async fn save_product(db: &chatbotx_db::Db, context: &Context, product: &Value) -> Result<()> {
    let mapped = mappers::map_product(product, &context.row.shop_domain, &context.row.currency);
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        save_mapped_product(conn, ws, SOURCE, mapped)
    })
    .await
}

/// Writes a mapped product with its category and vendor (by name, created when missing).
pub(crate) fn save_mapped_product(
    conn: &Connection,
    ws: i64,
    source: &str,
    mapped: mappers::MappedProduct,
) -> Result<()> {
    let category_id = match &mapped.category_name {
        Some(name) => product_category::create_missing_by_name(
            conn,
            ws,
            std::slice::from_ref(name),
            Some(source),
        )?
        .first()
        .map(|c| c.id),
        None => None,
    };
    let vendor_id = match &mapped.vendor_name {
        Some(name) => {
            vendor::create_missing(conn, ws, &[name.as_str()], Some(source))?;
            vendor::find_by_names(conn, ws, &[name.as_str()], Some(source))?
                .first()
                .map(|v| v.0)
        }
        None => None,
    };
    let mut values = mapped.values;
    values.push(("categoryId", external_store::int(category_id)));
    values.push(("vendorId", external_store::int(vendor_id)));
    store_repo::upsert_product(
        conn,
        &UpsertProductInput {
            workspace_id: ws,
            source: source.to_string(),
            external_id: mapped.external_id,
            values,
            options: mapped.options,
            variants: mapped.variants,
        },
    )?;
    Ok(())
}

/// `saveDiscount`: false when Shopify's code cannot be priced in our cart.
async fn save_discount(db: &chatbotx_db::Db, context: &Context, node: &Value) -> Result<bool> {
    let Some(mapped) = mappers::map_discount(node) else {
        return Ok(false);
    };
    let ws = context.row.workspace_id;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        save_mapped_discount(conn, ws, SOURCE, mapped)
    })
    .await
}

/// Writes a mapped discount; false when none of its products is mirrored.
pub(crate) fn save_mapped_discount(
    conn: &Connection,
    ws: i64,
    source: &str,
    mapped: mappers::MappedDiscount,
) -> Result<bool> {
    let mut product_ids = Vec::new();
    if mapped.applies_to_products {
        product_ids = store_repo::find_products_by_external_ids(
            conn,
            ws,
            source,
            &mapped.product_external_ids,
        )?
        .into_iter()
        .map(|p| Value::String(p.id.to_string()))
        .collect();
        if product_ids.is_empty() {
            return Ok(false);
        }
    }
    let mut values = mapped.values;
    values.push((
        "productIds",
        Sql::Text(Value::Array(product_ids).to_string()),
    ));
    store_repo::upsert_discount(conn, ws, source, &mapped.external_id, &values)?;
    Ok(true)
}

async fn map_order_items(
    db: &chatbotx_db::Db,
    ws: i64,
    order: &Value,
    currency: &str,
) -> Result<Vec<MirrorItem>> {
    let lines = order["lineItems"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let variant_ids: Vec<String> = lines
        .iter()
        .filter_map(|l| s(&l["variant"]["id"]).map(str::to_string))
        .collect();
    let product_ids: Vec<String> = lines
        .iter()
        .filter_map(|l| s(&l["product"]["id"]).map(str::to_string))
        .collect();
    let (variants, products) = tx::read(db, move |conn| {
        Ok((
            store_repo::find_variants_by_external_ids(conn, ws, SOURCE, &variant_ids)?,
            store_repo::find_products_by_external_ids(conn, ws, SOURCE, &product_ids)?,
        ))
    })
    .await?;
    let currency = currency.to_string();
    Ok(lines
        .iter()
        .map(|line| {
            let variant = s(&line["variant"]["id"]).and_then(|id| {
                variants
                    .iter()
                    .find(|v| v.external_id.as_deref() == Some(id))
            });
            let product = s(&line["product"]["id"]).and_then(|id| {
                products
                    .iter()
                    .find(|p| p.external_id.as_deref() == Some(id))
            });
            let unit_price = round_money(
                money(&line["originalUnitPriceSet"]["shopMoney"]["amount"]),
                &currency,
            );
            let quantity = line["quantity"].as_i64().unwrap_or(0);
            let gross = unit_price * quantity as f64;
            let line_discount = money(&line["totalDiscountSet"]["shopMoney"]["amount"]);
            let combination = variant
                .map(|v| v.combination.0.clone())
                .filter(|c| !c.is_null());
            MirrorItem {
                product_id: variant.map(|v| v.product_id).or(product.map(|p| p.id)),
                variant_id: variant.map(|v| v.variant_id),
                vendor_id: variant
                    .and_then(|v| v.vendor_id)
                    .or(product.and_then(|p| p.vendor_id)),
                category_id: variant
                    .and_then(|v| v.category_id)
                    .or(product.and_then(|p| p.category_id)),
                name: s(&line["title"]).unwrap_or_default().to_string(),
                variant_label: variant_label_of(combination.as_ref())
                    .or_else(|| s(&line["variantTitle"]).map(str::to_string)),
                combination,
                sku: s(&line["sku"]).map(str::to_string),
                image_url: s(&line["image"]["url"]).map(str::to_string),
                unit_price,
                quantity,
                discount_percent: if gross > 0.0 {
                    round_money(line_discount / gross * 100.0, "USD")
                } else {
                    0.0
                },
                discount_total: round_money(line_discount, &currency),
                tax_total: 0.0,
                line_total: round_money(gross - line_discount, &currency),
            }
        })
        .collect())
}
