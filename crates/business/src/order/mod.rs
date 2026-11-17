//! `packages/business/src/order/{service,errors}.ts`: orders placed here
//! (web shop, API, a member), priced on the server from the products and the
//! store settings, their status moves, deletion, list and CSV export.
//!
//! The SQLite writer is single and its closures are sync, so provider calls
//! (closing / refunding a payment page) run before the write transaction and
//! the transaction checks the order again.

mod effects;
pub mod messages;

pub use effects::{Deps, after_paid, announce_placed};

use std::collections::HashMap;

use chatbotx_core::utils::money::{
    LinePriceInput, compute_line_price, percent_of, round_money, stack_discount_percents, sum_money,
};
use chatbotx_db::repo::models::{Order, OrderItem, OrderPayment};
use chatbotx_db::repo::order::{self as order_repo, OrderDetail, OrderListInput, OrderListPage};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::product::{self as product_repo, OrderProduct};
use chatbotx_db::repo::{self, iso};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use crate::discount_code::{self, Applied, CouponLine, OrderContext};
use crate::errors::{BusinessError, Result};
use crate::payment_provider::method_unavailable;
use crate::store_source::{external_store_managed, is_external_store_source};
use crate::{ecommerce_settings, order_payment, tx};

pub const STATUSES: &[&str] = &["pending", "accepted", "shipped", "concluded", "cancelled"];
pub const SOURCES: &[&str] = &["webshop", "api", "manual", "shopify", "woocommerce"];
pub const PAYMENT_METHODS: &[&str] = &["cod", "stripe", "paypal"];
pub const PAYMENT_STATUSES: &[&str] = &["unpaid", "paid", "refunded"];
pub const FULFILLMENT_STATUSES: &[&str] = &["unfulfilled", "partial", "fulfilled"];
pub const EXPORT_LIMIT: usize = 5000;
const EXPORT_PAGE_SIZE: i64 = 50;
pub const PAYMENT_EXPIRY_MINUTES: i64 = 30;
pub const PAYMENT_EXPIRED_CANCEL_REASON: &str = "Payment not received in time.";
const STOCK_HOLDING: &[&str] = &["pending", "accepted", "shipped"];
const MONEY_SCALE: f64 = 10_000.0;

pub fn major(scaled: i64) -> f64 {
    scaled as f64 / MONEY_SCALE
}

pub fn scaled(major: f64) -> i64 {
    (major * MONEY_SCALE).round() as i64
}

fn number(value: f64) -> Value {
    crate::payments::js_number(value)
        .parse::<serde_json::Number>()
        .map(Value::Number)
        .unwrap_or(json!(value))
}

fn money(scaled: i64) -> Value {
    number(major(scaled))
}

// ---- errors.ts ------------------------------------------------------------------

fn exception(message: impl Into<String>, code: &str, status: u16) -> BusinessError {
    BusinessError::exception(message, code, status)
}

pub fn not_found() -> BusinessError {
    BusinessError::not_found("Order does not exist.")
}

pub fn product_unavailable(detail: impl Into<String>) -> BusinessError {
    exception(detail, "orderProductUnavailable", 400)
}

pub fn invalid_transition(from: &str, to: &str) -> BusinessError {
    exception(
        format!("An order that is {from} cannot become {to}."),
        "orderInvalidStatusTransition",
        409,
    )
    .with_data(json!({ "from": from, "to": to }))
}

pub fn refund_failed(provider: &str) -> BusinessError {
    exception(
        format!("{provider} could not refund the payment, so the order was not cancelled. Try again in a moment."),
        "orderRefundFailed",
        502,
    )
    .with_data(json!({ "provider": provider }))
}

pub fn payment_processing() -> BusinessError {
    exception(
        "A payment for this order is still being processed. Try again later.",
        "orderPaymentProcessing",
        409,
    )
}

pub fn payment_contact_required() -> BusinessError {
    exception(
        "An order paid online needs a contact (the customer).",
        "orderPaymentContactRequired",
        400,
    )
}

pub fn payment_not_open(reason: &str) -> BusinessError {
    exception(
        "This order cannot be paid online now.",
        "orderPaymentNotOpen",
        409,
    )
    .with_data(json!({ "reason": reason }))
}

pub fn paid_online() -> BusinessError {
    exception(
        "This order was paid online. Cancel it first: cancelling refunds the payment.",
        "orderPaidOnline",
        409,
    )
}

// ---- partials/ecommerce.ts ------------------------------------------------------

pub fn is_online(method: &str) -> bool {
    method == "stripe" || method == "paypal"
}

/// `isAwaitingOnlinePayment`.
pub fn is_awaiting_online_payment(order: &Order) -> bool {
    is_online(&order.payment_method) && order.payment_status == "unpaid"
}

/// `getOrderStatusesAllowedBefore`.
pub fn allowed_before(to: &str) -> Vec<String> {
    let from: &[&str] = match to {
        "accepted" => &["pending"],
        "shipped" => &["accepted"],
        "concluded" => &["accepted", "shipped"],
        "cancelled" => &["pending", "accepted", "shipped"],
        _ => &[],
    };
    from.iter().map(|s| s.to_string()).collect()
}

/// `SHIPPING_TYPES_BY_MODE`.
fn shipping_types(mode: &str) -> &'static [&'static str] {
    match mode {
        "delivery" => &["delivery"],
        "pickup" => &["pickup"],
        "deliveryAndPickup" => &["delivery", "pickup"],
        _ => &[],
    }
}

// ---- JSON (the API shapes) ------------------------------------------------------

fn opt_id(id: Option<i64>) -> Value {
    id.map_or(Value::Null, |id| json!(id.to_string()))
}

fn opt_date(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, |ms| json!(iso(ms)))
}

