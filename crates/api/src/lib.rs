//! HTTP API: the axum router and its handlers.

pub mod auth;
pub mod booking_public;
pub mod channel_connect;
pub mod connect_callback;
pub mod errors;
pub mod files;
pub mod handlers;
pub mod idempotency;
pub mod inbound_webhook_intake;
pub mod iso8601;
pub mod jobs_admin;
pub mod magic_link_redirect;
pub mod mail;
pub mod orpc;
pub mod payment_webhooks;
pub mod presigned_upload;
pub mod reflink_widget;
pub mod routes;
pub mod rpc;
pub mod rpc_ai;
pub mod rpc_procedures;
pub mod session;
pub mod shop_public;
pub mod spa;
pub mod store_webhooks;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// Build the HTTP router.
pub fn router() -> Router {
    Router::new().route("/api/health", get(health))
}

/// `GET /api/health`, same body and key order as the Next.js builder route.
async fn health() -> impl IntoResponse {
    let body = format!(
        r#"{{"status":"healthy","timestamp":"{}","service":"builder"}}"#,
        iso8601::now()
    );
    ([(header::CONTENT_TYPE, "application/json")], body)
}
