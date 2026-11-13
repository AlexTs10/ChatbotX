//! better-auth `bearer()` plugin: `Authorization: Bearer <token>` stands for
//! the session cookie (before hook), and a response that sets the session
//! cookie mirrors it in `set-auth-token` (after hook).

use axum::http::{HeaderMap, HeaderValue, header};

use super::{SESSION_COOKIE, sign_cookie_value, verify_signed_cookie};
use crate::auth::decode_query_component;

/// The before hook: the request headers with the bearer token written into
/// the session cookie (unchanged when there is no valid bearer token).
pub fn apply_bearer(headers: &HeaderMap, secret: &str) -> HeaderMap {
    let mut rewritten = headers.clone();
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() >= 7 && value[..7].eq_ignore_ascii_case("bearer "))
        .map(|value| value[7..].trim())
        .filter(|token| !token.is_empty())
    else {
        return rewritten;
    };
    let signed = if token.contains('.') {
        token.to_string()
    } else {
        sign_cookie_value(token, secret)
    };
    if verify_signed_cookie(&signed, secret).is_none() {
        return rewritten;
    }
    let decoded = decode_query_component(&signed.replace('+', "%2B"));
    let mut cookies: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split('=').next() != Some(SESSION_COOKIE))
        .map(str::to_string)
        .collect();
    cookies.push(format!(
        "{SESSION_COOKIE}={}",
        sign_cookie_value_from_decoded(&decoded)
    ));
    rewritten.remove(header::COOKIE);
    if let Ok(value) = HeaderValue::from_str(&cookies.join("; ")) {
        rewritten.insert(header::COOKIE, value);
    }
    rewritten
}

/// The after hook: `set-auth-token` and `Access-Control-Expose-Headers` when
/// the response sets a live session cookie.
pub fn expose_auth_token(response_headers: &mut HeaderMap) {
    let prefix = format!("{SESSION_COOKIE}=");
    let token = response_headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|cookie| {
            let mut parts = cookie.split(';').map(str::trim);
            let value = parts.next()?.strip_prefix(&prefix)?.to_string();
            let cleared = parts.any(|part| part.eq_ignore_ascii_case("max-age=0"));
            (!value.is_empty() && !cleared).then_some(value)
        })
        .next_back();
    let Some(token) = token else { return };
    let mut exposed: Vec<String> = response_headers
        .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    if !exposed.iter().any(|name| name == "set-auth-token") {
        exposed.push("set-auth-token".to_string());
    }
    if let (Ok(token), Ok(exposed)) = (
        HeaderValue::from_str(&token),
        HeaderValue::from_str(&exposed.join(", ")),
    ) {
        response_headers.insert("set-auth-token", token);
        response_headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, exposed);
    }
}

/// The cookie form (URL-encoded) of a decoded `token.signature` value.
fn sign_cookie_value_from_decoded(decoded: &str) -> String {
    decoded
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}
