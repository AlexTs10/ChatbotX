//! Q.D8 slice 6b: the public web shop (`app/shop/[workspaceId]/*`). The data
//! of the five server pages as page loaders (`POST /rpc/pages/shop*`,
//! `{ json: { workspaceId, token, ... } }`) and the six public actions of
//! `features/webshop/actions/webshop.action.ts` (`actionClient`: no session;
//! the signed shop token is the authority, `runShopAction`).
//!
//! - A bad, expired or foreign token: the loaders answer `null` (the page
//!   shows "invalid link"), the actions `{ ok: false, error: invalidLink }`.
//! - The actions keep the original limit (`checkGuestRateLimit`, key
//!   `shop-{workspaceId}`, the contact or `preview-{workspaceId}` as the
//!   session); the loaders use the guest per-IP limiter (key `shop`, as the
//!   booking webview; the original pages have none).
//! - A preview link's cart is an httpOnly cookie (`preview-cart.ts`); it is
//!   read back through the same checks and every price comes from the server.
//! - The answers are JSON for the SPA (React escapes the text).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_business::BusinessError;
use chatbotx_business::order::PlaceItem;
use chatbotx_business::tx;
use chatbotx_business::webshop::{self, CheckoutForm, MAX_PREVIEW_CART_LINES, Mode, Shopper};
use chatbotx_core::encryption::ShopTokenPayload;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::product::ShopListInput;
use serde_json::{Map, Value, json};

use crate::handlers::ApiState;
use crate::handlers::orders::deps;
use crate::rpc::rpc_error;

const PER_PAGE: i64 = 24;
/// Deeper than any real catalog; a huge `?page=` would make a costly OFFSET.
const MAX_SHOP_PAGE: i64 = 1000;
/// A bot's search hits; a link with more is not one the shop made.
const MAX_LINKED_PRODUCTS: usize = 20;
const PREVIEW_CART_MAX_AGE_SECONDS: i64 = 24 * 60 * 60;
const DEFAULT_SERVER_ERROR: &str = "Something went wrong while executing the operation.";

/// The routes (merged before the `/rpc/{*rest}` router: static paths win).
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/rpc/pages/shopHome", post(home_page))
        .route("/rpc/pages/shopCategories", post(categories_page))
        .route("/rpc/pages/shopProduct", post(product_page))
        .route("/rpc/pages/shopCart", post(cart_page))
        .route("/rpc/pages/shopOrder", post(order_page))
        .route("/rpc/actions/changeCartAction", post(change_cart))
        .route("/rpc/actions/checkoutAction", post(checkout))
        .route("/rpc/actions/payOrderAction", post(pay_order))
        .route("/rpc/actions/rateOrderAction", post(rate_order))
        .route("/rpc/actions/applyDiscountCodeAction", post(apply_code))
        .route("/rpc/actions/removeDiscountCodeAction", post(remove_code))
        .with_state(state)
}

fn ok(value: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "json": value, "meta": [] }).to_string(),
    )
        .into_response()
}

fn server_error() -> Response {
    rpc_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_SERVER_ERROR",
        DEFAULT_SERVER_ERROR,
    )
}

fn invalid_input() -> Response {
    rpc_error(
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
        "Input validation failed",
    )
}

fn body_json(body: &Bytes) -> Value {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null)
}

