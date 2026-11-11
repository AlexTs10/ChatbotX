//! HTTP API: the axum router and its handlers.

pub mod iso8601;

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
