//! `webshop/service.ts`: the customer side of the store for the public shop
//! pages (`app/shop/[workspaceId]/*`) and their actions. Prices and stock
//! always come from `order::quote` / `order::place`, never from the cart or
//! the browser.
//!
//! An external store (Shopify / WooCommerce) checkout runs in
//! `external_checkout` (`checkoutOnExternalStore`).

use std::collections::HashMap;

use chatbotx_core::utils::money::stack_discount_percents;
use chatbotx_core::utils::web_url::to_safe_web_url;
use chatbotx_db::repo::models::{Cart, EcommerceOpenHours, ProductVariant};
use chatbotx_db::repo::order::OrderDetail;
use chatbotx_db::repo::product::{self as product_repo, ProductDetail, RelatedTo, ShopListInput};
use chatbotx_db::repo::{self, find_one};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::ecommerce_open_hours::is_within_open_hours;
use crate::ecommerce_open_hours::schedule::ranges_from_json;
use crate::errors::{BusinessError, Result};
use crate::order::{self, Actor, ContactRef, Deps, PlaceInput, PlaceItem, Quote};
use crate::store_source::{is_external_store_source, resolve_store_source};
use crate::{cart, ecommerce_settings, order_payment, tx};

pub mod collection;
mod external_checkout;
pub mod product_search;

/// Most distinct lines (product and variant pairs) one cart may hold.
pub const MAX_CART_LINES: usize = 100;
/// A preview cart is a cookie: kept well under the browser's 4 KB limit.
pub const MAX_PREVIEW_CART_LINES: usize = 30;
const RELATED_PRODUCTS_LIMIT: i64 = 10;
const MAX_LINE_QUANTITY: i64 = 10_000;

/// The customer a shop link was signed for.
#[derive(Debug, Clone, Copy)]
pub struct Shopper {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: Option<i64>,
    pub conversation_id: Option<i64>,
}

/// `add` adds to a line, `set` sets its quantity (0 removes it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Add,
    Set,
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

// ---- errors (`webshop/errors.ts`) -------------------------------------------

pub fn closed(reason: &str, vendor_name: Option<&str>) -> BusinessError {
    let message = if reason == "outsideOpenHours" {
        match vendor_name {
            Some(name) => format!("\"{name}\" is closed right now."),
            None => "The store is closed right now.".to_string(),
        }
    } else {
        "The store is not taking orders right now.".to_string()
    };
    let mut data = json!({ "reason": reason });
    if let Some(name) = vendor_name {
        data["vendorName"] = json!(name);
    }
    BusinessError::exception(message, "webshopClosed", 400).with_data(data)
}

pub fn preview_only() -> BusinessError {
    BusinessError::exception(
        "A preview link can browse the shop but not order.",
        "webshopPreviewOnly",
        403,
    )
}

fn rating_not_allowed() -> BusinessError {
    BusinessError::exception(
        "This order cannot be rated.",
        "webshopRatingNotAllowed",
        400,
    )
}

fn cart_full(maximum: usize) -> BusinessError {
    BusinessError::exception(
        format!("A cart holds at most {maximum} different items."),
        "webshopCartFull",
        400,
    )
    .with_data(json!({ "maximum": maximum }))
}

fn order_empty() -> BusinessError {
    BusinessError::exception("An order needs at least one item.", "orderEmpty", 400)
}

// ---- cart lines ---------------------------------------------------------------

fn same_line(a: &PlaceItem, b: &PlaceItem) -> bool {
    a.product_id == b.product_id && a.variant_id == b.variant_id
}

fn line_quantity(lines: &[PlaceItem], line: &PlaceItem) -> i64 {
    lines
        .iter()
        .find(|c| same_line(c, line))
        .map_or(0, |c| c.quantity)
}

/// `changeLine`: an existing line keeps its place; 0 or less removes it.
pub fn change_line(lines: &[PlaceItem], change: &PlaceItem, mode: Mode) -> Vec<PlaceItem> {
    let current = line_quantity(lines, change);
    let quantity = match mode {
        Mode::Add => current + change.quantity,
        Mode::Set => change.quantity,
    };
    if quantity <= 0 {
        return lines
            .iter()
            .filter(|l| !same_line(l, change))
            .cloned()
            .collect();
    }
    let capped = quantity.min(MAX_LINE_QUANTITY);
    if !lines.iter().any(|l| same_line(l, change)) {
        let mut next = lines.to_vec();
        next.push(PlaceItem {
            quantity: capped,
            ..change.clone()
        });
        return next;
    }
    lines
        .iter()
        .map(|l| {
            if same_line(l, change) {
                PlaceItem {
                    quantity: capped,
                    ..l.clone()
                }
            } else {
                l.clone()
            }
        })
        .collect()
}

/// `subtractOrderedLines`: `None` when the cart no longer holds all of
/// `ordered` (a second submit of the same checkout).
pub fn subtract_ordered_lines(cart: &[PlaceItem], ordered: &[PlaceItem]) -> Option<Vec<PlaceItem>> {
    let mut left = cart.to_vec();
    for line in ordered {
        let remaining = line_quantity(&left, line) - line.quantity;
        if remaining < 0 {
            return None;
        }
        left = left
            .into_iter()
            .filter_map(|c| {
                if !same_line(&c, line) {
                    Some(c)
                } else if remaining > 0 {
                    Some(PlaceItem {
                        quantity: remaining,
                        ..c
                    })
                } else {
                    None
                }
            })
            .collect();
    }
    Some(left)
}

/// The stored `items` (`[{productId, variantId, quantity}]`, ids as text).
pub fn lines_json(lines: &[PlaceItem]) -> Value {
    lines
        .iter()
        .map(|l| {
            json!({
                "productId": l.product_id.to_string(),
                "variantId": l.variant_id.map(|v| v.to_string()),
                "quantity": l.quantity,
            })
        })
        .collect()
}

/// The item count of the cart badge.
pub fn count_items(lines: &[PlaceItem]) -> i64 {
    lines.iter().map(|l| l.quantity).sum()
}

// ---- cart rows ------------------------------------------------------------------