fn too_many(retry_after: i64) -> Response {
    let mut response = rpc_error(
        StatusCode::TOO_MANY_REQUESTS,
        "TOO_MANY_REQUESTS",
        "Too many requests",
    );
    if let Ok(value) = retry_after.to_string().parse() {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

// ---- session ------------------------------------------------------------------------

/// Who is shopping (`resolveShopSession`).
#[derive(Debug, Clone, Copy)]
enum Session {
    Customer(Shopper),
    Preview(i64),
}

impl Session {
    fn workspace_id(&self) -> i64 {
        match self {
            Session::Customer(s) => s.workspace_id,
            Session::Preview(ws) => *ws,
        }
    }

    fn json(&self) -> Value {
        json!({ "mode": match self { Session::Customer(_) => "customer", Session::Preview(_) => "preview" } })
    }
}

/// A bigint id: 1 to 20 digits (`asId`, `zodBigintAsString`).
fn as_id(value: &Value) -> Option<i64> {
    let text = match value {
        Value::String(s) => s.as_str(),
        _ => return None,
    };
    (!text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// `null` for a missing, forged or expired token, one signed for another
/// workspace than the URL's, or an unresolved `{{shop_token}}`.
fn resolve_session(state: &ApiState, input: &Value) -> Option<Session> {
    let workspace = input["workspaceId"].as_str()?;
    let workspace_id = as_id(&input["workspaceId"])?;
    let token = input["token"]
        .as_str()
        .filter(|t| !t.is_empty() && t.len() <= 4096 && !t.starts_with("{{"))?;
    let payload = state.encryption.as_deref()?.verify_shop_token(token).ok()?;
    match payload {
        ShopTokenPayload::Preview {
            workspace_id: ws, ..
        } if ws == workspace => Some(Session::Preview(workspace_id)),
        ShopTokenPayload::Customer {
            workspace_id: ws,
            contact_id,
            contact_inbox_id,
            conversation_id,
            ..
        } if ws == workspace => Some(Session::Customer(Shopper {
            workspace_id,
            contact_id: contact_id.parse().ok()?,
            contact_inbox_id: contact_inbox_id.and_then(|v| v.parse().ok()),
            conversation_id: conversation_id.and_then(|v| v.parse().ok()),
        })),
        _ => None,
    }
}

// ---- preview cart cookie ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct PreviewCart {
    items: Vec<PlaceItem>,
    discount_code: Option<String>,
}

fn cookie_name(workspace_id: i64) -> String {
    format!("shop-preview-cart-{workspace_id}")
}

/// `readPreviewCart`: empty when there is none or it is unreadable (the
/// shape of `previewCartSchema`).
fn read_preview_cart(headers: &HeaderMap, workspace_id: i64) -> PreviewCart {
    let name = cookie_name(workspace_id);
    let raw = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string());
    let parsed = raw
        .and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let Some(value) = parsed else {
        return PreviewCart::default();
    };
    let Some(lines) = value["items"].as_array() else {
        return PreviewCart::default();
    };
    let mut items = Vec::new();
    for line in lines.iter().take(MAX_PREVIEW_CART_LINES + 1) {
        let quantity = line["quantity"]
            .as_i64()
            .filter(|q| (1..=10_000).contains(q));
        let (Some(product_id), Some(quantity)) = (as_id(&line["productId"]), quantity) else {
            return PreviewCart::default();
        };
        let variant_id = match &line["variantId"] {
            Value::Null => None,
            other => match as_id(other) {
                Some(id) => Some(id),
                None => return PreviewCart::default(),
            },
        };
        items.push(PlaceItem {
            product_id,
            variant_id,
            quantity,
        });
    }
    if items.len() > MAX_PREVIEW_CART_LINES {
        return PreviewCart::default();
    }
    let discount_code = value["discountCode"]
        .as_str()
        .filter(|c| c.len() <= 64)
        .map(str::to_string);
    PreviewCart {
        items,
        discount_code,
    }
}

/// `writePreviewCart`: the `Set-Cookie` of a preview action's answer.
fn preview_cookie(state: &ApiState, workspace_id: i64, cart: &PreviewCart) -> Option<HeaderValue> {
    let value = json!({
        "items": webshop::lines_json(&cart.items),
        "discountCode": cart.discount_code,
    });
    let secure = if state.app_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    // ponytail: the actions post to `/rpc`, so the cookie lives there (the
    // original's `/shop/{id}` is the page path of a Next server action).
    format!(
        "{}={}; Path=/rpc; Max-Age={PREVIEW_CART_MAX_AGE_SECONDS}; HttpOnly; SameSite=Lax{secure}",
        cookie_name(workspace_id),
        URL_SAFE_NO_PAD.encode(value.to_string())
    )
    .parse()
    .ok()
}

// ---- page loaders ------------------------------------------------------------------------

/// The guest per-IP limit of a page load: `None` = allowed.
fn page_limited(state: &ApiState, headers: &HeaderMap) -> Option<Response> {
    let ip = crate::auth::guest_client_ip(headers);
    let limit = crate::reflink_widget::check_guest_rate_limit(state, "shop", &ip, None, now_ms());
    limit.limited.then(|| too_many(limit.retry_after))
}

/// `loadShopPage`: the store, who is shopping and the cart badge.
async fn load_shop(
    state: &ApiState,
    input: &Value,
    headers: &HeaderMap,
) -> Option<(Session, Value)> {
    let session = resolve_session(state, input)?;
    let ws = session.workspace_id();
    let shop = webshop::storefront(&state.db, ws).await.ok()?;
    let cart_count = match session {
        Session::Customer(s) => tx::read(&state.db, move |conn| {
            webshop::count_cart_items(conn, ws, s.contact_id)
        })
        .await
        .ok()?,
        Session::Preview(_) => webshop::count_items(&read_preview_cart(headers, ws).items),
    };
    Some((
        session,
        json!({ "shop": shop, "session": session.json(), "cartCount": cart_count }),
    ))
}

macro_rules! page_prelude {
    ($state:ident, $headers:ident, $body:ident) => {{
        if let Some(response) = page_limited(&$state, &$headers) {
            return response;
        }
        let input = body_json(&$body);
        let Some((session, page)) = load_shop(&$state, &input, &$headers).await else {
            return ok(Value::Null);
        };
        (input, session, page)
    }};
}

/// `shop/[workspaceId]/page.tsx`: products, searchable, by category or vendor.
async fn home_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session, mut page) = page_prelude!(state, headers, body);
    let keyword: String = input["q"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take(255)
        .collect();
    let product_ids: Vec<i64> = input["products"]
        .as_str()
        .unwrap_or("")
        .split(',')
        .filter_map(|part| as_id(&Value::from(part.trim())))
        .take(MAX_LINKED_PRODUCTS)
        .collect();
    let number = input["page"]
        .as_str()
        .and_then(|p| p.trim().parse::<f64>().ok())
        .map_or(1, |p| p.trunc() as i64)
        .clamp(1, MAX_SHOP_PAGE);
    let list = ShopListInput {
        workspace_id: session.workspace_id(),
        keyword: Some(keyword.clone()),
        category_id: as_id(&input["category"]),
        vendor_id: as_id(&input["vendor"]),
        product_ids: (!product_ids.is_empty()).then_some(product_ids),
        page: Some(number),
        per_page: Some(PER_PAGE),
        ..ShopListInput::default()
    };
    let (category, vendor) = (list.category_id, list.vendor_id);
    let Ok(products) = tx::read(&state.db, move |conn| webshop::list_products(conn, list)).await
    else {
        return server_error();
    };
    page["products"] = products;
    page["query"] = json!({
        "keyword": keyword,
        "categoryId": category.map(|c| c.to_string()),
        "vendorId": vendor.map(|v| v.to_string()),
        "page": number,
    });
    ok(page)
}

/// `categories/page.tsx`.
async fn categories_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (_, _, page) = page_prelude!(state, headers, body);
    ok(page)
}

