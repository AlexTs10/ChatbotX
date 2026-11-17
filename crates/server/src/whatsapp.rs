//! `GET|POST /integrations/whatsapp/webhook/{integrationId}` (the manual
//! route) and `GET|POST /integrations/whatsapp/webhook` (the platform app)
//! (P1.20, Q.I1a): the HTTP layer of
//! `chatbotx_worker::channels::whatsapp_webhook`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::Db;
use chatbotx_jobs::Enqueuer;
use chatbotx_worker::channels::whatsapp_webhook::{
    HUB_SIGNATURE_HEADER, PlatformRequest, WebhookReply, manual_webhook, platform_webhook,
};

#[derive(Clone)]
struct WebhookState {
    db: Arc<Db>,
    jobs: Arc<dyn Enqueuer>,
    /// `ENCRYPTION_SECRET`; `None` = no platform credential can be read.
    encryption: Option<Arc<Encryption>>,
    is_cloud: bool,
    /// `isBrokerHost`: the host of `NEXT_PUBLIC_BROKER_URL` or the app URL.
    broker_host: String,
}

pub fn router(db: Arc<Db>, jobs: Arc<dyn Enqueuer>, is_cloud: bool, app_url: &str) -> Router {
    let encryption = Keys::from_env(&chatbotx_core::env::ProcessEnv)
        .map(|keys| Arc::new(Encryption::new(keys)))
        .ok();
    let broker = std::env::var("NEXT_PUBLIC_BROKER_URL").unwrap_or_else(|_| app_url.to_owned());
    let broker_host = url::Url::parse(&broker)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    Router::new()
        .route(
            "/integrations/whatsapp/webhook/{integration_id}",
            get(handle).post(handle),
        )
        .route(
            "/integrations/whatsapp/webhook",
            get(handle_platform).post(handle_platform),
        )
        // ponytail: the other channel webhooks share this state and `reply`.
        .route(
            "/integrations/telegram/webhook",
            get(handle_telegram).post(handle_telegram),
        )
        .route(
            "/integrations/tiktok/webhook",
            get(handle_tiktok).post(handle_tiktok),
        )
        .route(
            "/integrations/zalo/webhook",
            get(handle_zalo).post(handle_zalo),
        )
        .route("/api/zalo-verifier/{verifier}", get(handle_zalo_verifier))
        .with_state(WebhookState {
            db,
            jobs,
            encryption,
            is_cloud,
            broker_host,
        })
}

fn reply(reply: WebhookReply) -> Response {
    (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_REQUEST),
        [(header::CONTENT_TYPE, reply.content_type)],
        reply.body,
    )
        .into_response()
}

/// `/integrations/telegram/webhook?botId=` (P1.22).
async fn handle_telegram(
    State(state): State<WebhookState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let bot_id = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == "botId")
        .map(|(_, v)| v.into_owned());
    let secret = headers
        .get(chatbotx_worker::channels::telegram::SECRET_HEADER)
        .and_then(|v| v.to_str().ok());
    reply(
        chatbotx_worker::channels::telegram::webhook(
            &state.db,
            state.jobs.as_ref(),
            bot_id.as_deref(),
            secret,
            &String::from_utf8_lossy(&body),
        )
        .await,
    )
}

/// `/integrations/tiktok/webhook` (P1.22).
async fn handle_tiktok(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signature = headers
        .get(chatbotx_worker::channels::tiktok::SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok());
    reply(
        chatbotx_worker::channels::tiktok::webhook(
            &state.db,
            state.jobs.as_ref(),
            signature,
            &String::from_utf8_lossy(&body),
        )
        .await,
    )
}

/// `/integrations/zalo/webhook` (P1.22): the platform `zalo` credential.
async fn handle_zalo(
    State(state): State<WebhookState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header_text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    reply(
        chatbotx_worker::channels::zalo::webhook(
            &state.db,
            state.jobs.as_ref(),
            state.encryption.as_deref(),
            state.is_cloud,
            &state.broker_host,
            chatbotx_worker::channels::zalo::WebhookRequest {
                method: method.as_str(),
                signature: header_text(chatbotx_worker::channels::zalo::SIGNATURE_HEADER),
                domain: header_text("x-domain"),
                body: &String::from_utf8_lossy(&body),
            },
        )
        .await,
    )
}

/// `GET /api/zalo-verifier/{verifier}`.
async fn handle_zalo_verifier(Path(verifier): Path<String>) -> Response {
    reply(chatbotx_worker::channels::zalo::verifier_page(&verifier))
}

/// The `/zalo_verifier:verifier.html` rewrite (`next.config`), as a layer on
/// the whole app: Zalo fetches the file from the site root.
pub async fn zalo_verifier_rewrite(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let verifier = request
        .uri()
        .path()
        .strip_prefix("/zalo_verifier")
        .and_then(|rest| rest.strip_suffix(".html"))
        // ponytail: not percent-decoded; Zalo verifiers are plain tokens.
        .filter(|v| !v.contains('/'))
        .map(str::to_owned);
    match verifier {
        Some(verifier) if request.method() == Method::GET => {
            reply(chatbotx_worker::channels::zalo::verifier_page(&verifier))
        }
        _ => next.run(request).await,
    }
}

async fn handle_platform(
    State(state): State<WebhookState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header_text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    reply(
        platform_webhook(
            &state.db,
            state.jobs.as_ref(),
            state.encryption.as_deref(),
            state.is_cloud,
            &state.broker_host,
            PlatformRequest {
                is_get: method == Method::GET,
                query: uri.query(),
                signature_header: header_text(HUB_SIGNATURE_HEADER),
                domain: header_text("x-domain"),
                body: &body,
            },
        )
        .await,
    )
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
    self::reply(reply)
}