fn find_cart(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<Option<Cart>> {
    Ok(find_one(
        conn,
        r#""workspaceId" = ?1 AND "contactId" = ?2"#,
        &[&workspace_id, &contact_id],
    )?)
}

fn cart_lines(cart: Option<&Cart>) -> Vec<PlaceItem> {
    cart.map_or_else(Vec::new, |c| cart::items_of(&c.items.0))
}

/// `cartRepository.upsert`: the lines, a fresh activity time, the reminder
/// re-armed; the channel is kept when none is given.
fn upsert_lines(conn: &Connection, shopper: &Shopper, lines: &[PlaceItem]) -> Result<()> {
    let now = repo::now_ms();
    conn.execute(
        r#"INSERT INTO "Cart" ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "contactInboxId", "items", "lastActivityAt", "abandonedNotifiedAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?2, NULL)
           ON CONFLICT ("workspaceId", "contactId") DO UPDATE SET
             "items" = excluded."items", "lastActivityAt" = excluded."lastActivityAt",
             "abandonedNotifiedAt" = NULL, "updatedAt" = excluded."updatedAt",
             "contactInboxId" = COALESCE(excluded."contactInboxId", "Cart"."contactInboxId")"#,
        params![
            repo::new_id(),
            now,
            shopper.workspace_id,
            shopper.contact_id,
            shopper.contact_inbox_id,
            lines_json(lines).to_string()
        ],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `cartRepository.setDiscountCode`.
fn set_code(conn: &Connection, shopper: &Shopper, code: Option<&str>) -> Result<()> {
    let now = repo::now_ms();
    conn.execute(
        r#"INSERT INTO "Cart" ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "contactInboxId", "items", "discountCode", "lastActivityAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, '[]', ?6, ?2)
           ON CONFLICT ("workspaceId", "contactId") DO UPDATE SET
             "discountCode" = excluded."discountCode", "updatedAt" = excluded."updatedAt""#,
        params![
            repo::new_id(),
            now,
            shopper.workspace_id,
            shopper.contact_id,
            shopper.contact_inbox_id,
            code
        ],
    )
    .map_err(repo_err)?;
    Ok(())
}

// ---- storefront -----------------------------------------------------------------

/// `visibleShopCategories`: visible ones, a sub-category only under a shown parent.
pub fn visible_shop_categories<T>(
    categories: Vec<T>,
    key: impl Fn(&T) -> (i64, Option<i64>, bool),
) -> Vec<T> {
    let visible: std::collections::HashSet<i64> = categories
        .iter()
        .map(&key)
        .filter(|(_, _, v)| *v)
        .map(|(id, _, _)| id)
        .collect();
    categories
        .into_iter()
        .filter(|c| {
            let (_, parent, shown) = key(c);
            shown && parent.is_none_or(|p| visible.contains(&p))
        })
        .collect()
}

struct WorkspaceRow {
    name: String,
    brand_color: String,
    language: String,
    timezone: String,
    scheduled_deletion_at: Option<i64>,
    owner_id: Option<i64>,
}

fn workspace_row(conn: &Connection, workspace_id: i64) -> Result<Option<WorkspaceRow>> {
    conn.query_row(
        r#"SELECT "name", "brandColor", "language", "timezone", "scheduledDeletionAt", "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
        [workspace_id],
        |r| {
            Ok(WorkspaceRow {
                name: r.get(0)?,
                brand_color: r.get(1)?,
                language: r.get(2)?,
                timezone: r.get(3)?,
                scheduled_deletion_at: r.get(4)?,
                owner_id: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(repo_err)
}

/// The `ownerBlocked` part of `resolveWorkspaceFreezeReasonById` (cloud
/// only): the owner's plan has lapsed or hit its contact limit.
// ponytail: a plan-less owner counts as on the tenant's default plan
// (active); that plan's own contact limit lives in KV and is not read here.
fn owner_blocked(conn: &Connection, owner_id: Option<i64>, now_ms: i64) -> Result<bool> {
    let Some(owner_id) = owner_id.filter(|_| crate::keys::Edition::from_env().is_cloud()) else {
        return Ok(false);
    };
    let quota =
        crate::user_quota::get_for_user(conn, owner_id)?.filter(|q| q.plan_status.is_some());
    Ok(crate::user_quota::access_state_from_quota(quota.as_ref(), now_ms).blocked)
}

/// `orderingState`: ordering on, the workspace not frozen, and inside the
/// store's open hours and those of every vendor `(name, openHoursId)`.
///
/// ponytail: the freeze check is the community one (missing or scheduled
/// for deletion), as `inbound_webhook::workspace_unavailable`.
pub fn ordering_state(
    conn: &Connection,
    workspace_id: i64,
    settings: &Value,
    vendors: &[(String, Option<i64>)],
    now_ms: i64,
) -> Result<Value> {
    let unavailable = json!({ "open": false, "reason": "orderingUnavailable" });
    if settings["general"]["orderingAvailable"] != true {
        return Ok(unavailable);
    }
    let Some(workspace) = workspace_row(conn, workspace_id)? else {
        return Ok(unavailable);
    };
    // A frozen workspace (owner blocked, deletion pending) takes no orders.
    if workspace.scheduled_deletion_at.is_some() || owner_blocked(conn, workspace.owner_id, now_ms)?
    {
        return Ok(unavailable);
    }
    let store_hours = settings["general"]["openHoursId"]
        .as_str()
        .and_then(|id| id.parse::<i64>().ok());
    let ids: Vec<i64> = store_hours
        .into_iter()
        .chain(vendors.iter().filter_map(|(_, id)| *id))
        .collect();
    if ids.is_empty() {
        return Ok(json!({ "open": true }));
    }
    let mut schedules: HashMap<i64, EcommerceOpenHours> = HashMap::new();
    for id in &ids {
        let row: Option<EcommerceOpenHours> = find_one(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[id, &workspace_id],
        )?;
        if let Some(row) = row {
            schedules.insert(row.id, row);
        }
    }
    // A schedule that was deleted leaves its users always open.
    let is_open = |id: Option<i64>| {
        id.and_then(|id| schedules.get(&id)).is_none_or(|s| {
            is_within_open_hours(
                &ranges_from_json(&s.ranges.0.to_string()),
                &workspace.timezone,
                now_ms,
            )
        })
    };
    if !is_open(store_hours) {
        return Ok(json!({ "open": false, "reason": "outsideOpenHours" }));
    }
    if let Some((name, _)) = vendors.iter().find(|(_, id)| !is_open(*id)) {
        return Ok(json!({ "open": false, "reason": "outsideOpenHours", "vendorName": name }));
    }
    Ok(json!({ "open": true }))
}

/// `getStorefront` + `toShopView` (`lib/load-shop.ts`): what every shop page
/// needs, safe to hand to the browser. `locale` is the workspace language
/// (the SPA resolves it to a supported locale).
pub async fn storefront(db: &chatbotx_db::Db, workspace_id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let settings = ecommerce_settings::get_in(conn, workspace_id)?;
        let workspace = workspace_row(conn, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Workspace not found"))?;
        let source = resolve_store_source(conn, workspace_id)?;
        let categories = visible_shop_categories(
            chatbotx_db::repo::product_category::list_with_product_count(
                conn,
                workspace_id,
                &source,
            )?,
            |c| (c.id, c.parent_id, c.is_visible),
        );
        let vendors = chatbotx_db::repo::vendor::list_with_product_count(
            conn,
            workspace_id,
            Some(&source),
            None,
            chatbotx_db::repo::utils::PaginationInput::default(),
        )?;
        let ordering = ordering_state(conn, workspace_id, &settings, &[], repo::now_ms())?;
        Ok(shop_view(
            &settings,
            &workspace,
            &ordering,
            categories
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id.to_string(),
                        "parentId": c.parent_id.map(|p| p.to_string()),
                        "name": c.name,
                        "imageUrl": c.image_url,
                    })
                })
                .collect(),
            vendors
                .data
                .iter()
                .filter(|v| v.vendor.is_active)
                .map(|v| {
                    json!({
                        "id": v.vendor.id.to_string(),
                        "name": v.vendor.name,
                        "shortDescription": v.vendor.short_description,
                        "imageUrl": v.vendor.image_url,
                    })
                })
                .collect(),
        ))
    })
    .await
}

fn shop_view(
    settings: &Value,
    workspace: &WorkspaceRow,
    ordering: &Value,
    categories: Vec<Value>,
    vendors: Vec<Value>,
) -> Value {
    let (general, payments, shipping, checkout) = (
        &settings["general"],
        &settings["payments"],
        &settings["shipping"],
        &settings["checkout"],
    );
    let store_source = general["storeSource"].as_str().unwrap_or("chatbotx");
    let external = is_external_store_source(store_source);
    let web_checkout = external && checkout["externalCheckoutMethod"] == "webCheckout";
    json!({
        "storeName": workspace.name,
        "brandColor": workspace.brand_color,
        "locale": workspace.language,
        "ordering": ordering,
        "money": {
            "currency": payments["currency"],
            "currencySymbol": payments["currencySymbol"],
            "currencyPosition": payments["currencyPosition"],
        },
        "productView": general["webshopProductView"],
        "productImageSize": general["productImageSize"],
        "productButtons": general["productButtons"],
        "allowRateOrders": general["allowRateOrders"],
        // An external store takes cash on delivery here and nothing else.
        "paymentMethods": if external { json!(["cod"]) } else { payments["paymentMethods"].clone() },
        "facebookPixelId": general["facebookPixelId"],
        "checkout": {
            "mode": checkout["checkoutMode"],
            "externalLink": to_safe_web_url(checkout["checkoutLink"].as_str()),
            "minimumOrderValue": checkout["minimumOrderValue"],
            "requireName": checkout["requireName"],
            "requireEmail": checkout["requireEmail"],
            "requirePhone": checkout["requirePhone"],
            "requireState": checkout["requireState"],
            "requireZip": checkout["requireZip"],
            "externalWebCheckout": web_checkout,
            "shopifyWebCheckout": web_checkout && store_source == "shopify",
            "showDiscountCodes": !external || checkout["externalAutoApplyDiscounts"] == true,
        },
        "shipping": {
            "mode": shipping["shippingType"],
            "storeAddress": shipping["storeAddress"],
            "countries": shipping["shippingCountries"],
            "defaultCountry": shipping["defaultShippingCountry"],
            "deliveryCost": shipping["deliveryCost"],
        },
        "categories": categories,
        "vendors": vendors,
    })
}

// ---- products ---------------------------------------------------------------------

fn is_product_in_stock(p: &chatbotx_db::repo::models::Product) -> bool {
    p.inventory_policy != "track" || p.inventory_quantity > 0 || p.allow_out_of_stock_purchase
}

fn in_stock(p: &chatbotx_db::repo::models::Product, quantities: &[Option<i64>]) -> bool {
    if quantities.iter().all(Option::is_none) {
        return is_product_in_stock(p);
    }
    quantities.iter().any(|q| match q {
        None => is_product_in_stock(p),
        Some(q) => *q > 0 || p.allow_out_of_stock_purchase,
    })
}

/// `toShopProduct`.
fn shop_product_json(
    p: &chatbotx_db::repo::models::Product,
    vendor: Option<&str>,
    vendor_discount: i64,
    has_variants: bool,
    quantities: &[Option<i64>],
) -> Value {
    let images: Vec<Value> = p
        .images
        .0
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["url"].as_str().map(Value::from))
        .collect();
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "shortDescription": p.short_description,
        "price": p.price,
        "discount": stack_discount_percents(p.discount, order::major(vendor_discount)),
        "imageUrl": images.first(),
        "images": images,
        "vendorName": vendor,
        "categoryId": p.category_id.map(|c| c.to_string()),
        "productUrl": to_safe_web_url(p.product_url.as_deref()),
        "inStock": in_stock(p, quantities),
        "hasVariants": has_variants,
    })
}