/// `products/[productId]/page.tsx`: the product (`null` when the shop does
/// not show it) and "You may also like".
async fn product_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session, mut page) = page_prelude!(state, headers, body);
    let ws = session.workspace_id();
    let detail = match as_id(&input["productId"]) {
        Some(id) => tx::read(&state.db, move |conn| webshop::product_page(conn, ws, id)).await,
        None => Ok(Value::Null),
    };
    let Ok(detail) = detail else {
        return server_error();
    };
    page["product"] = detail["product"].clone();
    page["related"] = detail.get("related").cloned().unwrap_or_else(|| json!([]));
    ok(page)
}

/// `cart/page.tsx`: the cart priced for the delivery choice in the URL.
async fn cart_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session, mut page) = page_prelude!(state, headers, body);
    let offered: &[&str] = match page["shop"]["shipping"]["mode"].as_str() {
        Some("delivery") => &["delivery"],
        Some("pickup") => &["pickup"],
        Some("deliveryAndPickup") => &["delivery", "pickup"],
        _ => &[],
    };
    let requested = input["shipping"].as_str();
    let shipping = offered
        .iter()
        .find(|t| Some(**t) == requested)
        .or_else(|| offered.first())
        .map(|t| (*t).to_string());
    let ws = session.workspace_id();
    let (shopper, preview) = match session {
        Session::Customer(s) => (Some(s), (Vec::new(), None)),
        Session::Preview(_) => {
            let cart = read_preview_cart(&headers, ws);
            (None, (cart.items, cart.discount_code))
        }
    };
    match webshop::cart_page(&state.db, ws, shopper, preview, shipping).await {
        Ok(cart) => {
            page["cart"] = cart;
            ok(page)
        }
        Err(_) => server_error(),
    }
}

