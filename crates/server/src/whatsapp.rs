//! `GET|POST /integrations/whatsapp/webhook/{integrationId}` (P1.20, Q.I1a):
//! the HTTP layer of `chatbotx_worker::channels::whatsapp_webhook`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_db::Db;
use chatbotx_jobs::Enqueuer;
use chatbotx_worker::channels::whatsapp_webhook::{HUB_SIGNATURE_HEADER, manual_webhook};

#[derive(Clone)]
struct WebhookState {
    db: Arc<Db>,
    jobs: Arc<dyn Enqueuer>,
}

pub fn router(db: Arc<Db>, jobs: Arc<dyn Enqueuer>) -> Router {
    Router::new()
        .route(
            "/integrations/whatsapp/webhook/{integration_id}",
            get(handle).post(handle),
        )
        .with_state(WebhookState { db, jobs })
}

async fn handle(
    State(state): State<WebhookState>,
    Path(integration_id): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signature = headers
        .get(HUB_SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok());
    let reply = manual_webhook(
        &state.db,
        state.jobs.as_ref(),
        method == Method::GET,
        &integration_id,
        uri.query(),
        signature,
        &body,
    )
    .await;
    (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_REQUEST),
        [(header::CONTENT_TYPE, reply.content_type)],
        reply.body,
    )
        .into_response()
}