fn row_json(row: &product_repo::ShopProduct) -> Value {
    shop_product_json(
        &row.product,
        row.vendor.as_deref(),
        row.vendor_discount_percent,
        row.has_variants,
        &row.variant_inventory_quantities,
    )
}

/// `listProducts`: one page of what the shop shows.
pub fn list_products(conn: &Connection, mut input: ShopListInput) -> Result<Value> {
    input.source = Some(resolve_store_source(conn, input.workspace_id)?);
    let page = product_repo::list_for_shop(conn, &input)?;
    Ok(json!({
        "data": page.data.iter().map(row_json).collect::<Vec<_>>(),
        "pageCount": page.page_count,
    }))
}

/// `getProduct`: a product the shop shows, with its enabled variants.
fn shop_product(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
) -> Result<Option<(ProductDetail, i64, Vec<ProductVariant>)>> {
    let listed = product_repo::list_for_shop(
        conn,
        &ShopListInput {
            workspace_id,
            source: Some(resolve_store_source(conn, workspace_id)?),
            product_ids: Some(vec![product_id]),
            ..ShopListInput::default()
        },
    )?;
    let Some(row) = listed.data.first() else {
        return Ok(None);
    };
    let Some(detail) = product_repo::find_detail(conn, product_id, workspace_id)? else {
        return Ok(None);
    };
    let enabled = detail
        .variants
        .iter()
        .filter(|v| v.is_enabled)
        .cloned()
        .collect();
    Ok(Some((detail, row.vendor_discount_percent, enabled)))
}