/// `orders/[orderId]/page.tsx`: only the customer's own order (`null`
/// otherwise). Back from the provider's page (`payment=return`) the payment
/// is read back first.
async fn order_page(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session, mut page) = page_prelude!(state, headers, body);
    let returned = input["payment"].as_str() == Some("return");
    let order = match (session, as_id(&input["orderId"])) {
        (Session::Customer(shopper), Some(order_id)) => {
            let found = if returned {
                webshop::refresh_customer_order_payment(deps(&state), shopper, order_id).await
            } else {
                webshop::customer_order(&state.db, shopper, order_id).await
            };
            match found {
                Ok(detail) => webshop::customer_order_json(&detail),
                Err(error) if error.code() == Some("notFound") => Value::Null,
                Err(_) => return server_error(),
            }
        }
        _ => Value::Null,
    };
    page["order"] = order;
    ok(page)
}

// ---- actions -----------------------------------------------------------------------------

/// `runShopAction`'s refusal.
fn fail(code: &str) -> Response {
    ok(json!({ "ok": false, "error": { "code": code } }))
}

/// `runReportingRefusals`: a business refusal is its error code; anything
/// else is the default server error.
fn reported(result: Result<Value, BusinessError>) -> Response {
    match result {
        Ok(data) => ok(json!({ "ok": true, "data": data })),
        Err(BusinessError::Exception {
            code,
            status,
            field,
            data,
            ..
        }) if status < 500 => {
            let mut error = Map::new();
            error.insert("code".into(), json!(code));
            if let Some(field) = field {
                error.insert("field".into(), json!(field));
            }
            if let Some(data) = data {
                error.insert("data".into(), data);
            }
            ok(json!({ "ok": false, "error": error }))
        }
        Err(error) => {
            tracing::error!(%error, "[webshop] action failed");
            server_error()
        }
    }
}

/// The action's input (`args[0]`), its session, and whether the caller is
/// within `checkGuestRateLimit` (`shop-{ws}`, the contact or the preview as
/// the session, the IP or else the session as the client key).
enum Gate {
    Go(Value, Session),
    Stop(Response),
}

fn gate(state: &ApiState, headers: &HeaderMap, body: &Bytes) -> Gate {
    let input = body_json(body)["args"][0].clone();
    if !input.is_object() {
        return Gate::Stop(invalid_input());
    }
    let Some(session) = resolve_session(state, &input) else {
        return Gate::Stop(fail("invalidLink"));
    };
    let ws = session.workspace_id();
    let key = match session {
        Session::Customer(s) => s.contact_id.to_string(),
        Session::Preview(_) => format!("preview-{ws}"),
    };
    let ip = crate::auth::guest_client_ip(headers);
    let client = if ip == crate::auth::UNKNOWN_CLIENT_IP {
        key.clone()
    } else {
        ip
    };
    let limit = crate::reflink_widget::check_guest_rate_limit(
        state,
        &format!("shop-{ws}"),
        &client,
        Some(&key),
        now_ms(),
    );
    if limit.limited {
        return Gate::Stop(fail("rateLimited"));
    }
    Gate::Go(input, session)
}

macro_rules! action_prelude {
    ($state:ident, $headers:ident, $body:ident) => {
        match gate(&$state, &$headers, &$body) {
            Gate::Go(input, session) => (input, session),
            Gate::Stop(response) => return response,
        }
    };
}

/// An optional trimmed string of at most `max` characters; `Err` = invalid.
fn opt_text(input: &Value, key: &str, max: usize) -> Result<Option<String>, ()> {
    match &input[key] {
        Value::Null => Ok(None),
        Value::String(s) => {
            let t = s.trim();
            (t.chars().count() <= max)
                .then(|| Some(t.to_string()))
                .ok_or(())
        }
        _ => Err(()),
    }
}

fn opt_enum(input: &Value, key: &str, options: &[&str]) -> Result<Option<String>, ()> {
    match &input[key] {
        Value::Null => Ok(None),
        Value::String(s) if options.contains(&s.as_str()) => Ok(Some(s.clone())),
        _ => Err(()),
    }
}

const SHIPPING_TYPES: [&str; 2] = ["delivery", "pickup"];

