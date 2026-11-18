//! The auth rate limits (Q.A1): better-auth's built-in `rateLimit` (memory
//! storage, 10 s windows, key = client IP + path) with its default special
//! rule and the `mobileAuth` plugin rule, plus the builder's
//! `rateLimitMobilePasswordGrant` (5 password grants / 10 s per email) that
//! `app/api/auth/[...all]/route.ts` runs first.
//!
//! ponytail: always on (better-auth enables it in production; the Rust
//! binary has no dev mode).

use std::sync::Arc;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, check_api_rate_limit};
use serde_json::{Value, json};

/// better-auth `rateLimit.max` (window 10 s).
pub const AUTH_RATE_LIMIT: u64 = 100;
/// The special rules (`getDefaultSpecialRules`, `mobileAuth` `/mobile/token`).
pub const AUTH_SIGN_IN_RATE_LIMIT: u64 = 3;
/// `PASSWORD_GRANT_ACCOUNT_LIMIT`.
pub const PASSWORD_GRANT_ACCOUNT_LIMIT: u64 = 5;
const BODY_LIMIT: usize = 1 << 20;

/// The state of [`auth_rate_limit`]: the in-memory counter and the clock.
#[derive(Clone)]
pub struct AuthRateLimit {
    pub kv: Arc<chatbotx_kv::Kv>,
    pub now_ms: fn() -> i64,
}

fn rule_limit(path: &str) -> u64 {
    let special = ["/sign-in", "/sign-up", "/change-password", "/change-email"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || path == "/mobile/token";
    if special {
        AUTH_SIGN_IN_RATE_LIMIT
    } else {
        AUTH_RATE_LIMIT
    }
}

fn json_response(status: StatusCode, body: Value, retry: Option<(&'static str, u64)>) -> Response {
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response();
    if let Some((name, seconds)) = retry
        && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
    {
        response.headers_mut().insert(name, value);
    }
    response
}

/// `readPasswordGrant`: `Ok(Some(email))` for a password grant, `Ok(None)`
/// for another grant, `Err(())` for an unreadable / unknown one.
fn password_grant(content_type: &str, body: &Bytes) -> Result<Option<Option<String>>, ()> {
    let (grant, email) = if content_type.contains("application/json") {
        let value: Value = serde_json::from_slice(body).map_err(|_| ())?;
        let grant = value
            .get("grantType")
            .filter(|v| !v.is_null())
            .or_else(|| value.get("grant_type"))
            .and_then(Value::as_str)
            .map(str::to_string);
        (
            grant,
            value
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_string),
        )
    } else {
        let form: Vec<(String, String)> = url::form_urlencoded::parse(body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let get = |key: &str| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        (get("grantType").or_else(|| get("grant_type")), get("email"))
    };
    match grant.as_deref() {
        Some("refresh_token" | "exchange_code") => Ok(None),
        Some("password") => Ok(Some(email.map(|e| e.trim().to_lowercase()))),
        _ => Err(()),
    }
}

/// The middleware on the `/api/auth/*` routes.
pub async fn auth_rate_limit(
    State(limits): State<AuthRateLimit>,
    request: Request,
    next: Next,
) -> Response {
    let Some(path) = request
        .uri()
        .path()
        .strip_prefix("/api/auth")
        .map(|p| p.trim_end_matches('/').to_string())
    else {
        return next.run(request).await;
    };
    let now_ms = (limits.now_ms)();
    let store = &limits.kv.mem;

    // `rateLimitMobilePasswordGrant` (before better-auth).
    let request = if request.method() == Method::POST && path == "/mobile/token" {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, BODY_LIMIT).await else {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "code": "BAD_REQUEST", "message": "Invalid password grant request" }),
                None,
            );
        };
        let content_type = parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match password_grant(&content_type, &bytes) {
            Err(()) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "code": "BAD_REQUEST", "message": "Invalid password grant request" }),
                    None,
                );
            }
            Ok(Some(Some(email))) => {
                let result = check_api_rate_limit(ApiRateLimitInput {
                    scope: "mobile-token-password-account-rate-limit",
                    key: &email,
                    store,
                    now_ms,
                    limit: Some(PASSWORD_GRANT_ACCOUNT_LIMIT),
                });
                if result.limited {
                    return json_response(
                        StatusCode::TOO_MANY_REQUESTS,
                        json!({
                            "code": "TOO_MANY_REQUESTS",
                            "message": "Too many sign-in attempts. Please try again later.",
                        }),
                        Some(("retry-after", result.retry_after)),
                    );
                }
            }
            Ok(_) => {}
        }
        Request::from_parts(parts, Body::from(bytes))
    } else {
        request
    };

    // better-auth `onRequestRateLimit`.
    let ip = crate::auth::guest_client_ip(request.headers());
    let key = format!("{ip}{path}");
    let result = check_api_rate_limit(ApiRateLimitInput {
        scope: "better-auth",
        key: &key,
        store,
        now_ms,
        limit: Some(rule_limit(&path)),
    });
    if result.limited {
        return json_response(
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "message": "Too many requests. Please try again later." }),
            Some(("x-retry-after", result.retry_after)),
        );
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_rules() {
        assert_eq!(rule_limit("/sign-in/email"), 3);
        assert_eq!(rule_limit("/sign-up/email"), 3);
        assert_eq!(rule_limit("/change-password"), 3);
        assert_eq!(rule_limit("/mobile/token"), 3);
        assert_eq!(rule_limit("/get-session"), 100);
        assert_eq!(rule_limit("/oauth2/token"), 100);
    }

    #[test]
    fn password_grant_reading() {
        let json = Bytes::from(r#"{"grantType":"password","email":" A@B.c "}"#);
        assert_eq!(
            password_grant("application/json", &json),
            Ok(Some(Some("a@b.c".into())))
        );
        let refresh = Bytes::from("grant_type=refresh_token");
        assert_eq!(
            password_grant("application/x-www-form-urlencoded", &refresh),
            Ok(None)
        );
        assert_eq!(
            password_grant("application/json", &Bytes::from("{}")),
            Err(())
        );
        assert_eq!(
            password_grant("application/json", &Bytes::from("nope")),
            Err(())
        );
    }
}
