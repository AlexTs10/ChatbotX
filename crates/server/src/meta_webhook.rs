//! `GET|POST /integrations/{type}/webhook[/c/{credentialId}]` (P1.21,
//! Q.I1b): the HTTP layer of `chatbotx_worker::channels::meta_webhook`.
//! Each Meta type gets static paths so they never overlap the WhatsApp
//! manual route.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_worker::channels::meta_webhook::{MetaRequest, MetaWebhook, TYPES};
use chatbotx_worker::channels::whatsapp_webhook::HUB_SIGNATURE_HEADER;

pub fn router(webhook: MetaWebhook) -> Router {
    let webhook = Arc::new(webhook);
    let mut router = Router::new();
    for kind in TYPES {
        let plain = move |s, m, u, h, b| handle(s, kind, None, m, u, h, b);
        let scoped =
            move |s, Path(c): Path<String>, m, u, h, b| handle(s, kind, Some(c), m, u, h, b);
        router = router
            .route(
                &format!("/integrations/{kind}/webhook"),
                get(plain).post(plain),
            )
            .route(
                &format!("/integrations/{kind}/webhook/c/{{credential_id}}"),
                get(scoped).post(scoped),
            );
    }
    router.with_state(webhook)
}

async fn handle(
    State(webhook): State<Arc<MetaWebhook>>,
    kind: &'static str,
    credential_id: Option<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let reply = webhook
        .handle(MetaRequest {
            kind,
            credential_id: credential_id.as_deref(),
            is_get: method == Method::GET,
            query: uri.query(),
            domain: header("x-domain").unwrap_or(""),
            signature: header(HUB_SIGNATURE_HEADER),
            body: &body,
        })
        .await;
    (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_REQUEST),
        [(header::CONTENT_TYPE, reply.content_type)],
        reply.body,
    )
        .into_response()
}