/// `changeCartAction`: adds to a line or sets its quantity; `{ itemCount }`.
async fn change_cart(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session) = action_prelude!(state, headers, body);
    let mode = match input["mode"].as_str() {
        Some("add") => Mode::Add,
        Some("set") => Mode::Set,
        _ => return invalid_input(),
    };
    let variant_id = match &input["variantId"] {
        Value::Null => None,
        other => match as_id(other) {
            Some(id) => Some(id),
            None => return invalid_input(),
        },
    };
    let (Some(product_id), Some(quantity)) = (
        as_id(&input["productId"]),
        input["quantity"]
            .as_i64()
            .filter(|q| (0..=10_000).contains(q)),
    ) else {
        return invalid_input();
    };
    let change = PlaceItem {
        product_id,
        variant_id,
        quantity,
    };
    match session {
        Session::Customer(shopper) => reported(
            webshop::change_cart(deps(&state), shopper, change, mode)
                .await
                .map(|count| json!({ "itemCount": count })),
        ),
        Session::Preview(ws) => {
            let mut cart = read_preview_cart(&headers, ws);
            match webshop::change_preview_cart(&state.db, ws, cart.items.clone(), change, mode)
                .await
            {
                Ok(items) => {
                    cart.items = items;
                    with_cookie(
                        &state,
                        ws,
                        &cart,
                        json!({ "itemCount": webshop::count_items(&cart.items) }),
                    )
                }
                Err(error) => reported(Err(error)),
            }
        }
    }
}

fn with_cookie(state: &ApiState, ws: i64, cart: &PreviewCart, data: Value) -> Response {
    let mut response = ok(json!({ "ok": true, "data": data }));
    if let Some(cookie) = preview_cookie(state, ws, cart) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.starts_with('.')
        && domain.contains('.')
        && !domain.ends_with('.')
        && !text.contains(char::is_whitespace)
        && !domain.contains('@')
}

/// `orderShippingAddressSchema`.
fn parse_address(value: &Value) -> Result<Option<Value>, ()> {
    let obj = match value {
        Value::Null => return Ok(None),
        Value::Object(obj) => obj,
        _ => return Err(()),
    };
    let mut out = Map::new();
    for (field, max) in [
        ("line1", 500),
        ("line2", 500),
        ("city", 255),
        ("state", 255),
        ("zip", 32),
        ("country", 2),
    ] {
        if obj.contains_key(field) {
            out.insert(field.into(), json!(opt_text(value, field, max)?));
        }
    }
    Ok(Some(Value::Object(out)))
}

fn checkout_form(input: &Value) -> Result<CheckoutForm, ()> {
    let email = opt_text(input, "customerEmail", 255)?;
    if email
        .as_deref()
        .is_some_and(|e| !e.is_empty() && !is_email(e))
    {
        return Err(());
    }
    Ok(CheckoutForm {
        payment_method: opt_enum(input, "paymentMethod", &["cod", "stripe", "paypal"])?,
        customer_name: opt_text(input, "customerName", 255)?,
        // `customerEmail || null`.
        customer_email: email.filter(|e| !e.is_empty()),
        customer_phone: opt_text(input, "customerPhone", 64)?,
        shipping_type: opt_enum(input, "shippingType", &SHIPPING_TYPES)?,
        shipping_address: parse_address(&input["shippingAddress"])?,
        note: opt_text(input, "note", 2000)?,
    })
}

/// `checkoutAction`: `{ orderId, total, currency, paymentUrl }`; a preview
/// link cannot order.
async fn checkout(State(state): State<Arc<ApiState>>, headers: HeaderMap, body: Bytes) -> Response {
    let (input, session) = action_prelude!(state, headers, body);
    let Ok(form) = checkout_form(&input) else {
        return invalid_input();
    };
    let Session::Customer(shopper) = session else {
        return fail("webshopPreviewOnly");
    };
    reported(webshop::checkout(deps(&state), shopper, form).await)
}

/// `payOrderAction`: the provider's page for the shopper's own order.
async fn pay_order(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session) = action_prelude!(state, headers, body);
    let Some(order_id) = as_id(&input["orderId"]) else {
        return invalid_input();
    };
    let Session::Customer(shopper) = session else {
        return fail("webshopPreviewOnly");
    };
    reported(
        webshop::pay_customer_order(deps(&state), shopper, order_id)
            .await
            .map(|url| json!({ "paymentUrl": url })),
    )
}

