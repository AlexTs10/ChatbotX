//! Port of `packages/auth/__tests__/bearer-plugin.test.ts`.
//!
//! The original checks that `createAuth` registers better-auth's `bearer()`
//! plugin after `oneTimeToken()` and before `nextCookies()`, so a bearer token
//! works as the session cookie and the `set-auth-token` header mirrors every
//! session cookie write. There is no plugin list in the Rust port; the test
//! checks the two hooks of the plugin instead (adapted).

use axum::http::{HeaderMap, HeaderValue, header};
use chatbotx_api::session::{
    SESSION_COOKIE, apply_bearer, expose_auth_token, request_cookie, sign_cookie_value,
    verify_signed_cookie,
};

const SECRET: &str = "test-secret";

#[test]
fn registers_the_bearer_plugin_after_onetimetoken_and_before_nextcookies() {
    // Before hook: a raw session token becomes the signed session cookie,
    // replacing a stale cookie of the same name and keeping other cookies.
    let mut request = HeaderMap::new();
    request.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer rawsessiontoken"),
    );
    request.insert(
        header::COOKIE,
        HeaderValue::from_static("theme=dark; better-auth.session_token=stale.sig"),
    );
    let rewritten = apply_bearer(&request, SECRET);
    let cookie = request_cookie(&rewritten, SESSION_COOKIE).expect("session cookie");
    assert_eq!(
        verify_signed_cookie(&cookie, SECRET).as_deref(),
        Some("rawsessiontoken")
    );
    assert_eq!(request_cookie(&rewritten, "theme").as_deref(), Some("dark"));

    // A signed bearer token is used as is; a forged one is ignored.
    let signed = sign_cookie_value("othertoken", SECRET);
    let mut request = HeaderMap::new();
    request.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("bearer {signed}")).unwrap(),
    );
    let cookie = request_cookie(&apply_bearer(&request, SECRET), SESSION_COOKIE).unwrap();
    assert_eq!(
        verify_signed_cookie(&cookie, SECRET).as_deref(),
        Some("othertoken")
    );
    let mut forged = HeaderMap::new();
    forged.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer token.forged"),
    );
    assert!(request_cookie(&apply_bearer(&forged, SECRET), SESSION_COOKIE).is_none());

    // After hook: a session Set-Cookie is mirrored in `set-auth-token`;
    // a cleared cookie (Max-Age=0) is not.
    let mut response = HeaderMap::new();
    response.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{SESSION_COOKIE}={signed}; Max-Age=604800; Path=/; HttpOnly; SameSite=Lax"
        ))
        .unwrap(),
    );
    expose_auth_token(&mut response);
    assert_eq!(response.get("set-auth-token").unwrap(), signed.as_str());
    assert_eq!(
        response.get(header::ACCESS_CONTROL_EXPOSE_HEADERS).unwrap(),
        "set-auth-token"
    );
    let mut cleared = HeaderMap::new();
    cleared.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("better-auth.session_token=; Max-Age=0; Path=/"),
    );
    expose_auth_token(&mut cleared);
    assert!(cleared.get("set-auth-token").is_none());
}
