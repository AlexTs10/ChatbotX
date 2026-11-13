//! better-auth `bearer()` plugin: `Authorization: Bearer <token>` stands for
//! the session cookie (before hook), and a response that sets the session
//! cookie mirrors it in `set-auth-token` (after hook).

use axum::http::HeaderMap;

/// The before hook: the request headers with the bearer token written into
/// the session cookie (unchanged when there is no valid bearer token).
pub fn apply_bearer(headers: &HeaderMap, secret: &str) -> HeaderMap {
    let _ = (headers, secret);
    todo!("P3.5d")
}

/// The after hook: `set-auth-token` and `Access-Control-Expose-Headers` when
/// the response sets a live session cookie.
pub fn expose_auth_token(response_headers: &mut HeaderMap) {
    let _ = response_headers;
    todo!("P3.5d")
}