/// `rateOrderAction`.
async fn rate_order(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session) = action_prelude!(state, headers, body);
    let (Some(order_id), Some(rating), Ok(comment)) = (
        as_id(&input["orderId"]),
        input["rating"].as_i64().filter(|r| (1..=5).contains(r)),
        opt_text(&input, "comment", 2000),
    ) else {
        return invalid_input();
    };
    let Session::Customer(shopper) = session else {
        return fail("webshopPreviewOnly");
    };
    reported(webshop::rate_order(&state.db, shopper, order_id, rating, comment).await)
}

/// `applyDiscountCodeAction`: `{ discountError }`; a usable code is kept.
async fn apply_code(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, session) = action_prelude!(state, headers, body);
    let (Ok(Some(code)), Ok(shipping)) = (
        opt_text(&input, "code", 64),
        opt_enum(&input, "shippingType", &SHIPPING_TYPES),
    ) else {
        return invalid_input();
    };
    if code.is_empty() {
        return invalid_input();
    }
    let ws = session.workspace_id();
    match session {
        Session::Customer(shopper) => reported(
            webshop::apply_discount_code(&state.db, ws, Some(shopper), Vec::new(), &code, shipping)
                .await
                .map(|error| json!({ "discountError": error })),
        ),
        Session::Preview(_) => {
            let mut cart = read_preview_cart(&headers, ws);
            match webshop::apply_discount_code(
                &state.db,
                ws,
                None,
                cart.items.clone(),
                &code,
                shipping,
            )
            .await
            {
                Ok(error) if error.is_null() => {
                    cart.discount_code = Some(code);
                    with_cookie(&state, ws, &cart, json!({ "discountError": null }))
                }
                Ok(error) => ok(json!({ "ok": true, "data": { "discountError": error } })),
                Err(error) => reported(Err(error)),
            }
        }
    }
}

/// `removeDiscountCodeAction`.
async fn remove_code(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (_, session) = action_prelude!(state, headers, body);
    match session {
        Session::Customer(shopper) => reported(
            webshop::remove_discount_code(&state.db, shopper)
                .await
                .map(|()| json!({ "removed": true })),
        ),
        Session::Preview(ws) => {
            let mut cart = read_preview_cart(&headers, ws);
            cart.discount_code = None;
            with_cookie(&state, ws, &cart, json!({ "removed": true }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_bigint_text() {
        assert_eq!(as_id(&json!("12")), Some(12));
        assert_eq!(as_id(&json!("1e3")), None);
        assert_eq!(as_id(&json!(12)), None);
        assert_eq!(as_id(&json!("123456789012345678901")), None);
    }

    #[test]
    fn preview_cookie_round_trip_and_bad_values() {
        let cart = json!({ "items": [{ "productId": "4", "variantId": null, "quantity": 2 }], "discountCode": "SAVE" });
        let mut headers = HeaderMap::new();
        let cookie = format!(
            "a=b; shop-preview-cart-7={}",
            URL_SAFE_NO_PAD.encode(cart.to_string())
        );
        headers.insert(header::COOKIE, cookie.parse().unwrap());
        let read = read_preview_cart(&headers, 7);
        assert_eq!(read.items.len(), 1);
        assert_eq!(read.discount_code.as_deref(), Some("SAVE"));
        // Another workspace's cookie is not this shop's cart.
        assert!(read_preview_cart(&headers, 8).items.is_empty());
        // A forged quantity empties the cart.
        let bad = json!({ "items": [{ "productId": "4", "quantity": 0 }] });
        headers.insert(
            header::COOKIE,
            format!(
                "shop-preview-cart-7={}",
                URL_SAFE_NO_PAD.encode(bad.to_string())
            )
            .parse()
            .unwrap(),
        );
        assert!(read_preview_cart(&headers, 7).items.is_empty());
    }

    #[test]
    fn checkout_form_checks_the_schema() {
        assert!(checkout_form(&json!({ "customerEmail": "" })).is_ok());
        assert!(checkout_form(&json!({ "customerEmail": "nope" })).is_err());
        assert!(checkout_form(&json!({ "paymentMethod": "bitcoin" })).is_err());
        assert!(checkout_form(&json!({ "shippingAddress": { "country": "DEU" } })).is_err());
        let form =
            checkout_form(&json!({ "customerName": "  Ann ", "shippingType": "pickup" })).unwrap();
        assert_eq!(form.customer_name.as_deref(), Some("Ann"));
        assert_eq!(form.shipping_type.as_deref(), Some("pickup"));
    }
}
