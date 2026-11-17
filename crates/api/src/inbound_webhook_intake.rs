//! `apps/builder/src/app/api/hooks/[token]/route.ts` +
//! `features/inbound-webhooks/lib/handle-inbound-request.ts`: accepts one
//! delivery (token check, rate limits, size cap, durable row, job).
//! The token, URL and payload are never logged.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chatbotx_business::inbound_webhook as service;
use chatbotx_core::utils::json_path::is_jsonb_storable;
use chatbotx_jobs::queues::inbound_webhook::{
    InboundWebhookRequestData, enqueue_process_inbound_webhook_request,
};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, check_api_rate_limit};
use serde_json::{Value, json};

use crate::auth::{UNKNOWN_CLIENT_IP, guest_client_ip};
use crate::handlers::ApiState;

/// Per 10 s window.
const PREAUTH_LIMIT_PER_IP: u64 = 300;
const PREAUTH_LIMIT_UNKNOWN_IP: u64 = 1500;
const WEBHOOK_LIMIT: u64 = 600;

/// `POST /api/hooks/{token}`; every other method is 405.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route(
            "/api/hooks/{token}",
            post(handle).fallback(method_not_allowed),
        )
        .with_state(state)
}

async fn method_not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST"), (header::CACHE_CONTROL, "no-store")],
    )
        .into_response()
}

fn respond(status: StatusCode, body: Value, retry_after: Option<u64>) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(seconds) = retry_after {
        headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
    }
    response
}

fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    respond(
        status,
        json!({ "accepted": false, "code": code, "message": message }),
        None,
    )
}

fn rate_limited(retry_after: u64) -> Response {
    respond(
        StatusCode::TOO_MANY_REQUESTS,
        json!({ "accepted": false, "code": "RATE_LIMITED", "message": "Too many requests" }),
        Some(retry_after),
    )
}

fn system_error() -> Response {
    failure(
        StatusCode::INTERNAL_SERVER_ERROR,
        "SYSTEM_ERROR",
        "Please retry",
    )
}

