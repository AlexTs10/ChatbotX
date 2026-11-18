//! Q.I4 (P1.23): `POST /api/shopify/{workspaceId}/webhook` and
//! `POST /api/woocommerce/{workspaceId}/webhook` (`receiveShopifyWebhook`,
//! `receiveWooCommerceWebhook`). The HMAC is checked with the store's
//! secret (constant time), the delivery is queued as `shopifyWebhook` /
//! `woocommerceWebhook` on the integration queue (job id = delivery, so a
//! retry runs once) and answered at once. The job side (re-read and write
//! the object) is Q.D8's store sync.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine as _;
use chatbotx_core::encryption::EncryptedData;
use chatbotx_jobs::{JobOptions, QueueName};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::handlers::ApiState;

pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/shopify/{workspaceId}/webhook", post(shopify))
        .route("/api/woocommerce/{workspaceId}/webhook", post(woocommerce))
        .with_state(state)
}

/// `timingSafeEqual` over the base64 HMAC-SHA256 of the raw body.
pub fn valid_signature(raw_body: &[u8], signature: Option<&str>, secret: &str) -> bool {
    let Some(signature) = signature else {
        return false;
    };
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(raw_body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let (left, right) = (expected.as_bytes(), signature.trim().as_bytes());
    // Every byte of the longer input, so the time does not show the first difference.
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(a ^ b);
    }
    difference == 0
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// JS truthiness of an id field, as text.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) if n.as_f64() != Some(0.0) => Some(n.to_string()),
        _ => None,
    }
}

/// `shopifyWebhookObjectId`.
pub fn shopify_object_id(topic: &str, body: &Value) -> Option<String> {
    let gid = |kind: &str, id: String| format!("gid://shopify/{kind}/{id}");
    if topic == "inventory_levels/update" {
        return id_text(body.get("inventory_item_id")).map(|id| gid("InventoryItem", id));
    }
    if topic.starts_with("fulfillments/") {
        return id_text(body.get("order_id")).map(|id| gid("Order", id));
    }
    if let Some(Value::String(id)) = body.get("admin_graphql_api_id")
        && !id.is_empty()
    {
        return Some(id.clone());
    }
    let id = id_text(body.get("id"))?;
    if topic.starts_with("products/") {
        return Some(gid("Product", id));
    }
    topic.starts_with("orders/").then(|| gid("Order", id))
}

/// The decrypted auth secret (`encryptUtils.decryptObject`, AAD `<store>:<ws>`).
fn secret_of(encrypted: &Value, aad: &str) -> Option<Value> {
    let data: EncryptedData = serde_json::from_value(encrypted.clone()).ok()?;
    chatbotx_business::meta_catalog::process_encryption()
        .decrypt_object::<Value>(&data, Some(aad))
        .ok()
}

fn reply(status: StatusCode) -> Response {
    crate::handlers::json(
        status.as_u16(),
        json!({ "received": status == StatusCode::OK }),
    )
}

fn workspace_id(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A number past i64 has no row: 404 as the TS lookup.
    Some(text.parse().unwrap_or(-1))
}

