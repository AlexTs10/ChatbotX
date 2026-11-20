//! Q.P6-U5 (COVERAGE C7, C8): `GET /email-topic/open` (the 1x1 pixel,
//! `app/email-topic/open/route.ts`) and `GET /email-topic/click` (the signed
//! redirect, `app/email-topic/click/route.ts`). The click route only
//! redirects to a URL recovered from an authenticated token (no open
//! redirect); anything else goes to the same origin.

use std::sync::Arc;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::email_tracking::{self as tracking, Phase};
use chatbotx_core::utils::request::{
    RequestParts, force_public_https, get_public_origin_from_request,
};
use serde_json::json;

use crate::handlers::ApiState;
use crate::magic_link_redirect::servable;

/// The two routes.
pub fn router(api: Arc<ApiState>) -> Router {
    Router::new()
        .route("/email-topic/open", get(open))
        .route("/email-topic/click", get(click))
        .with_state(api)
}

/// A transparent 1x1 GIF.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x01, 0x44, 0x00, 0x3b,
];

fn param(query: Option<&str>, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

async fn open(State(api): State<Arc<ApiState>>, RawQuery(query): RawQuery) -> Response {
    if let Some(token) = param(query.as_deref(), "r") {
        let jobs = api.jobs.as_deref();
        match tracking::workspace_id_by_token(&api.db, &token).await {
            Ok(Some(ws)) => {
                if servable(&api, ws).await {
                    tracking::record_transition_or_enqueue_repair(
                        &api.db,
                        jobs,
                        Phase::Opened,
                        &token,
                        Some(ws),
                        None,
                    )
                    .await;
                    tracking::award_leaderboard_engagement(&api.db, jobs, &token, true, None).await;
                }
            }
            _ => {
                tracking::record_transition_or_enqueue_repair(
                    &api.db,
                    jobs,
                    Phase::Opened,
                    &token,
                    None,
                    None,
                )
                .await;
            }
        }
    }
    (
        [
            (header::CONTENT_TYPE, "image/gif"),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        GIF,
    )
        .into_response()
}

fn redirect(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(value) => (StatusCode::FOUND, [(header::LOCATION, value)]).into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn gone() -> Response {
    crate::handlers::json(410, json!({ "code": "workspaceScheduledDeletion" }))
}

/// `new URL(request.url).origin` (behind the proxy: the public origin).
fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let url = format!("http://{host}/email-topic/click");
    get_public_origin_from_request(
        &RequestParts { url: &url, headers },
        force_public_https(&chatbotx_core::env::ProcessEnv),
    )
}

async fn click(
    State(api): State<Arc<ApiState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let token = param(query.as_deref(), "r");
    let signed = param(query.as_deref(), "u");
    // `NextResponse.redirect(origin)`: `new URL(origin)` ends with `/`.
    let home = format!("{}/", origin(&headers).trim_end_matches('/'));
    let jobs = api.jobs.as_deref();
    let mut tracked_ws = None;
    let mut checked_ws = None;
    if let Some(token) = &token
        && let Ok(Some(ws)) = tracking::workspace_id_by_token(&api.db, token).await
    {
        tracked_ws = Some(ws);
        if !servable(&api, ws).await {
            return gone();
        }
        checked_ws = Some(ws);
    }
    if let Some(signed) = &signed
        // Tampered, malformed or expired: never redirect to it.
        && let Ok(payload) = tracking::verify_click_token(signed)
    {
        let payload_ws = payload.workspace_id.as_deref().map(str::parse::<i64>);
        let payload_ws = match payload_ws {
            Some(Ok(ws)) => Some(ws),
            Some(Err(_)) => return redirect(&home),
            None => None,
        };
        if let (Some(a), Some(b)) = (payload_ws, tracked_ws)
            && a != b
        {
            return redirect(&home);
        }
        let Some(redirect_ws) = payload_ws.or(tracked_ws) else {
            return redirect(&home);
        };
        if checked_ws != Some(redirect_ws) && !servable(&api, redirect_ws).await {
            return gone();
        }
        if let Some(token) = &token {
            tracking::record_transition_or_enqueue_repair(
                &api.db,
                jobs,
                Phase::Clicked,
                token,
                tracked_ws,
                Some(&payload.url),
            )
            .await;
            tracking::award_leaderboard_engagement(&api.db, jobs, token, false, Some(&payload.url))
                .await;
        }
        return redirect(&payload.url);
    }
    if signed.is_none()
        && let Some(token) = &token
    {
        tracking::record_transition_or_enqueue_repair(
            &api.db,
            jobs,
            Phase::Clicked,
            token,
            tracked_ws,
            None,
        )
        .await;
        tracking::award_leaderboard_engagement(&api.db, jobs, token, false, None).await;
    }
    redirect(&home)
}