/// The product page: `toShopProductDetail` and its related products, or
/// `null` for a product the shop does not show.
pub fn product_page(conn: &Connection, workspace_id: i64, product_id: i64) -> Result<Value> {
    let Some((detail, vendor_discount, variants)) = shop_product(conn, workspace_id, product_id)?
    else {
        return Ok(Value::Null);
    };
    let p = &detail.product;
    let quantities: Vec<Option<i64>> = variants.iter().map(|v| v.inventory_quantity).collect();
    let mut view = shop_product_json(
        p,
        detail.vendor.as_deref(),
        vendor_discount,
        !variants.is_empty(),
        &quantities,
    );
    let mut options = detail.variant_options.clone();
    options.sort_by_key(|o| o.position);
    view["sku"] = json!(p.sku);
    view["allowOutOfStockPurchase"] = json!(p.allow_out_of_stock_purchase);
    view["longDescription"] = json!(p.long_description);
    view["variantOptions"] = options
        .iter()
        .map(|o| json!({ "name": o.name, "values": o.values.0 }))
        .collect();
    view["variants"] = variants
        .iter()
        .map(|v| {
            json!({
                "id": v.id.to_string(),
                "combination": v.combination.0,
                "price": v.price,
                "sku": v.sku,
                "inventoryQuantity": v.inventory_quantity,
            })
        })
        .collect();
    let tags = p
        .tags
        .0
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect();
    let related = product_repo::list_related_for_shop(
        conn,
        workspace_id,
        Some(&resolve_store_source(conn, workspace_id)?),
        &RelatedTo {
            id: p.id,
            category_id: p.category_id,
            subcategory_id: p.subcategory_id,
            tags,
        },
        RELATED_PRODUCTS_LIMIT,
    )?;
    Ok(json!({
        "product": view,
        "related": related.iter().map(row_json).collect::<Vec<_>>(),
    }))
}

// ---- cart -------------------------------------------------------------------------

/// `countCartItems`.
pub fn count_cart_items(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<i64> {
    Ok(count_items(&cart_lines(
        find_cart(conn, workspace_id, contact_id)?.as_ref(),
    )))
}

/// `assertCartLineCanGrow`: only what the shop shows can go in the cart (or
/// grow in it), with a variant it offers; lowering a line is always allowed.
fn assert_can_grow(
    conn: &Connection,
    workspace_id: i64,
    change: &PlaceItem,
    before: &[PlaceItem],
    mode: Mode,
) -> Result<()> {
    if line_quantity(&change_line(before, change, mode), change) <= line_quantity(before, change) {
        return Ok(());
    }
    let (detail, _, enabled) = shop_product(conn, workspace_id, change.product_id)?
        .ok_or_else(|| BusinessError::not_found("Product not found"))?;
    let name = &detail.product.name;
    match change.variant_id {
        None if !enabled.is_empty() => Err(order::product_unavailable(format!(
            "Choose a variant of \"{name}\"."
        ))),
        Some(id) if !enabled.iter().any(|v| v.id == id) => Err(order::product_unavailable(
            format!("The chosen variant of \"{name}\" is not available."),
        )),
        _ => Ok(()),
    }
}

/// `addToCart` / `setCartQuantity`: the item count after the change.
pub async fn change_cart(
    deps: Deps<'_>,
    shopper: Shopper,
    change: PlaceItem,
    mode: Mode,
) -> Result<i64> {
    let ws = shopper.workspace_id;
    let check = change.clone();
    tx::read(deps.db, move |conn| {
        let before = cart_lines(find_cart(conn, ws, shopper.contact_id)?.as_ref());
        assert_can_grow(conn, ws, &check, &before, mode)
    })
    .await?;
    let line = change.clone();
    // One writer: two quick taps apply one after the other.
    let (before, after) = tx::write(deps.db, move |tx| {
        let conn: &Connection = tx;
        let before = cart_lines(find_cart(conn, ws, shopper.contact_id)?.as_ref());
        let after = change_line(&before, &line, mode);
        if after.len() > before.len().max(MAX_CART_LINES) {
            return Err(cart_full(MAX_CART_LINES));
        }
        upsert_lines(conn, &shopper, &after)?;
        Ok((before, after))
    })
    .await?;
    announce_change(deps, &shopper, &change, &before, &after).await;
    Ok(count_items(&after))
}

/// `productAddedToCart` / `productRemovedFromCart`; best effort.
async fn announce_change(
    deps: Deps<'_>,
    shopper: &Shopper,
    change: &PlaceItem,
    before: &[PlaceItem],
    after: &[PlaceItem],
) {
    let quantity = line_quantity(after, change);
    let delta = quantity - line_quantity(before, change);
    if delta == 0 {
        return;
    }
    let (ws, product_id) = (shopper.workspace_id, change.product_id);
    let product = tx::read(deps.db, move |conn| {
        Ok(product_repo::find_for_order(conn, ws, &[product_id], None)?
            .into_iter()
            .next())
    })
    .await;
    let product = match product {
        Ok(product) => product,
        Err(error) => {
            tracing::warn!(%error, product_id, "[webshop] could not announce the cart change");
            return;
        }
    };
    let categories: Vec<String> = product
        .as_ref()
        .map(|p| [p.product.category_id, p.product.subcategory_id])
        .into_iter()
        .flatten()
        .flatten()
        .map(|id| id.to_string())
        .collect();
    let mut metadata = Map::new();
    metadata.insert("channel".into(), json!("webshop"));
    metadata.insert(
        "product".into(),
        json!({
            "productId": product_id.to_string(),
            "variantId": change.variant_id.map(|v| v.to_string()),
            "name": product.as_ref().map_or("", |p| p.product.name.as_str()),
            "categoryIds": categories,
        }),
    );
    metadata.insert("quantity".into(), json!(quantity));
    metadata.insert("change".into(), json!(delta));
    let inbox = shopper.contact_inbox_id.map(|i| i.to_string());
    crate::events::trigger::emit_cart_item_changed(
        deps.events,
        &ws.to_string(),
        &shopper.contact_id.to_string(),
        &metadata,
        inbox.as_deref(),
    )
    .await;
}

/// `changePreviewCart`: checked like a customer's, saved nowhere.
pub async fn change_preview_cart(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    items: Vec<PlaceItem>,
    change: PlaceItem,
    mode: Mode,
) -> Result<Vec<PlaceItem>> {
    tx::read(db, move |conn| {
        assert_can_grow(conn, workspace_id, &change, &items, mode)?;
        let after = change_line(&items, &change, mode);
        if after.len() > items.len().max(MAX_PREVIEW_CART_LINES) {
            return Err(cart_full(MAX_PREVIEW_CART_LINES));
        }
        Ok(after)
    })
    .await
}

fn discount_error_json(quote: &Quote) -> Value {
    quote
        .discount_error
        .as_ref()
        .map_or(Value::Null, |(code, data)| match data {
            Some(data) => json!({ "code": code, "data": data }),
            None => json!({ "code": code }),
        })
}

/// `applyDiscountCode` (customer, `shopper`) or `quotePreviewDiscountCode`
/// (preview, `items`): the reason a code cannot be used, or `null` (the
/// customer's cart keeps it).
pub async fn apply_discount_code(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    shopper: Option<Shopper>,
    preview_items: Vec<PlaceItem>,
    code: &str,
    shipping_type: Option<String>,
) -> Result<Value> {
    let code = code.trim().to_string();
    let items = match shopper {
        Some(s) => {
            tx::read(db, move |conn| {
                Ok(cart_lines(
                    find_cart(conn, workspace_id, s.contact_id)?.as_ref(),
                ))
            })
            .await?
        }
        None => preview_items,
    };
    let quote = order::quote(
        db,
        workspace_id,
        items,
        shipping_type,
        shopper.map(|s| s.contact_id),
        Some(code.clone()),
        true,
    )
    .await?;
    let error = discount_error_json(&quote);
    if let (Some(s), true) = (shopper, error.is_null()) {
        tx::write(db, move |tx| set_code(tx, &s, Some(&code))).await?;
    }
    Ok(error)
}

/// `removeDiscountCode`.
pub async fn remove_discount_code(db: &chatbotx_db::Db, shopper: Shopper) -> Result<()> {
    let shopper = Shopper {
        contact_inbox_id: None,
        ..shopper
    };
    tx::write(db, move |tx| set_code(tx, &shopper, None)).await
}

/// The cart page's cart: the customer's (`getCart`) or a preview's
/// (`getPreviewCart`), priced the same way.
pub async fn cart_page(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    shopper: Option<Shopper>,
    preview: (Vec<PlaceItem>, Option<String>),
    shipping_type: Option<String>,
) -> Result<Value> {
    match shopper {
        Some(s) => cart::get_cart(db, workspace_id, s.contact_id, shipping_type, true).await,
        None => {
            cart::price(
                db,
                workspace_id,
                lines_json(&preview.0),
                preview.1,
                None,
                shipping_type,
                true,
            )
            .await
        }
    }
}

// ---- checkout ---------------------------------------------------------------------

/// `WebshopCheckoutForm`.
#[derive(Debug, Clone, Default)]
pub struct CheckoutForm {
    pub payment_method: Option<String>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub customer_phone: Option<String>,
    pub shipping_type: Option<String>,
    pub shipping_address: Option<Value>,
    pub note: Option<String>,
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.trim().is_empty())
}