async fn enqueue(
    state: &ApiState,
    name: &str,
    data: Value,
    job_id: Option<String>,
) -> Result<(), String> {
    let jobs = state.jobs.as_ref().ok_or("the job queue is off")?;
    jobs.add(
        QueueName::Integration,
        name,
        json!({ "type": name, "data": data }),
        JobOptions {
            job_id,
            ..JobOptions::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

async fn shopify(
    State(state): State<Arc<ApiState>>,
    Path(ws): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(ws) = workspace_id(&ws) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match receive_shopify(&state, ws, &headers, &body).await {
        Ok(status) => reply(status),
        Err(err) => {
            tracing::error!(err = %err, "[shopify] the webhook could not be handled");
            reply(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn receive_shopify(
    state: &ApiState,
    ws: i64,
    headers: &HeaderMap,
    raw: &[u8],
) -> Result<StatusCode, String> {
    let row = chatbotx_db::repo::read(&state.db, move |conn| {
        chatbotx_db::repo::store_integration::find_shopify(conn, ws)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(row) = row else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let auth = secret_of(&row.encrypted_auth.0, &format!("shopify:{ws}"))
        .ok_or("the Shopify auth does not decrypt")?;
    // `webhookSecretOf`.
    let key = if auth["kind"] == "token" {
        "apiSecret"
    } else {
        "clientSecret"
    };
    let secret = auth[key].as_str().unwrap_or_default();
    if !valid_signature(raw, header(headers, "x-shopify-hmac-sha256"), secret) {
        return Ok(StatusCode::UNAUTHORIZED);
    }
    if let Some(shop) = header(headers, "x-shopify-shop-domain")
        && shop.to_lowercase() != row.shop_domain
    {
        tracing::warn!(
            workspace_id = ws,
            shop_domain = shop,
            "[shopify] a webhook came from another store"
        );
        return Ok(StatusCode::OK);
    }
    let topic = header(headers, "x-shopify-topic").unwrap_or("").to_owned();
    let webhook_id =
        header(headers, "x-shopify-webhook-id").or_else(|| header(headers, "x-shopify-event-id"));
    let Ok(body) = serde_json::from_slice::<Value>(raw) else {
        return Ok(StatusCode::OK);
    };
    let object_id = shopify_object_id(&topic, &body);
    if !(topic == "app/uninstalled" || object_id.is_some()) {
        return Ok(StatusCode::OK);
    }
    let data = json!({ "workspaceId": ws.to_string(), "topic": topic, "objectId": object_id });
    enqueue(
        state,
        "shopifyWebhook",
        data,
        webhook_id.map(|id| format!("shopify-webhook-{id}")),
    )
    .await?;
    Ok(StatusCode::OK)
}

async fn woocommerce(
    State(state): State<Arc<ApiState>>,
    Path(ws): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(ws) = workspace_id(&ws) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match receive_woocommerce(&state, ws, &headers, &body).await {
        Ok(status) => reply(status),
        Err(err) => {
            tracing::error!(err = %err, "[woocommerce] the webhook could not be handled");
            reply(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn same_site(left: &str, right: &str) -> bool {
    left.trim_end_matches('/').to_lowercase() == right.trim_end_matches('/').to_lowercase()
}

/// `deliveryKey`: `<ws>-sha256hex(webhookId \n deliveryId \n body)`.
fn delivery_key(ws: i64, headers: &HeaderMap, raw: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(header(headers, "x-wc-webhook-id").unwrap_or("").as_bytes());
    hash.update(b"\n");
    hash.update(
        header(headers, "x-wc-webhook-delivery-id")
            .unwrap_or("")
            .as_bytes(),
    );
    hash.update(b"\n");
    hash.update(raw);
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{ws}-{hex}")
}

async fn receive_woocommerce(
    state: &ApiState,
    ws: i64,
    headers: &HeaderMap,
    raw: &[u8],
) -> Result<StatusCode, String> {
    let Some(topic) = header(headers, "x-wc-webhook-topic") else {
        return Ok(StatusCode::OK);
    };
    let row = chatbotx_db::repo::read(&state.db, move |conn| {
        chatbotx_db::repo::store_integration::find_woocommerce(conn, ws)
    })
    .await
    .map_err(|e| e.message())?;
    let Some(row) = row else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let auth = secret_of(&row.encrypted_auth.0, &format!("woocommerce:{ws}"))
        .ok_or("the WooCommerce auth does not decrypt")?;
    let secret = auth["webhookSecret"].as_str().unwrap_or_default();
    if !valid_signature(raw, header(headers, "x-wc-webhook-signature"), secret) {
        return Ok(StatusCode::UNAUTHORIZED);
    }
    if let Some(source) = header(headers, "x-wc-webhook-source")
        && !same_site(source, &row.site_url)
    {
        tracing::warn!(
            workspace_id = ws,
            source,
            "[woocommerce] a webhook came from another site"
        );
        return Ok(StatusCode::OK);
    }
    // `resourceIdOf`: a number or string `id` (any string, also empty).
    let resource_id = match serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|b| b.get("id").cloned())
    {
        Some(Value::String(id)) => id,
        Some(Value::Number(id)) => id.to_string(),
        _ => return Ok(StatusCode::OK),
    };
    let data = json!({ "workspaceId": ws.to_string(), "topic": topic, "resourceId": resource_id });
    let job_id = format!("woocommerce-webhook-{}", delivery_key(ws, headers, raw));
    enqueue(state, "woocommerceWebhook", data, Some(job_id)).await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(body: &[u8], secret: &str) -> String {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    #[test]
    fn checks_signatures() {
        let good = sign(b"{}", "s3cret");
        assert!(valid_signature(b"{}", Some(&format!(" {good} ")), "s3cret"));
        assert!(!valid_signature(b"{}", Some(&good), "other"));
        assert!(!valid_signature(b"{ }", Some(&good), "s3cret"));
        assert!(!valid_signature(
            b"{}",
            Some(&good[..good.len() - 1]),
            "s3cret"
        ));
        assert!(!valid_signature(b"{}", None, "s3cret"));
    }

    #[test]
    fn shopify_object_ids() {
        let id = |topic: &str, body: Value| shopify_object_id(topic, &body);
        assert_eq!(
            id("inventory_levels/update", json!({ "inventory_item_id": 7 })).as_deref(),
            Some("gid://shopify/InventoryItem/7")
        );
        assert_eq!(
            id("fulfillments/create", json!({ "order_id": "9" })).as_deref(),
            Some("gid://shopify/Order/9")
        );
        assert_eq!(
            id(
                "products/update",
                json!({ "admin_graphql_api_id": "gid://shopify/Product/1", "id": 2 })
            )
            .as_deref(),
            Some("gid://shopify/Product/1")
        );
        assert_eq!(
            id("products/delete", json!({ "id": 2 })).as_deref(),
            Some("gid://shopify/Product/2")
        );
        assert_eq!(
            id("orders/paid", json!({ "id": 3 })).as_deref(),
            Some("gid://shopify/Order/3")
        );
        assert_eq!(id("discounts/delete", json!({ "id": 3 })), None);
        assert_eq!(id("products/delete", json!({ "id": 0 })), None);
    }

    #[test]
    fn woocommerce_sites_and_keys() {
        assert!(same_site("https://Shop.example/", "https://shop.example"));
        assert!(!same_site("https://a.example", "https://b.example"));
        let headers = HeaderMap::new();
        let key = delivery_key(5, &headers, b"x");
        assert!(key.starts_with("5-") && key.len() == 2 + 64);
    }
}
