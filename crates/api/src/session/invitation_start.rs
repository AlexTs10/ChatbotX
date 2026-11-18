//! `app/api/invitations/[code]/start/route.ts` (Q.A1): a per-IP rate limit,
//! then a redirect to sign-up with the invitation cookie, or to sign-in with
//! `error=invalid_invitation`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_kv::rate_limit::{ApiRateLimitInput, check_api_rate_limit};
use url::form_urlencoded::byte_serialize;

use super::{AuthState, INVITATION_COOKIE_MAX_AGE_SECONDS, INVITATION_COOKIE_NAME};

/// `START_LIMIT_PER_IP` (per 10 s window).
pub const START_LIMIT_PER_IP: u64 = 20;
/// `START_LIMIT_UNKNOWN_IP`: callers without a client-IP header share one bucket.
pub const START_LIMIT_UNKNOWN_IP: u64 = 100;

type StartState = (Arc<AuthState>, Arc<chatbotx_kv::Kv>);

/// `GET /api/invitations/{code}/start`.
pub fn invitation_start_router(auth: Arc<AuthState>, kv: Arc<chatbotx_kv::Kv>) -> Router {
    Router::new()
        .route("/api/invitations/{code}/start", get(start))
        .with_state((auth, kv))
}

fn redirect(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(value) => (StatusCode::FOUND, [(header::LOCATION, value)]).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn start(
    State((auth, kv)): State<StartState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let ip = crate::auth::guest_client_ip(&headers);
    let limit = if ip == crate::auth::UNKNOWN_CLIENT_IP {
        START_LIMIT_UNKNOWN_IP
    } else {
        START_LIMIT_PER_IP
    };
    let result = check_api_rate_limit(ApiRateLimitInput {
        scope: "invitation-start-rate-limit",
        key: &ip,
        store: &kv.mem,
        now_ms: (auth.now_ms)(),
        limit: Some(limit),
    });
    if result.limited {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, result.retry_after.to_string())],
        )
            .into_response();
    }

    let origin = super::email_password::public_origin(&auth.settings, &headers);
    // ponytail: one tenant, so `resolveTenantByDomain` is the root tenant
    // (see `invitation::lookup_active`).
    match chatbotx_business::invitation::details(&auth.db, code.clone()).await {
        Ok(Some(_)) => {}
        Ok(None) => return redirect(&format!("{origin}/auth/sign-in?error=invalid_invitation")),
        Err(error) => {
            tracing::error!(%error, "invitation start lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let callback: String = byte_serialize(format!("/invitations/{code}").as_bytes()).collect();
    let mut response = redirect(&format!("{origin}/auth/sign-up?callbackURL={callback}"));
    // `secure` in production: the Rust binary has no dev mode, so it follows
    // the scheme of `BETTER_AUTH_URL`.
    let secure = if auth.settings.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    let value: String = byte_serialize(code.as_bytes()).collect();
    if let Ok(cookie) = HeaderValue::from_str(&format!(
        "{INVITATION_COOKIE_NAME}={value}; Max-Age={INVITATION_COOKIE_MAX_AGE_SECONDS}; Path=/; HttpOnly; SameSite=lax{secure}"
    )) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}