/// An order row (`ORDER_KEYS`).
pub fn order_json(o: &Order) -> Map<String, Value> {
    let value = json!({
        "id": o.id.to_string(),
        "number": o.number,
        "contactId": opt_id(o.contact_id),
        "inboxId": opt_id(o.inbox_id),
        "conversationId": opt_id(o.conversation_id),
        "channel": o.channel,
        "source": o.source,
        "status": o.status,
        "paymentMethod": o.payment_method,
        "paymentStatus": o.payment_status,
        "paymentExpiresAt": opt_date(o.payment_expires_at),
        "paidAt": opt_date(o.paid_at),
        "currency": o.currency,
        "subtotal": money(o.subtotal),
        "discountTotal": money(o.discount_total),
        "shippingTotal": money(o.shipping_total),
        "serviceFeeTotal": money(o.service_fee_total),
        "taxTotal": money(o.tax_total),
        "couponDiscount": money(o.coupon_discount),
        "total": money(o.total),
        "discountCode": o.discount_code,
        "customerName": o.customer_name,
        "customerEmail": o.customer_email,
        "customerPhone": o.customer_phone,
        "shippingType": o.shipping_type,
        "shippingAddress": o.shipping_address.as_ref().map_or(Value::Null, |a| a.0.clone()),
        "note": o.note,
        "rating": o.rating,
        "ratingComment": o.rating_comment,
        "acceptedAt": opt_date(o.accepted_at),
        "shippedAt": opt_date(o.shipped_at),
        "concludedAt": opt_date(o.concluded_at),
        "cancelledAt": opt_date(o.cancelled_at),
        "cancelReason": o.cancel_reason,
        "externalId": o.external_id,
        "externalName": o.external_name,
        "fulfillmentStatus": o.fulfillment_status,
        "trackingCompany": o.tracking_company,
        "trackingNumber": o.tracking_number,
        "trackingUrl": o.tracking_url,
        "createdAt": iso(o.created_at),
        "updatedAt": iso(o.updated_at),
    });
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

pub fn item_json(i: &OrderItem) -> Value {
    json!({
        "id": i.id.to_string(),
        "productId": opt_id(i.product_id),
        "variantId": opt_id(i.variant_id),
        "vendorId": opt_id(i.vendor_id),
        "name": i.name,
        "variantLabel": i.variant_label,
        "sku": i.sku,
        "retailerId": i.retailer_id,
        "imageUrl": i.image_url,
        "vendorName": i.vendor_name,
        "unitPrice": money(i.unit_price),
        "quantity": i.quantity,
        "discountPercent": money(i.discount_percent),
        "taxPercent": money(i.tax_percent),
        "discountTotal": money(i.discount_total),
        "taxTotal": money(i.tax_total),
        "lineTotal": money(i.line_total),
    })
}

pub fn payment_json(p: &OrderPayment) -> Value {
    json!({
        "id": p.id.to_string(),
        "provider": p.provider,
        "status": p.status,
        "amount": money(p.amount),
        "currency": p.currency,
        "livemode": p.livemode,
        "providerReference": p.provider_reference,
        "transactionId": p.transaction_id,
        "refundId": p.refund_id,
        "expiresAt": iso(p.expires_at),
        "paidAt": opt_date(p.paid_at),
        "refundedAt": opt_date(p.refunded_at),
        "createdAt": iso(p.created_at),
    })
}

/// `getDetail` as the API returns it.
pub fn detail_json(detail: &OrderDetail) -> Value {
    let mut map = order_json(&detail.order);
    map.insert("items".into(), detail.items.iter().map(item_json).collect());
    map.insert(
        "history".into(),
        detail
            .history
            .iter()
            .map(|h| {
                json!({
                    "id": h.history.id.to_string(),
                    "fromStatus": h.history.from_status,
                    "toStatus": h.history.to_status,
                    "actorType": h.history.actor_type,
                    "actorUser": h.actor_user.as_ref().map_or(Value::Null, |(id, name)| json!({ "id": id.to_string(), "name": name })),
                    "note": h.history.note,
                    "createdAt": iso(h.history.created_at),
                })
            })
            .collect(),
    );
    map.insert(
        "payments".into(),
        detail.payments.iter().map(payment_json).collect(),
    );
    Value::Object(map)
}

/// A list page (`data` rows with `itemCount`, `pageCount`).
pub fn page_json(page: &OrderListPage) -> Value {
    json!({
        "data": page.data.iter().map(|row| {
            let mut map = order_json(&row.order);
            map.insert("itemCount".into(), json!(row.item_count));
            Value::Object(map)
        }).collect::<Vec<_>>(),
        "pageCount": page.page_count,
    })
}

// ---- list / export / get --------------------------------------------------------

fn default_sources(conn: &Connection, workspace_id: i64) -> Result<Vec<String>> {
    let store = crate::store_source::resolve_store_source(conn, workspace_id)?;
    Ok(if is_external_store_source(&store) {
        vec![store]
    } else {
        SOURCES
            .iter()
            .filter(|s| !is_external_store_source(s))
            .map(|s| s.to_string())
            .collect()
    })
}

/// `list`: the store's current sources unless `source` names some.
pub async fn list(db: &chatbotx_db::Db, mut input: OrderListInput) -> Result<OrderListPage> {
    tx::read(db, move |conn| {
        if input.source.is_empty() {
            input.source = default_sources(conn, input.workspace_id)?;
        }
        Ok(order_repo::list(conn, &input)?)
    })
    .await
}

/// `toCsvCell`.
fn csv_cell(value: &str) -> String {
    let safe = if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if safe.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// `exportCsv`: up to [`EXPORT_LIMIT`] rows of `list`, newest first.
pub async fn export_csv(db: &chatbotx_db::Db, mut input: OrderListInput) -> Result<Value> {
    let (rows, truncated) = tx::read(db, move |conn| {
        if input.source.is_empty() {
            input.source = default_sources(conn, input.workspace_id)?;
        }
        input.per_page = Some(EXPORT_PAGE_SIZE);
        let mut data = Vec::new();
        let mut page = 1;
        let page_count = loop {
            input.page = Some(page);
            let result = order_repo::list(conn, &input)?;
            data.extend(result.data);
            page += 1;
            if page > result.page_count || data.len() >= EXPORT_LIMIT {
                break result.page_count;
            }
        };
        let truncated = data.len() > EXPORT_LIMIT || page <= page_count;
        data.truncate(EXPORT_LIMIT);
        Ok((data, truncated))
    })
    .await?;
    let header =
        "Order,Created at,Status,Payment,Fulfillment,Customer,Email,Phone,Items,Currency,Total";
    let mut lines = vec![header.to_string()];
    for row in &rows {
        let o = &row.order;
        let cells = [
            o.external_name
                .clone()
                .unwrap_or_else(|| format!("#{}", o.number)),
            iso(o.created_at),
            o.status.clone(),
            o.payment_status.clone(),
            o.fulfillment_status.clone().unwrap_or_default(),
            o.customer_name.clone().unwrap_or_default(),
            o.customer_email.clone().unwrap_or_default(),
            o.customer_phone.clone().unwrap_or_default(),
            row.item_count.to_string(),
            o.currency.clone(),
            crate::payments::js_number(major(o.total)),
        ];
        lines.push(
            cells
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    Ok(json!({ "csv": lines.join("\r\n"), "rowCount": rows.len(), "truncated": truncated }))
}

pub fn find_detail(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<OrderDetail> {
    order_repo::find_detail(conn, workspace_id, order_id)?.ok_or_else(not_found)
}

/// `getDetail`.
pub async fn get_detail(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    order_id: i64,
) -> Result<OrderDetail> {
    tx::read(db, move |conn| find_detail(conn, workspace_id, order_id)).await
}

// ---- place ----------------------------------------------------------------------

/// `OrderActor`.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    /// `user` | `apiToken` | `system` | `contact`.
    pub kind: &'static str,
    pub user_id: Option<i64>,
}

/// `OrderContactRef`.
#[derive(Debug, Clone, Default)]
pub struct ContactRef {
    pub contact_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
    pub inbox_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceItem {
    pub product_id: i64,
    pub variant_id: Option<i64>,
    pub quantity: i64,
}

/// `PlaceOrderInput` (without `onPlaced`: the web shop empties its cart
/// through `on_placed`).
#[derive(Debug, Clone, Default)]
pub struct PlaceInput {
    pub workspace_id: i64,
    pub source: &'static str,
    pub actor_kind: &'static str,
    pub actor_user_id: Option<i64>,
    pub contact: Option<ContactRef>,
    pub items: Vec<PlaceItem>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub customer_phone: Option<String>,
    pub shipping_type: Option<String>,
    pub shipping_address: Option<Value>,
    pub note: Option<String>,
    pub discount_code: Option<String>,
    pub auto_apply_discount: bool,
    pub payment_method: Option<String>,
}

/// `mergeItems`.
pub fn merge_items(items: &[PlaceItem]) -> Vec<PlaceItem> {
    let mut merged: Vec<PlaceItem> = Vec::new();
    for item in items {
        match merged
            .iter_mut()
            .find(|m| m.product_id == item.product_id && m.variant_id == item.variant_id)
        {
            Some(existing) => existing.quantity += item.quantity,
            None => merged.push(item.clone()),
        }
    }
    merged
}

/// A priced line (major units) and the product its stock comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub product_id: i64,
    pub variant_id: Option<i64>,
    pub vendor_id: Option<i64>,
    pub category_id: Option<i64>,
    pub name: String,
    pub variant_label: Option<String>,
    pub combination: Option<Value>,
    pub sku: Option<String>,
    pub image_url: Option<String>,
    pub vendor_name: Option<String>,
    pub unit_price: f64,
    pub quantity: i64,
    pub discount_percent: f64,
    pub tax_percent: f64,
    pub discount_total: f64,
    pub tax_total: f64,
    pub line_total: f64,
    pub gross: f64,
}

/// `assertOrderable`.
pub fn assert_orderable(product: Option<&OrderProduct>, id: i64) -> Result<&OrderProduct> {
    let Some(product) = product.filter(|p| p.product.status == "active") else {
        return Err(product_unavailable(format!(
            "Product {id} is not available."
        )));
    };
    if let Some((_, false, ..)) = &product.vendor {
        return Err(product_unavailable(format!(
            "The vendor of \"{}\" is not taking orders.",
            product.product.name
        )));
    }
    Ok(product)
}

/// "Size: L, Color: Red".
fn variant_label(combination: &Value) -> String {
    combination
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// `priceLine`.
pub fn price_line(product: &OrderProduct, item: &PlaceItem, currency: &str) -> Result<Line> {
    let p = &product.product;
    let variant = item
        .variant_id
        .and_then(|id| product.variants.iter().find(|v| v.id == id));
    if item.variant_id.is_some() && !variant.is_some_and(|v| v.is_enabled) {
        return Err(product_unavailable(format!(
            "The chosen variant of \"{}\" is not available.",
            p.name
        )));
    }
    if item.variant_id.is_none() && product.variants.iter().any(|v| v.is_enabled) {
        return Err(product_unavailable(format!(
            "Choose a variant of \"{}\".",
            p.name
        )));
    }
    let unit_price = variant.map_or(p.price, |v| v.price);
    let vendor_percent = product.vendor.as_ref().map_or(0.0, |v| major(v.2));
    let discount_percent = stack_discount_percents(p.discount, vendor_percent);
    let line = compute_line_price(&LinePriceInput {
        unit_price,
        quantity: item.quantity as f64,
        discount_percent: Some(discount_percent),
        tax_percent: Some(p.taxes),
        currency,
    });
    Ok(Line {
        product_id: p.id,
        variant_id: variant.map(|v| v.id),
        vendor_id: p.vendor_id,
        category_id: p.category_id,
        name: p.name.clone(),
        variant_label: variant.map(|v| variant_label(&v.combination.0)),
        combination: variant.map(|v| v.combination.0.clone()),
        sku: p.sku.clone(),
        image_url: p.images.0[0]["url"].as_str().map(str::to_string),
        vendor_name: product.vendor.as_ref().map(|v| v.0.clone()),
        unit_price,
        quantity: item.quantity,
        discount_percent,
        tax_percent: p.taxes,
        discount_total: line.discount_total,
        tax_total: line.tax_total,
        line_total: line.line_total,
        gross: line.gross,
    })
}

/// `splitOrderableItems`: what can be ordered now, and what cannot (named
/// when the product still exists).
pub fn split_orderable(
    products: &HashMap<i64, OrderProduct>,
    items: &[PlaceItem],
) -> (Vec<PlaceItem>, Vec<(PlaceItem, Option<String>)>) {
    let mut orderable = Vec::new();
    let mut unavailable = Vec::new();
    for item in items {
        let ok = assert_orderable(products.get(&item.product_id), item.product_id)
            .and_then(|p| price_line(p, item, "USD"))
            .is_ok();
        if ok {
            orderable.push(item.clone());
        } else {
            unavailable.push((
                item.clone(),
                products
                    .get(&item.product_id)
                    .map(|p| p.product.name.clone()),
            ));
        }
    }
    (orderable, unavailable)
}

pub fn find_products(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<HashMap<i64, OrderProduct>> {
    Ok(product_repo::find_for_order(conn, workspace_id, ids, None)?
        .into_iter()
        .map(|p| (p.product.id, p))
        .collect())
}

/// `OrderTotals` (major units).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Totals {
    pub subtotal: f64,
    pub discount_total: f64,
    pub tax_total: f64,
    pub service_fee_total: f64,
    pub shipping_total: f64,
    pub coupon_discount: f64,
    pub total: f64,
}

impl Totals {
    pub fn json(&self) -> Value {
        json!({
            "subtotal": number(self.subtotal),
            "discountTotal": number(self.discount_total),
            "shippingTotal": number(self.shipping_total),
            "serviceFeeTotal": number(self.service_fee_total),
            "taxTotal": number(self.tax_total),
            "couponDiscount": number(self.coupon_discount),
            "total": number(self.total),
        })
    }
}

/// `computeTotals`.
pub fn compute_totals(
    lines: &[Line],
    currency: &str,
    shipping_total: f64,
    service_fee_percent: f64,
    coupon: Option<(f64, bool)>,
) -> Totals {
    let sum = |f: fn(&Line) -> f64| sum_money(&lines.iter().map(f).collect::<Vec<_>>(), currency);
    let subtotal = sum(|l| l.gross);
    let discount_total = sum(|l| l.discount_total);
    let tax_total = sum(|l| l.tax_total);
    let coupon_discount = coupon.map_or(0.0, |c| c.0);
    let goods_coupon = if coupon.is_some_and(|c| c.1) {
        0.0
    } else {
        coupon_discount
    };
    let service_fee_total = percent_of(
        sum_money(&[subtotal, -discount_total, -goods_coupon], currency),
        service_fee_percent,
        currency,
    );
    let shipping_total = round_money(shipping_total, currency);
    Totals {
        subtotal,
        discount_total,
        tax_total,
        service_fee_total,
        shipping_total,
        coupon_discount,
        total: sum_money(
            &[
                subtotal,
                -discount_total,
                tax_total,
                service_fee_total,
                shipping_total,
                -coupon_discount,
            ],
            currency,
        ),
    }
}

/// `goodsAfterDiscounts`.
pub fn goods_after_discounts(totals: &Totals, discount: Option<&Applied>, currency: &str) -> f64 {
    let coupon = match discount {
        Some(d) if d.discount_type == "freeShipping" => 0.0,
        _ => totals.coupon_discount,
    };
    sum_money(
        &[totals.subtotal, -totals.discount_total, -coupon],
        currency,
    )
}

/// `quote`'s result (no `availableDiscounts`: the web shop cart lists them).
pub struct Quote {
    pub draft: Draft,
    /// A typed code that cannot be used: its error `code` and `data`.
    pub discount_error: Option<(String, Option<Value>)>,
    pub unavailable: Vec<(PlaceItem, Option<String>)>,
}

/// `quote`: prices the orderable items; a typed code that cannot be used
/// does not fail the quote (it is priced without it and reported).
pub async fn quote(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    items: Vec<PlaceItem>,
    shipping_type: Option<String>,
    contact_id: Option<i64>,
    discount_code: Option<String>,
    auto_apply: bool,
) -> Result<Quote> {
    tx::read(db, move |conn| {
        let items = merge_items(&items);
        let products = find_products(
            conn,
            workspace_id,
            &items.iter().map(|i| i.product_id).collect::<Vec<_>>(),
        )?;
        let settings = ecommerce_settings::get_in(conn, workspace_id)?;
        let (orderable, unavailable) = split_orderable(&products, &items);
        let loaded = || Some((products.clone(), settings.clone()));
        let shipping = shipping_type.as_deref();
        match price_draft(
            conn,
            workspace_id,
            &orderable,
            shipping,
            contact_id,
            discount_code.as_deref(),
            auto_apply,
            loaded(),
        ) {
            Ok(draft) => Ok(Quote {
                draft,
                discount_error: None,
                unavailable,
            }),
            Err(BusinessError::Exception { code, data, .. })
                if discount_code.is_some() && code.starts_with("discountCode") =>
            {
                let draft = price_draft(
                    conn,
                    workspace_id,
                    &orderable,
                    shipping,
                    contact_id,
                    None,
                    auto_apply,
                    loaded(),
                )?;
                Ok(Quote {
                    draft,
                    discount_error: Some((code, data)),
                    unavailable,
                })
            }
            Err(error) => Err(error),
        }
    })
    .await
}

/// `priceDraft`'s result.
pub struct Draft {
    pub settings: Value,
    pub currency: String,
    pub products: HashMap<i64, OrderProduct>,
    pub lines: Vec<Line>,
    pub shipping_type: Option<String>,
    pub totals: Totals,
    pub discount: Option<Applied>,
    pub coupon_context: OrderContext,
}

/// `priceDraft` (`redeeming` = inside the order's transaction).
pub fn price_draft(
    conn: &Connection,
    workspace_id: i64,
    items: &[PlaceItem],
    shipping_type: Option<&str>,
    contact_id: Option<i64>,
    discount_code: Option<&str>,
    auto_apply: bool,
    loaded: Option<(HashMap<i64, OrderProduct>, Value)>,
) -> Result<Draft> {
    let (products, settings) = match loaded {
        Some(loaded) => loaded,
        None => (
            find_products(
                conn,
                workspace_id,
                &items.iter().map(|i| i.product_id).collect::<Vec<_>>(),
            )?,
            ecommerce_settings::get_in(conn, workspace_id)?,
        ),
    };
    let currency = settings["payments"]["currency"]
        .as_str()
        .unwrap_or("USD")
        .to_string();
    let lines = items
        .iter()
        .map(|item| {
            price_line(
                assert_orderable(products.get(&item.product_id), item.product_id)?,
                item,
                &currency,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let allowed = shipping_types(
        settings["shipping"]["shippingType"]
            .as_str()
            .unwrap_or("none"),
    );
    let shipping_type = match (allowed.is_empty(), shipping_type) {
        (true, _) => None,
        (false, None) => allowed.first().map(|s| s.to_string()),
        (false, Some(requested)) if allowed.contains(&requested) => Some(requested.to_string()),
        (false, Some(requested)) => {
            return Err(exception(
                format!("The store does not offer {requested}."),
                "orderShippingTypeUnavailable",
                400,
            )
            .with_data(json!({ "shippingType": requested })));
        }
    };
    let shipping_total = if shipping_type.as_deref() == Some("delivery") {
        round_money(
            settings["shipping"]["deliveryCost"].as_f64().unwrap_or(0.0),
            &currency,
        )
    } else {
        0.0
    };
    let coupon_context = OrderContext {
        workspace_id,
        source: settings["general"]["storeSource"]
            .as_str()
            .unwrap_or("chatbotx")
            .to_string(),
        contact_id,
        lines: lines
            .iter()
            .map(|line| CouponLine {
                product_id: Some(line.product_id),
                category_ids: products.get(&line.product_id).map_or_else(Vec::new, |p| {
                    [p.product.category_id, p.product.subcategory_id]
                        .into_iter()
                        .flatten()
                        .collect()
                }),
                amount: round_money(line.gross - line.discount_total, &currency),
                quantity: line.quantity,
            })
            .collect(),
        shipping_type: shipping_type.clone(),
        shipping_total,
        currency: currency.clone(),
    };
    let discount = if lines.is_empty() {
        None
    } else {
        discount_code::resolve(
            conn,
            &coupon_context,
            discount_code,
            auto_apply,
            repo::now_ms(),
        )?
    };
    let totals = compute_totals(
        &lines,
        &currency,
        shipping_total,
        settings["payments"]["serviceFeePercent"]
            .as_f64()
            .unwrap_or(0.0),
        discount
            .as_ref()
            .map(|d| (d.amount, d.discount_type == "freeShipping")),
    );
    Ok(Draft {
        settings,
        currency,
        products,
        lines,
        shipping_type,
        totals,
        discount,
        coupon_context,
    })
}

/// `assertVendorMinimums`.
fn assert_vendor_minimums(
    lines: &[Line],
    products: &HashMap<i64, OrderProduct>,
    currency: &str,
) -> Result<()> {
    let mut by_vendor: Vec<(i64, Vec<f64>)> = Vec::new();
    for line in lines {
        if let Some(vendor) = line.vendor_id {
            let amount = line.gross - line.discount_total;
            match by_vendor.iter_mut().find(|(v, _)| *v == vendor) {
                Some((_, amounts)) => amounts.push(amount),
                None => by_vendor.push((vendor, vec![amount])),
            }
        }
    }
    for (vendor_id, amounts) in by_vendor {
        let vendor = products
            .values()
            .find(|p| p.product.vendor_id == Some(vendor_id))
            .and_then(|p| p.vendor.as_ref());
        if let Some((name, _, _, minimum, _)) = vendor {
            let minimum = major(*minimum);
            if minimum > 0.0 && sum_money(&amounts, currency) < minimum {
                return Err(exception(
                    format!(
                        "The items from \"{name}\" are below its minimum order of {}.",
                        crate::payments::js_number(minimum)
                    ),
                    "orderBelowVendorMinimum",
                    400,
                )
                .with_data(json!({ "vendorName": name, "minimum": number(minimum) })));
            }
        }
    }
    Ok(())
}

/// `takeStock`: per product; `(product, taken)`.
fn take_stock(
    conn: &Connection,
    workspace_id: i64,
    lines: &[Line],
    products: &HashMap<i64, OrderProduct>,
) -> Result<HashMap<i64, i64>> {
    let mut needed: Vec<(i64, i64)> = Vec::new();
    for line in lines {
        match needed.iter_mut().find(|(p, _)| *p == line.product_id) {
            Some((_, q)) => *q += line.quantity,
            None => needed.push((line.product_id, line.quantity)),
        }
    }
    let ids: Vec<i64> = needed.iter().map(|(p, _)| *p).collect();
    let mut taken = HashMap::new();
    for row in product_repo::lock_for_stock(conn, workspace_id, &ids)? {
        let quantity = needed.iter().find(|(p, _)| *p == row.id).map_or(0, |n| n.1);
        if row.inventory_policy != "track" || quantity == 0 {
            continue;
        }
        if row.inventory_quantity < quantity && !row.allow_out_of_stock_purchase {
            let name = products
                .get(&row.id)
                .map_or_else(|| row.id.to_string(), |p| p.product.name.clone());
            return Err(exception(
                format!("Not enough stock for \"{name}\"."),
                "orderOutOfStock",
                400,
            )
            .with_data(json!({ "productName": name })));
        }
        taken.insert(
            row.id,
            product_repo::take_inventory(conn, workspace_id, row.id, quantity)?,
        );
    }
    Ok(taken)
}

/// `returnStock`: what each line recorded as taken.
fn return_stock(conn: &Connection, workspace_id: i64, order_id: i64) -> Result<()> {
    let mut quantities: Vec<(i64, i64)> = Vec::new();
    for item in order_repo::list_items(conn, workspace_id, order_id)? {
        if let Some(product) = item.product_id {
            match quantities.iter_mut().find(|(p, _)| *p == product) {
                Some((_, q)) => *q += item.stock_taken,
                None => quantities.push((product, item.stock_taken)),
            }
        }
    }
    let ids: Vec<i64> = quantities.iter().map(|(p, _)| *p).collect();
    for row in product_repo::lock_for_stock(conn, workspace_id, &ids)? {
        let quantity = quantities
            .iter()
            .find(|(p, _)| *p == row.id)
            .map_or(0, |q| q.1);
        product_repo::restore_inventory(conn, workspace_id, row.id, quantity)?;
    }
    Ok(())
}

/// `nextOrderNumber`: the workspace's counter, one up.
fn next_order_number(conn: &Connection, workspace_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        r#"INSERT INTO "EcommerceOrderCounter" ("workspaceId", "lastNumber") VALUES (?1, 1)
           ON CONFLICT ("workspaceId") DO UPDATE SET "lastNumber" = "lastNumber" + 1 RETURNING "lastNumber""#,
        params![workspace_id],
        |r| r.get(0),
    )?)
}

fn text(value: Option<&str>) -> Sql {
    value.map_or(Sql::Null, |v| Sql::Text(v.to_string()))
}

fn int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

fn insert_history(
    conn: &Connection,
    workspace_id: i64,
    order_id: i64,
    from: Option<&str>,
    to: &str,
    actor: Actor,
    note: Option<&str>,
) -> Result<()> {
    order_repo::insert_history(
        conn,
        &vec![
            ("orderId", Sql::Integer(order_id)),
            ("workspaceId", Sql::Integer(workspace_id)),
            ("fromStatus", text(from)),
            ("toStatus", Sql::Text(to.to_string())),
            ("actorType", Sql::Text(actor.kind.to_string())),
            ("actorUserId", int(actor.user_id)),
            ("note", text(note)),
        ],
    )?;
    Ok(())
}

/// `resolveConversation`: an order for a contact is tied to its latest
/// conversation (a shop link's channel when it names one).
pub async fn resolve_conversation(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    contact: Option<ContactRef>,
) -> Result<Option<ContactRef>> {
    let Some(contact) = contact else {
        return Ok(None);
    };
    let Some(contact_id) = contact
        .contact_id
        .filter(|_| contact.conversation_id.is_none())
    else {
        return Ok(Some(contact));
    };
    match crate::conversation::resolve_contact_inbox_for_send(
        db,
        workspace_id,
        contact_id,
        contact.inbox_id,
    )
    .await
    {
        Ok((found, contact_inbox)) => {
            let chosen = contact
                .contact_inbox_id
                .and_then(|id| found.contact_inboxes.iter().find(|c| c.id == id))
                .unwrap_or(&contact_inbox);
            Ok(Some(ContactRef {
                contact_id: Some(contact_id),
                contact_inbox_id: Some(chosen.id),
                inbox_id: Some(chosen.inbox_id),
                conversation_id: Some(found.conversation.id),
                channel: Some(chosen.channel.clone()),
            }))
        }
        Err(error) if error.code() == Some("notFound") => Ok(Some(contact)),
        Err(error) => Err(error),
    }
}

/// What `place` hands back for its caller's own work inside the order's
/// transaction (the web shop empties its cart there).
pub type OnPlaced = Box<dyn FnOnce(&Connection, &Order) -> Result<()> + Send>;

/// `place`: prices, stock, number and discount in one transaction.
pub async fn place(
    deps: Deps<'_>,
    input: PlaceInput,
    on_placed: Option<OnPlaced>,
) -> Result<OrderDetail> {
    let items = merge_items(&input.items);
    if items.is_empty() {
        return Err(exception(
            "An order needs at least one item.",
            "orderEmpty",
            400,
        ));
    }
    let contact = resolve_conversation(deps.db, input.workspace_id, input.contact.clone()).await?;
    let order = tx::write(deps.db, move |tx| {
        let conn: &Connection = tx;
        let ws = input.workspace_id;
        let contact = contact.unwrap_or_default();
        let draft = price_draft(
            conn,
            ws,
            &items,
            input.shipping_type.as_deref(),
            contact.contact_id,
            input.discount_code.as_deref(),
            input.auto_apply_discount,
            None,
        )?;
        let settings = &draft.settings;
        if input.source == "webshop" && settings["general"]["orderingAvailable"] == false {
            return Err(exception(
                "The store is not taking orders right now.",
                "orderingUnavailable",
                400,
            ));
        }
        if input.source != "manual" {
            let minimum = settings["checkout"]["minimumOrderValue"]
                .as_f64()
                .unwrap_or(0.0);
            if minimum > 0.0
                && goods_after_discounts(&draft.totals, draft.discount.as_ref(), &draft.currency)
                    < minimum
            {
                return Err(exception(
                    format!(
                        "The order total is below the store minimum of {}.",
                        crate::payments::js_number(minimum)
                    ),
                    "orderBelowMinimum",
                    400,
                )
                .with_data(json!({ "minimum": number(minimum) })));
            }
            assert_vendor_minimums(&draft.lines, &draft.products, &draft.currency)?;
        }
        // `resolvePayment`.
        let method = input.payment_method.clone().unwrap_or_else(|| "cod".into());
        let online = is_online(&method);
        let offered = settings["payments"]["paymentMethods"]
            .as_array()
            .is_some_and(|m| m.iter().any(|x| x == method.as_str()));
        if !offered && (online || input.source == "webshop") {
            return Err(method_unavailable(&method));
        }
        let now = repo::now_ms();
        let (payment_status, expires_at, paid_at) = if !online {
            ("unpaid", None, None)
        } else if contact.contact_id.is_none() {
            return Err(payment_contact_required());
        } else if draft.totals.total <= 0.0 {
            ("paid", None, Some(now))
        } else {
            ("unpaid", Some(now + PAYMENT_EXPIRY_MINUTES * 60_000), None)
        };

        let taken_by_product = take_stock(conn, ws, &draft.lines, &draft.products)?;
        let mut remaining = taken_by_product;
        let taken_by_line: Vec<i64> = draft
            .lines
            .iter()
            .map(|line| {
                let available = remaining.get(&line.product_id).copied().unwrap_or(0);
                let taken = line.quantity.min(available);
                remaining.insert(line.product_id, available - taken);
                taken
            })
            .collect();
        let number = next_order_number(conn, ws)?;
        let t = draft.totals;
        let delivery = draft.shipping_type.as_deref() == Some("delivery");
        let values: Patch = vec![
            ("workspaceId", Sql::Integer(ws)),
            ("number", Sql::Integer(number)),
            ("contactId", int(contact.contact_id)),
            ("contactInboxId", int(contact.contact_inbox_id)),
            ("inboxId", int(contact.inbox_id)),
            ("conversationId", int(contact.conversation_id)),
            ("channel", text(contact.channel.as_deref())),
            ("source", Sql::Text(input.source.to_string())),
            ("paymentMethod", Sql::Text(method.clone())),
            ("paymentStatus", Sql::Text(payment_status.to_string())),
            ("paymentExpiresAt", int(expires_at)),
            ("paidAt", int(paid_at)),
            ("currency", Sql::Text(draft.currency.clone())),
            ("subtotal", Sql::Integer(scaled(t.subtotal))),
            ("discountTotal", Sql::Integer(scaled(t.discount_total))),
            ("shippingTotal", Sql::Integer(scaled(t.shipping_total))),
            ("serviceFeeTotal", Sql::Integer(scaled(t.service_fee_total))),
            ("taxTotal", Sql::Integer(scaled(t.tax_total))),
            ("couponDiscount", Sql::Integer(scaled(t.coupon_discount))),
            ("total", Sql::Integer(scaled(t.total))),
            ("discountCodeId", int(draft.discount.as_ref().map(|d| d.id))),
            (
                "discountCode",
                text(draft.discount.as_ref().map(|d| d.code.as_str())),
            ),
            ("customerName", text(input.customer_name.as_deref())),
            ("customerEmail", text(input.customer_email.as_deref())),
            ("customerPhone", text(input.customer_phone.as_deref())),
            ("shippingType", text(draft.shipping_type.as_deref())),
            (
                "shippingAddress",
                match (&input.shipping_address, delivery) {
                    (Some(a), true) if !a.is_null() => Sql::Text(a.to_string()),
                    _ => Sql::Null,
                },
            ),
            ("note", text(input.note.as_deref())),
        ];
        let created = order_repo::insert(conn, &values)?
            .ok_or_else(|| BusinessError::Repo(repo::Error::msg("Failed to create the order")))?;
        if let Some(applied) = &draft.discount {
            discount_code::redeem(conn, ws, created.id, created.contact_id, applied)?;
        }
        let rows: Vec<Patch> = draft
            .lines
            .iter()
            .zip(&taken_by_line)
            .map(|(l, taken)| {
                vec![
                    ("productId", Sql::Integer(l.product_id)),
                    ("variantId", int(l.variant_id)),
                    ("vendorId", int(l.vendor_id)),
                    ("categoryId", int(l.category_id)),
                    ("name", Sql::Text(l.name.clone())),
                    ("variantLabel", text(l.variant_label.as_deref())),
                    (
                        "combination",
                        l.combination
                            .as_ref()
                            .map_or(Sql::Null, |c| Sql::Text(c.to_string())),
                    ),
                    ("sku", text(l.sku.as_deref())),
                    ("retailerId", Sql::Null),
                    ("imageUrl", text(l.image_url.as_deref())),
                    ("vendorName", text(l.vendor_name.as_deref())),
                    ("unitPrice", Sql::Integer(scaled(l.unit_price))),
                    ("quantity", Sql::Integer(l.quantity)),
                    ("stockTaken", Sql::Integer(*taken)),
                    ("discountPercent", Sql::Integer(scaled(l.discount_percent))),
                    ("taxPercent", Sql::Integer(scaled(l.tax_percent))),
                    ("discountTotal", Sql::Integer(scaled(l.discount_total))),
                    ("taxTotal", Sql::Integer(scaled(l.tax_total))),
                    ("lineTotal", Sql::Integer(scaled(l.line_total))),
                ]
            })
            .collect();
        order_repo::insert_items(conn, ws, created.id, &rows)?;
        let actor = Actor {
            kind: input.actor_kind,
            user_id: input.actor_user_id,
        };
        insert_history(conn, ws, created.id, None, &created.status, actor, None)?;
        if let Some(on_placed) = on_placed {
            on_placed(conn, &created)?;
        }
        Ok(created)
    })
    .await?;
    effects::after_status_change(deps, &order, None, None, false, false).await;
    get_detail(deps.db, order.workspace_id, order.id).await
}

// ---- transition / delete --------------------------------------------------------

/// `transition`: one guarded move along the allowed statuses. A cancel
/// settles the online payment first (close, refund) and gives the stock back.
pub async fn transition(
    deps: Deps<'_>,
    workspace_id: i64,
    order_id: i64,
    to: &'static str,
    actor: Actor,
    message: Option<&str>,
    payment_expired: bool,
) -> Result<OrderDetail> {
    let message = message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let _guard = order_payment::lock_order(workspace_id, order_id).await?;
    let current = tx::read(deps.db, move |conn| {
        order_repo::find_for_update(conn, workspace_id, order_id)?.ok_or_else(not_found)
    })
    .await?;
    let check = |current: &Order| -> Result<()> {
        if is_external_store_source(&current.source) {
            return Err(external_store_managed(&current.source));
        }
        if !allowed_before(to).contains(&current.status) {
            return Err(invalid_transition(&current.status, to));
        }
        if to == "accepted" && is_awaiting_online_payment(current) {
            return Err(exception(
                "The order is waiting for its online payment.",
                "orderAwaitingPayment",
                409,
            ));
        }
        Ok(())
    };
    check(&current)?;
    let refunded = if to == "cancelled" {
        order_payment::settle_for_cancel(deps, &current, payment_expired).await?
    } else {
        false
    };
    let note = message.clone();
    let (order, from) = tx::write(deps.db, move |tx| {
        let conn: &Connection = tx;
        let current =
            order_repo::find_for_update(conn, workspace_id, order_id)?.ok_or_else(not_found)?;
        let now = repo::now_ms();
        let column = match to {
            "accepted" => "acceptedAt",
            "shipped" => "shippedAt",
            "concluded" => "concludedAt",
            _ => "cancelledAt",
        };
        let mut values: Patch = vec![(column, Sql::Integer(now))];
        if to == "concluded" && current.payment_method == "cod" {
            values.push(("paymentStatus", Sql::Text("paid".into())));
        }
        if to == "cancelled" {
            if refunded {
                values.push(("paymentStatus", Sql::Text("refunded".into())));
            }
            values.push(("paymentExpiresAt", Sql::Null));
            let reason = note
                .clone()
                .or_else(|| payment_expired.then(|| PAYMENT_EXPIRED_CANCEL_REASON.to_string()));
            values.push(("cancelReason", text(reason.as_deref())));
        }
        let Some(updated) = order_repo::transition_status(
            conn,
            workspace_id,
            order_id,
            &allowed_before(to),
            to,
            &values,
        )?
        else {
            return Err(invalid_transition(&current.status, to));
        };
        if to == "cancelled" {
            discount_code::release(conn, workspace_id, order_id)?;
            return_stock(conn, workspace_id, order_id)?;
        }
        insert_history(
            conn,
            workspace_id,
            order_id,
            Some(&current.status),
            to,
            actor,
            note.as_deref(),
        )?;
        Ok((updated, current.status))
    })
    .await?;
    effects::after_status_change(
        deps,
        &order,
        Some(&from),
        message.as_deref(),
        refunded,
        payment_expired,
    )
    .await;
    get_detail(deps.db, workspace_id, order_id).await
}

/// `delete`: a paid online order is cancelled (refunded), never deleted.
pub async fn delete(deps: Deps<'_>, workspace_id: i64, order_id: i64) -> Result<()> {
    let _guard = order_payment::lock_order(workspace_id, order_id).await?;
    let current = tx::read(deps.db, move |conn| {
        order_repo::find_for_update(conn, workspace_id, order_id)?.ok_or_else(not_found)
    })
    .await?;
    if is_external_store_source(&current.source) {
        return Err(external_store_managed(&current.source));
    }
    order_payment::assert_deletable(deps, &current).await?;
    tx::write(deps.db, move |tx| {
        let conn: &Connection = tx;
        let current =
            order_repo::find_for_update(conn, workspace_id, order_id)?.ok_or_else(not_found)?;
        discount_code::release(conn, workspace_id, order_id)?;
        if STOCK_HOLDING.contains(&current.status.as_str()) {
            return_stock(conn, workspace_id, order_id)?;
        }
        order_repo::delete(conn, workspace_id, order_id)?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_are_quoted_and_defused() {
        assert_eq!(csv_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_cell("+15550002222"), "'+15550002222");
        assert_eq!(csv_cell("Lovelace, Ada"), "\"Lovelace, Ada\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }

    #[test]
    fn statuses_allowed_before_a_move() {
        assert_eq!(allowed_before("accepted"), ["pending"]);
        assert_eq!(allowed_before("concluded"), ["accepted", "shipped"]);
        assert_eq!(
            allowed_before("cancelled"),
            ["pending", "accepted", "shipped"]
        );
        assert!(allowed_before("pending").is_empty());
    }

    #[test]
    fn items_merge_by_product_and_variant() {
        let item = |p, v, q| PlaceItem {
            product_id: p,
            variant_id: v,
            quantity: q,
        };
        assert_eq!(
            merge_items(&[item(1, None, 1), item(1, None, 2), item(1, Some(5), 1)]),
            vec![item(1, None, 3), item(1, Some(5), 1)]
        );
    }

    #[test]
    fn totals_take_the_coupon_off_the_goods_before_the_fee() {
        let line = Line {
            product_id: 1,
            variant_id: None,
            vendor_id: None,
            category_id: None,
            name: "x".into(),
            variant_label: None,
            combination: None,
            sku: None,
            image_url: None,
            vendor_name: None,
            unit_price: 10.0,
            quantity: 2,
            discount_percent: 0.0,
            tax_percent: 0.0,
            discount_total: 0.0,
            tax_total: 0.0,
            line_total: 20.0,
            gross: 20.0,
        };
        let t = compute_totals(&[line.clone()], "USD", 5.0, 10.0, Some((4.0, false)));
        assert_eq!(
            (t.subtotal, t.service_fee_total, t.total),
            (20.0, 1.6, 22.6)
        );
        let free = compute_totals(&[line], "USD", 5.0, 10.0, Some((5.0, true)));
        assert_eq!((free.service_fee_total, free.total), (2.0, 22.0));
    }
}