fn trimmed(value: Option<&String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// `assertCheckoutForm`: the buyer fields the store requires, and a country
/// it ships to.
pub fn assert_checkout_form(settings: &Value, form: &CheckoutForm) -> Result<()> {
    let checkout = &settings["checkout"];
    for (setting, field, value) in [
        ("requireName", "customerName", &form.customer_name),
        ("requireEmail", "customerEmail", &form.customer_email),
        ("requirePhone", "customerPhone", &form.customer_phone),
    ] {
        if checkout[setting] == true && is_blank(value.as_deref()) {
            return Err(BusinessError::validation(
                field,
                format!("{field} is required."),
            ));
        }
    }
    let mode = settings["shipping"]["shippingType"].as_str().unwrap_or("");
    let delivering = form.shipping_type.as_deref().unwrap_or("delivery") == "delivery"
        && mode != "none"
        && mode != "pickup";
    if !delivering {
        return Ok(());
    }
    let address = form.shipping_address.as_ref().unwrap_or(&Value::Null);
    let text = |key: &str| address[key].as_str();
    if is_blank(text("line1")) {
        return Err(BusinessError::validation(
            "shippingAddress.line1",
            "Address is required.",
        ));
    }
    if checkout["requireState"] == true && is_blank(text("state")) {
        return Err(BusinessError::validation(
            "shippingAddress.state",
            "State is required.",
        ));
    }
    if checkout["requireZip"] == true && is_blank(text("zip")) {
        return Err(BusinessError::validation(
            "shippingAddress.zip",
            "ZIP code is required.",
        ));
    }
    let countries: Vec<&str> = settings["shipping"]["shippingCountries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let country = text("country").map(str::to_uppercase);
    if !countries.is_empty() && !country.is_some_and(|c| countries.contains(&c.as_str())) {
        return Err(BusinessError::validation(
            "shippingAddress.country",
            "The store does not ship to this country.",
        ));
    }
    Ok(())
}

/// `checkout`: places the cart as an order for the shopper. The cart loses
/// what was ordered in the order's transaction, so a refused order keeps it.
/// Paid online, the provider's page is opened (`paymentUrl`); a page that
/// cannot be opened cancels the order and puts its items back in the cart.
pub async fn checkout(deps: Deps<'_>, shopper: Shopper, form: CheckoutForm) -> Result<Value> {
    let ws = shopper.workspace_id;
    let (settings, lines, code) = tx::read(deps.db, move |conn| {
        let settings = ecommerce_settings::get_in(conn, ws)?;
        let source = settings["general"]["storeSource"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let external = is_external_store_source(&source);
        if !external && settings["checkout"]["checkoutMode"] == "externalLink" {
            return Err(closed("orderingUnavailable", None));
        }
        let cart = find_cart(conn, ws, shopper.contact_id)?;
        let items = cart_lines(cart.as_ref());
        let products = order::find_products(
            conn,
            ws,
            &items.iter().map(|i| i.product_id).collect::<Vec<_>>(),
        )?;
        let (lines, unavailable) = order::split_orderable(&products, &items);
        if lines.is_empty() {
            return Err(if unavailable.is_empty() {
                order_empty()
            } else {
                order::product_unavailable("None of the items in the cart can be ordered.")
            });
        }
        let vendors: Vec<(String, Option<i64>)> = lines
            .iter()
            .filter_map(|l| products.get(&l.product_id)?.vendor.as_ref())
            .map(|(name, _, _, _, hours)| (name.clone(), *hours))
            .collect();
        let ordering = ordering_state(conn, ws, &settings, &vendors, repo::now_ms())?;
        if ordering["open"] != true {
            return Err(closed(
                ordering["reason"].as_str().unwrap_or("orderingUnavailable"),
                ordering["vendorName"].as_str(),
            ));
        }
        assert_checkout_form(&settings, &form)?;
        let source = external.then_some(source);
        Ok((
            settings,
            lines,
            (cart.and_then(|c| c.discount_code), form, source),
        ))
    })
    .await?;
    let (discount_code, form, source) = code;
    if let Some(source) = source {
        return external_checkout::checkout(
            deps,
            shopper,
            &source,
            &settings,
            &lines,
            discount_code,
            form,
        )
        .await;
    }
    let ordered = lines.clone();
    let placed = order::place(
        deps,
        PlaceInput {
            workspace_id: ws,
            source: "webshop",
            actor_kind: "contact",
            actor_user_id: None,
            contact: Some(ContactRef {
                contact_id: Some(shopper.contact_id),
                contact_inbox_id: shopper.contact_inbox_id,
                conversation_id: shopper.conversation_id,
                ..ContactRef::default()
            }),
            items: lines.clone(),
            customer_name: trimmed(form.customer_name.as_ref()),
            customer_email: trimmed(form.customer_email.as_ref()),
            customer_phone: trimmed(form.customer_phone.as_ref()),
            shipping_type: form.shipping_type.clone(),
            shipping_address: form.shipping_address.clone(),
            note: trimmed(form.note.as_ref()),
            discount_code: discount_code.clone(),
            auto_apply_discount: true,
            payment_method: Some(form.payment_method.clone().unwrap_or_else(|| "cod".into())),
        },
        Some(Box::new(move |conn: &Connection, _order| {
            // Only what was ordered leaves the cart; a second submit of the
            // same checkout finds its items gone and rolls back.
            let current = cart_lines(find_cart(conn, ws, shopper.contact_id)?.as_ref());
            let left = subtract_ordered_lines(&current, &ordered).ok_or_else(order_empty)?;
            upsert_lines(conn, &shopper, &left)?;
            set_code(conn, &shopper, None)
        })),
    )
    .await?;
    let order = &placed.order;
    let result = |payment_url: Option<String>| {
        json!({
            "orderId": order.id.to_string(),
            "total": order::major(order.total),
            "currency": order.currency,
            "paymentUrl": payment_url,
        })
    };
    if !order::is_awaiting_online_payment(order) {
        return Ok(result(None));
    }
    match order_payment::start_checkout(deps, ws, order.id).await {
        Ok((url, _)) => Ok(result(Some(url))),
        Err(error) => {
            undo_unpayable(deps, shopper, order.id, &lines, discount_code).await;
            Err(error)
        }
    }
}

/// `undoUnpayableOrder`: cancel the order, put its items (and code) back.
async fn undo_unpayable(
    deps: Deps<'_>,
    shopper: Shopper,
    order_id: i64,
    lines: &[PlaceItem],
    code: Option<String>,
) {
    let ws = shopper.workspace_id;
    let cancelled = order::transition(
        deps,
        ws,
        order_id,
        "cancelled",
        Actor {
            kind: "system",
            user_id: None,
        },
        None,
        false,
    )
    .await;
    let lines = lines.to_vec();
    let restored = match cancelled {
        Ok(_) => {
            tx::write(deps.db, move |tx| {
                let conn: &Connection = tx;
                let current = cart_lines(find_cart(conn, ws, shopper.contact_id)?.as_ref());
                let restored = lines
                    .iter()
                    .fold(current, |cart, line| change_line(&cart, line, Mode::Add));
                upsert_lines(conn, &shopper, &restored)?;
                match code.as_deref() {
                    Some(code) => set_code(conn, &shopper, Some(code)),
                    None => Ok(()),
                }
            })
            .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = restored {
        tracing::error!(%error, order_id, workspace_id = ws, "[webshop] could not undo an order whose payment could not start");
    }
}

// ---- customer orders ----------------------------------------------------------------

/// `getCustomerOrder`: only the shopper's own order; another's is a 404.
pub async fn customer_order(
    db: &chatbotx_db::Db,
    shopper: Shopper,
    order_id: i64,
) -> Result<OrderDetail> {
    let detail = order::get_detail(db, shopper.workspace_id, order_id).await?;
    if detail.order.contact_id != Some(shopper.contact_id) {
        return Err(order::not_found());
    }
    Ok(detail)
}

/// The order page's view of an order (`orders/[orderId]/page.tsx`): no
/// history, payments or customer details.
pub fn customer_order_json(detail: &OrderDetail) -> Value {
    const KEYS: [&str; 20] = [
        "id",
        "number",
        "status",
        "currency",
        "subtotal",
        "discountTotal",
        "shippingTotal",
        "serviceFeeTotal",
        "taxTotal",
        "couponDiscount",
        "discountCode",
        "total",
        "paymentMethod",
        "paymentStatus",
        "paymentExpiresAt",
        "shippingType",
        "createdAt",
        "rating",
        "ratingComment",
        "items",
    ];
    let mut all = order::order_json(&detail.order);
    all.insert(
        "items".into(),
        detail
            .items
            .iter()
            .map(|i| {
                let item = order::item_json(i);
                json!({
                    "id": item["id"],
                    "name": item["name"],
                    "variantLabel": item["variantLabel"],
                    "quantity": item["quantity"],
                    "lineTotal": item["lineTotal"],
                })
            })
            .collect(),
    );
    Value::Object(
        KEYS.iter()
            .map(|k| ((*k).to_string(), all.remove(*k).unwrap_or(Value::Null)))
            .collect(),
    )
}

/// `refreshCustomerOrderPayment`: back from the provider's page, the payment
/// is read back first so the order shows paid at once.
pub async fn refresh_customer_order_payment(
    deps: Deps<'_>,
    shopper: Shopper,
    order_id: i64,
) -> Result<OrderDetail> {
    let detail = customer_order(deps.db, shopper, order_id).await?;
    if !order::is_awaiting_online_payment(&detail.order) {
        return Ok(detail);
    }
    order_payment::sync_order(deps, shopper.workspace_id, order_id).await?;
    customer_order(deps.db, shopper, order_id).await
}

/// `payCustomerOrder`: the provider's page for the shopper's own order.
pub async fn pay_customer_order(deps: Deps<'_>, shopper: Shopper, order_id: i64) -> Result<String> {
    customer_order(deps.db, shopper, order_id).await?;
    Ok(
        order_payment::start_checkout(deps, shopper.workspace_id, order_id)
            .await?
            .0,
    )
}

/// `rateOrder`: a completed order, when the store allows rating.
pub async fn rate_order(
    db: &chatbotx_db::Db,
    shopper: Shopper,
    order_id: i64,
    rating: i64,
    comment: Option<String>,
) -> Result<Value> {
    let detail = customer_order(db, shopper, order_id).await?;
    let ws = shopper.workspace_id;
    let settings = tx::read(db, move |conn| ecommerce_settings::get_in(conn, ws)).await?;
    if settings["general"]["allowRateOrders"] != true || detail.order.status != "concluded" {
        return Err(rating_not_allowed());
    }
    let comment = trimmed(comment.as_ref());
    let stored = comment.clone();
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "Order" SET "rating" = ?3, "ratingComment" = ?4, "updatedAt" = ?5 WHERE "workspaceId" = ?1 AND "id" = ?2"#,
            params![ws, order_id, rating, stored, repo::now_ms()],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await?;
    Ok(json!({ "rating": rating, "ratingComment": comment }))
}

// ---- inbound (WhatsApp catalog) carts --------------------------------------------

/// `replaceFromInboundCart`'s lines: items not linked to a product here
/// cannot be ordered and are left out (the count is returned).
pub fn inbound_cart_lines(cart: &Value) -> (Vec<PlaceItem>, usize) {
    let mut lines = Vec::new();
    let mut skipped = 0;
    for item in cart["items"].as_array().into_iter().flatten() {
        let product_id = item["product"]["productId"]
            .as_str()
            .and_then(|id| id.parse::<i64>().ok());
        let Some(product_id) = product_id else {
            skipped += 1;
            continue;
        };
        let quantity = item["quantity"].as_f64().unwrap_or(0.0) as i64;
        lines = change_line(
            &lines,
            &PlaceItem {
                product_id,
                variant_id: None,
                quantity,
            },
            Mode::Add,
        );
    }
    (lines, skipped)
}

/// `buildInboundCartReply`: a cart sent through WhatsApp's own catalog
/// becomes the contact's shop cart, and the reply sums it up above a CHECK
/// OUT button (`{ text, buttonLabel, buttonUrl }`). `None` when the store
/// takes no orders or none of the items is sold here.
pub async fn build_inbound_cart_reply(
    db: &chatbotx_db::Db,
    app_url: &str,
    shopper: Shopper,
    cart: &Value,
) -> Result<Option<Value>> {
    let ws = shopper.workspace_id;
    let (settings, language) = tx::read(db, move |conn| {
        let language = workspace_row(conn, ws)?.map(|w| w.language);
        Ok((ecommerce_settings::get_in(conn, ws)?, language))
    })
    .await?;
    if settings["general"]["orderingAvailable"] != true {
        return Ok(None);
    }
    let (lines, skipped) = inbound_cart_lines(cart);
    tx::write(db, move |tx| upsert_lines(tx, &shopper, &lines)).await?;
    let priced = cart::get_cart(db, ws, shopper.contact_id, None, false).await?;
    let priced_lines = priced["lines"].as_array().cloned().unwrap_or_default();
    if priced_lines.is_empty() {
        return Ok(None);
    }
    let language = order::messages::language(language.as_deref());
    let totals = &priced["totals"];
    let number = |key: &str| totals[key].as_f64().unwrap_or(0.0);
    let discount = priced["discount"]["code"]
        .as_str()
        .map(|code| (code, priced["discount"]["amount"].as_f64().unwrap_or(0.0)));
    let text = order::messages::cart_summary_text(
        &order::messages::CartSummary {
            item_count: priced_lines
                .iter()
                .map(|l| l["quantity"].as_i64().unwrap_or(0))
                .sum(),
            currency: priced["currency"].as_str().unwrap_or(""),
            subtotal: number("subtotal"),
            discount_total: number("discountTotal"),
            shipping_total: number("shippingTotal"),
            service_fee_total: number("serviceFeeTotal"),
            tax_total: number("taxTotal"),
            total: number("total"),
            discount,
            skipped: skipped + priced["unavailable"].as_array().map_or(0, Vec::len),
        },
        language,
        &order::messages::Payments::of(&settings["payments"]),
    );
    let external = (settings["checkout"]["checkoutMode"] == "externalLink")
        .then(|| settings["checkout"]["checkoutLink"].as_str())
        .flatten();
    let button_url = external.map_or_else(
        || {
            chatbotx_core::utils::web_url::join_app_url(
                app_url,
                &format!("/shop/{ws}/cart?token={{{{shop_token}}}}"),
            )
        },
        str::to_string,
    );
    Ok(Some(json!({
        "text": text,
        "buttonLabel": order::messages::strings(language).check_out,
        "buttonUrl": button_url,
    })))
}

// ---- abandoned carts (Q.P6-U5, `scanAbandonedCarts`) -----------------------------

/// One claimed abandoned cart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbandonedCart {
    pub id: i64,
    pub workspace_id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: Option<i64>,
    pub last_activity_at: i64,
}

/// `claimAbandonedCarts` (`cartRepository.claimAbandoned`): marks up to
/// `limit` carts left longer than their store's delay as notified, oldest
/// first, and returns them. Each cart is claimed once.
pub async fn claim_abandoned_carts(
    db: &chatbotx_db::Db,
    limit: i64,
    now: i64,
) -> Result<Vec<AbandonedCart>> {
    tx::write(db, move |tx| {
        let mut stmt = tx.prepare(
            r#"UPDATE "Cart" SET "abandonedNotifiedAt" = ?1
               WHERE "abandonedNotifiedAt" IS NULL AND "id" IN (
                 SELECT c."id" FROM "Cart" c
                 JOIN "EcommerceSettings" s ON s."workspaceId" = c."workspaceId"
                 WHERE c."abandonedNotifiedAt" IS NULL AND s."abandonedCartDelayMinutes" IS NOT NULL
                   AND json_array_length(c."items") > 0
                   AND c."lastActivityAt" < ?1 - s."abandonedCartDelayMinutes" * 60000
                 ORDER BY c."lastActivityAt" LIMIT ?2)
               RETURNING "id", "workspaceId", "contactId", "contactInboxId", "lastActivityAt""#,
        )?;
        let mut carts = stmt
            .query_map(params![now, limit], |r| {
                Ok(AbandonedCart {
                    id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    contact_id: r.get(2)?,
                    contact_inbox_id: r.get(3)?,
                    last_activity_at: r.get(4)?,
                })
            })?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
        carts.sort_by_key(|c| (c.last_activity_at, c.id));
        Ok(carts)
    })
    .await
}

/// `remindAbandonedCart`'s outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reminder {
    Reminded,
    Disabled,
    Empty,
    Stale,
}

/// `remindAbandonedCart`: the `cartAbandoned` trigger event and the store's
/// abandoned-cart flow in the customer's conversation (the cart's channel
/// when it is still there). Nothing for an emptied cart, a cart changed since
/// the claim, or a store that turned the reminder off.
pub async fn remind_abandoned_cart(
    db: &chatbotx_db::Db,
    jobs: &dyn chatbotx_jobs::Enqueuer,
    bus: &crate::events::EventBus,
    cart: AbandonedCart,
) -> Result<Reminder> {
    let (settings, latest) = tx::read(db, move |conn| {
        let settings: Option<(Option<i64>, Option<i64>)> = conn
            .query_row(
                r#"SELECT "abandonedCartDelayMinutes", "abandonedCartFlowId" FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#,
                [cart.workspace_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let latest: Option<i64> = conn
            .query_row(
                r#"SELECT "lastActivityAt" FROM "Cart" WHERE "workspaceId" = ?1 AND "contactId" = ?2"#,
                [cart.workspace_id, cart.contact_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok((settings, latest))
    })
    .await?;
    let Some((Some(_), flow_id)) = settings else {
        return Ok(Reminder::Disabled);
    };
    if latest != Some(cart.last_activity_at) {
        return Ok(Reminder::Stale);
    }
    let priced = cart::get_cart(db, cart.workspace_id, cart.contact_id, None, false).await?;
    let lines = priced["lines"].as_array().cloned().unwrap_or_default();
    if lines.is_empty() {
        return Ok(Reminder::Empty);
    }
    let ids: Vec<i64> = lines
        .iter()
        .filter_map(|l| l["productId"].as_str()?.parse().ok())
        .collect();
    let ws = cart.workspace_id;
    let products = tx::read(db, move |conn| {
        Ok(product_repo::find_by_ids(conn, ws, &ids)?)
    })
    .await?;
    let target =
        match crate::conversation::resolve_contact_inbox_for_send(db, ws, cart.contact_id, None)
            .await
        {
            Ok((found, recent)) => {
                let chosen = found
                    .contact_inboxes
                    .iter()
                    .find(|ci| Some(ci.id) == cart.contact_inbox_id)
                    .cloned()
                    .unwrap_or(recent);
                Some((found.conversation.id, chosen.id, chosen.channel))
            }
            Err(error) if error.code() == Some("notFound") => None,
            Err(error) => return Err(error),
        };
    let items: Vec<Value> = lines
        .iter()
        .map(|line| {
            let product = line["productId"]
                .as_str()
                .and_then(|id| id.parse::<i64>().ok())
                .and_then(|id| products.iter().find(|p| p.id == id));
            let category_ids: Vec<String> = product
                .map(|p| [p.category_id, p.subcategory_id])
                .into_iter()
                .flatten()
                .flatten()
                .map(|id| id.to_string())
                .collect();
            json!({
                "productId": line["productId"],
                "variantId": line["variantId"],
                "name": line["name"],
                "categoryIds": category_ids,
                "quantity": line["quantity"],
            })
        })
        .collect();
    let mut metadata = Map::new();
    metadata.insert(
        "channel".into(),
        json!(target.as_ref().map(|t| t.2.clone())),
    );
    metadata.insert(
        "conversationId".into(),
        json!(target.as_ref().map(|t| t.0.to_string())),
    );
    metadata.insert(
        "itemCount".into(),
        json!(
            lines
                .iter()
                .map(|l| l["quantity"].as_i64().unwrap_or(0))
                .sum::<i64>()
        ),
    );
    metadata.insert("total".into(), priced["totals"]["total"].clone());
    metadata.insert("currency".into(), priced["currency"].clone());
    metadata.insert("items".into(), Value::Array(items));
    let contact_inbox = target.as_ref().map(|t| t.1.to_string());
    let event = crate::events::trigger::cart_item_changed(
        "cartAbandoned",
        &ws.to_string(),
        &cart.contact_id.to_string(),
        metadata,
        contact_inbox.as_deref(),
    );
    bus.publish(crate::events::Event::Trigger(event)).await;
    if let (Some(flow_id), Some((conversation_id, contact_inbox_id, _))) = (flow_id, &target) {
        let opts = chatbotx_jobs::JobOptions {
            // Once per abandonment: a cart changed and left again is a new one.
            job_id: Some(format!(
                "abandoned-cart-flow-{}-{}",
                cart.id, cart.last_activity_at
            )),
            ..chatbotx_jobs::JobOptions::default()
        };
        jobs.add(
            chatbotx_jobs::QueueName::Integration,
            "sendFlow",
            json!({ "type": "sendFlow", "data": {
                "conversationId": conversation_id.to_string(),
                "contactInboxId": contact_inbox_id.to_string(),
                "flowId": flow_id.to_string(),
            }}),
            opts,
        )
        .await
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    }
    Ok(Reminder::Reminded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(product_id: i64, variant_id: Option<i64>, quantity: i64) -> PlaceItem {
        PlaceItem {
            product_id,
            variant_id,
            quantity,
        }
    }

    #[test]
    fn change_line_adds_sets_caps_and_removes() {
        let cart = vec![line(1, None, 2), line(2, Some(5), 1)];
        assert_eq!(
            change_line(&cart, &line(1, None, 3), Mode::Add),
            vec![line(1, None, 5), line(2, Some(5), 1)]
        );
        assert_eq!(
            change_line(&cart, &line(2, Some(5), 0), Mode::Set),
            vec![line(1, None, 2)]
        );
        assert_eq!(
            change_line(&cart, &line(3, None, 20_000), Mode::Add)[2],
            line(3, None, 10_000)
        );
        // A different variant is another line.
        assert_eq!(change_line(&cart, &line(2, None, 1), Mode::Add).len(), 3);
    }

    #[test]
    fn subtract_keeps_lines_added_meanwhile() {
        let cart = vec![line(1, None, 3), line(2, None, 1)];
        assert_eq!(
            subtract_ordered_lines(&cart, &[line(1, None, 2)]),
            Some(vec![line(1, None, 1), line(2, None, 1)])
        );
        assert_eq!(
            subtract_ordered_lines(&cart, &[line(2, None, 1)]),
            Some(vec![line(1, None, 3)])
        );
        // A second submit: the cart no longer holds the order.
        assert_eq!(subtract_ordered_lines(&cart, &[line(1, None, 4)]), None);
    }

    #[test]
    fn hidden_parent_hides_its_children() {
        let rows = vec![
            (1, None, true),
            (2, Some(1), true),
            (3, None, false),
            (4, Some(3), true),
        ];
        let shown = visible_shop_categories(rows, |c| *c);
        assert_eq!(shown, vec![(1, None, true), (2, Some(1), true)]);
    }

    #[test]
    fn checkout_form_rules() {
        let settings = json!({
            "checkout": { "requireName": true, "requireZip": true },
            "shipping": { "shippingType": "delivery", "shippingCountries": ["DE"] },
        });
        let mut form = CheckoutForm::default();
        let field = |r: Result<()>| match r {
            Err(BusinessError::Exception { field, .. }) => field,
            _ => None,
        };
        assert_eq!(
            field(assert_checkout_form(&settings, &form)).as_deref(),
            Some("customerName")
        );
        form.customer_name = Some("Ann".into());
        form.shipping_address = Some(json!({ "line1": "Main 1", "zip": "1", "country": "fr" }));
        assert_eq!(
            field(assert_checkout_form(&settings, &form)).as_deref(),
            Some("shippingAddress.country")
        );
        form.shipping_address = Some(json!({ "line1": "Main 1", "zip": "1", "country": "de" }));
        assert!(assert_checkout_form(&settings, &form).is_ok());
        // Pickup needs no address.
        form.shipping_address = None;
        form.shipping_type = Some("pickup".into());
        assert!(assert_checkout_form(&settings, &form).is_ok());
    }

    #[test]
    fn inbound_cart_keeps_linked_items() {
        let cart = json!({ "items": [
            { "product": { "productId": "5" }, "quantity": 2 },
            { "product": null, "quantity": 1 },
            { "product": { "productId": "5" }, "quantity": 1 },
        ] });
        let (lines, skipped) = inbound_cart_lines(&cart);
        assert_eq!(lines, vec![line(5, None, 3)]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn closed_reason_data() {
        let BusinessError::Exception {
            code,
            data,
            message,
            ..
        } = closed("outsideOpenHours", Some("Bakery"))
        else {
            panic!("exception");
        };
        assert_eq!(code, "webshopClosed");
        assert_eq!(message, "\"Bakery\" is closed right now.");
        assert_eq!(
            data,
            Some(json!({ "reason": "outsideOpenHours", "vendorName": "Bakery" }))
        );
    }
}
