//! HTTP API: the axum router and its handlers.

pub mod auth;
pub mod connect_callback;
pub mod errors;
pub mod files;
pub mod handlers;
pub mod idempotency;
pub mod iso8601;
pub mod jobs_admin;
pub mod mail;
pub mod orpc;
pub mod presigned_upload;
pub mod reflink_widget;
pub mod routes;
pub mod rpc;
pub mod rpc_ai;
pub mod rpc_procedures;
pub mod session;
pub mod spa;

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