/// `^iwh_[A-Za-z0-9_-]{32,64}$`.
fn token_shape(token: &str) -> bool {
    token
        .strip_prefix(service::TOKEN_PREFIX)
        .is_some_and(|rest| {
            (32..=64).contains(&rest.len())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

/// `checkApiRateLimit`: `Some(retryAfter)` when limited. No kv store = no limit.
fn limited(state: &ApiState, scope: &str, key: &str, limit: u64) -> Option<u64> {
    let kv = state.kv.as_ref()?;
    let result = check_api_rate_limit(ApiRateLimitInput {
        scope,
        key,
        store: &kv.mem,
        now_ms: chatbotx_db::repo::now_ms(),
        limit: Some(limit),
    });
    result.limited.then_some(result.retry_after)
}

/// The body as text, `None` once it exceeds the cap (declared or streamed).
async fn read_body(headers: &HeaderMap, body: Body) -> Option<Vec<u8>> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok());
    if declared.is_some_and(|n| n > service::MAX_PAYLOAD_BYTES) {
        return None;
    }
    axum::body::to_bytes(body, service::MAX_PAYLOAD_BYTES)
        .await
        .ok()
        .map(|b| b.to_vec())
}

async fn handle(
    State(state): State<Arc<ApiState>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if !token_shape(&token) {
        return failure(StatusCode::NOT_FOUND, "NOT_FOUND", "Webhook not found");
    }
    let client_ip = guest_client_ip(&headers);
    let preauth_limit = if client_ip == UNKNOWN_CLIENT_IP {
        PREAUTH_LIMIT_UNKNOWN_IP
    } else {
        PREAUTH_LIMIT_PER_IP
    };
    if let Some(retry) = limited(
        &state,
        "inbound-webhook-preauth-rate-limit",
        &client_ip,
        preauth_limit,
    ) {
        return rate_limited(retry);
    }
    let Some(raw) = read_body(&headers, body).await else {
        return failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "The request body is too large",
        );
    };

    let db = &state.db;
    let webhook = match service::find_by_token(db, token).await {
        Ok(Some(w)) => w,
        Ok(None) => return failure(StatusCode::NOT_FOUND, "NOT_FOUND", "Webhook not found"),
        Err(err) => {
            tracing::error!(err = ?err, "Inbound webhook request failed");
            return system_error();
        }
    };
    match service::workspace_unavailable(db, webhook.workspace_id).await {
        Ok(false) => {}
        Ok(true) => {
            return failure(
                StatusCode::GONE,
                "WORKSPACE_UNAVAILABLE",
                "This webhook is no longer available",
            );
        }
        Err(err) => {
            tracing::error!(err = ?err, "Inbound webhook request failed");
            return system_error();
        }
    }
    if !(webhook.active && webhook.is_configured) {
        return failure(
            StatusCode::FORBIDDEN,
            "WEBHOOK_INACTIVE",
            "This webhook is not active",
        );
    }
    if let Some(retry) = limited(
        &state,
        "inbound-webhook-rate-limit",
        &webhook.id.to_string(),
        WEBHOOK_LIMIT,
    ) {
        return rate_limited(retry);
    }

    let payload = serde_json::from_slice::<Value>(&raw)
        .ok()
        .filter(|v| (v.is_object() || v.is_array()) && is_jsonb_storable(v));
    let Some(payload) = payload else {
        return failure(
            StatusCode::BAD_REQUEST,
            "INVALID_JSON",
            "The body must be a JSON object or array",
        );
    };
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|k| !k.is_empty());
    if key.is_some_and(|k| k.chars().count() > service::MAX_IDEMPOTENCY_KEY_LENGTH) {
        return failure(
            StatusCode::BAD_REQUEST,
            "INVALID_IDEMPOTENCY_KEY",
            "Idempotency-Key is too long",
        );
    }
    // Audit data only: an address at all, never free text.
    let ip = client_ip
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|_| client_ip);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| {
            ua.chars()
                .take(service::MAX_USER_AGENT_LENGTH)
                .collect::<String>()
        })
        .filter(|ua| !ua.is_empty());

    let recorded = service::record_request(
        db,
        &webhook,
        payload,
        key.map(str::to_owned),
        ip,
        user_agent,
    )
    .await;
    let request = match recorded {
        Ok(Some(request)) => request,
        Ok(None) => {
            return respond(
                StatusCode::OK,
                json!({ "accepted": true, "duplicate": true }),
                None,
            );
        }
        Err(err) => {
            tracing::error!(err = ?err, "Inbound webhook request failed");
            return system_error();
        }
    };
    if let Some(jobs) = &state.jobs {
        let data = InboundWebhookRequestData {
            workspace_id: webhook.workspace_id.to_string(),
            inbound_webhook_id: webhook.id.to_string(),
            request_id: request.id.to_string(),
        };
        // The row is the outbox: a failed enqueue is still accepted.
        if let Err(err) = enqueue_process_inbound_webhook_request(jobs.as_ref(), data).await {
            tracing::warn!(
                err = %err,
                inbound_webhook_id = webhook.id,
                request_id = request.id,
                "Could not enqueue an inbound webhook request; the sweep will"
            );
        }
    }
    respond(
        StatusCode::ACCEPTED,
        json!({ "accepted": true, "requestId": request.id.to_string() }),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::token_shape;

    #[test]
    fn token_shape_matches_the_original_pattern() {
        assert!(token_shape(&format!("iwh_{}", "a".repeat(32))));
        assert!(token_shape(&format!("iwh_{}", "A-_9".repeat(16))));
        assert!(!token_shape(&format!("iwh_{}", "a".repeat(31))));
        assert!(!token_shape(&format!("iwh_{}", "a".repeat(65))));
        assert!(!token_shape(&format!("iwx_{}", "a".repeat(40))));
        assert!(!token_shape(&format!("iwh_{}.", "a".repeat(40))));
    }
}
