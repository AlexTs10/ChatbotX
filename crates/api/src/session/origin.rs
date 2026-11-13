//! Trusted origins and the better-auth origin / CSRF checks.
//!
//! - `createAuth(...).trustedOrigins` (`packages/auth/src/server.ts`): broker,
//!   builder, the mobile app schemes and (outside `next build`) every active
//!   custom domain as `https://<domain>`.
//! - better-auth `getTrustedOrigins`: the `BETTER_AUTH_URL` origin, the app
//!   list and `BETTER_AUTH_TRUSTED_ORIGINS` (comma list).
//! - better-auth `originCheckMiddleware` (every non-GET/HEAD/OPTIONS auth
//!   endpoint) and `formCsrfMiddleware` (sign-in / sign-up email).

use std::future::Future;

use axum::http::HeaderMap;
use serde_json::Value;

use super::AuthApiError;

/// `PHASE_PRODUCTION_BUILD` of `next/constants`.
pub const PHASE_PRODUCTION_BUILD: &str = "phase-production-build";

/// Mobile app deep-link schemes (`DEFAULT_MOBILE_APP_SCHEMES`).
pub const MOBILE_APP_ORIGINS: [&str; 2] = ["chatconnectxapp://", "chatbotxmobileapp://"];

/// The environment `trustedOrigins` reads.
#[derive(Debug, Clone, Default)]
pub struct TrustedOriginSettings {
    /// `NEXT_PUBLIC_BROKER_URL` (`getBrokerUrl()` falls back to the builder URL).
    pub broker_url: Option<String>,
    /// `NEXT_PUBLIC_BUILDER_URL`.
    pub builder_url: String,
    /// `NEXT_PHASE`.
    pub next_phase: Option<String>,
}

/// `customDomainService.listActiveDomains()`.
pub trait CustomDomainSource: Sync {
    /// The hostnames of every active custom domain.
    fn list_active_domains(&self) -> impl Future<Output = Vec<String>> + Send;
}

/// The `trustedOrigins` thunk of `createAuth`: static origins first, then the
/// active custom domains (skipped during the production build); no duplicates.
pub async fn trusted_origins<D: CustomDomainSource>(
    settings: &TrustedOriginSettings,
    domains: &D,
) -> Vec<String> {
    let broker = settings
        .broker_url
        .clone()
        .unwrap_or_else(|| settings.builder_url.clone());
    let mut origins = vec![broker, settings.builder_url.clone()];
    origins.extend(
        MOBILE_APP_ORIGINS
            .iter()
            .map(|origin| (*origin).to_string()),
    );
    if settings.next_phase.as_deref() != Some(PHASE_PRODUCTION_BUILD) {
        let active = domains.list_active_domains().await;
        origins.extend(active.into_iter().map(|domain| format!("https://{domain}")));
    }
    dedup(origins)
}

fn dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = Vec::with_capacity(values.len());
    for value in values {
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen
}

/// better-auth `getTrustedOrigins`: the base URL origin, the app origins and
/// `BETTER_AUTH_TRUSTED_ORIGINS`; empty entries dropped.
pub fn context_trusted_origins(
    base_url: &str,
    app_origins: Vec<String>,
    env_origins: Option<&str>,
) -> Vec<String> {
    let mut origins = Vec::new();
    if let Some(origin) = http_origin(base_url) {
        origins.push(origin);
    }
    origins.extend(app_origins);
    if let Some(env) = env_origins.filter(|env| !env.is_empty()) {
        origins.extend(env.split(',').map(str::to_string));
    }
    origins.retain(|origin| !origin.is_empty());
    origins
}

/// `getOrigin(url)`: the serialized origin of an absolute URL, `None` for an
/// unparsable URL or an opaque origin (custom schemes).
fn http_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let origin = parsed.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// `getHost(url)`: `host[:port]` of an absolute URL.
fn url_host(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// `getProtocol(url)`: `https:`, `myapp:`, ... (`None` when unparsable).
fn url_protocol(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .map(|parsed| format!("{}:", parsed.scheme()))
}

fn has_control_character(value: &str) -> bool {
    value
        .chars()
        .any(|c| c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c))
}

fn has_encoded_separator(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("%2f") || lower.contains("%5c")
}

/// `isSafeRelativeURL`: a root-relative path that cannot be read as another host.
fn is_safe_relative_url(value: &str) -> bool {
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || has_control_character(value)
    {
        return false;
    }
    let path = value.split(['?', '#']).next().unwrap_or_default();
    !has_encoded_separator(path)
}

fn percent_decode(value: &str) -> String {
    crate::auth::decode_query_component(&value.replace('+', "%2B"))
}

/// `normalizePath`: percent-decode, resolve `.` / `..`, drop empty segments.
fn normalize_path(path: &str) -> String {
    let decoded = percent_decode(path);
    let mut segments: Vec<&str> = Vec::new();
    for segment in decoded.split('/') {
        match segment {
            ".." => {
                segments.pop();
            }
            "." | "" => {}
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    }
}

struct CustomSchemeOrigin {
    scheme: String,
    authority: String,
    path: String,
}

/// `parseCustomSchemeOrigin`: scheme, authority and normalized path by plain
/// string operations.
fn parse_custom_scheme_origin(value: &str) -> Option<CustomSchemeOrigin> {
    if has_control_character(value) {
        return None;
    }
    let scheme_end = value.find(':').filter(|end| *end > 0)?;
    let scheme = value[..scheme_end].to_ascii_lowercase();
    let mut rest = &value[scheme_end + 1..];
    let mut authority = "";
    if let Some(after) = rest.strip_prefix("//") {
        match after.find(['/', '?', '#']) {
            Some(end) => {
                authority = &after[..end];
                rest = &after[end..];
            }
            None => {
                authority = after;
                rest = "";
            }
        }
    }
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    Some(CustomSchemeOrigin {
        scheme,
        authority: authority.to_ascii_lowercase(),
        path: normalize_path(path),
    })
}

/// `wildcardMatch(pattern)(value)`: `*` and `?` stop at `/`, `**` crosses it.
fn wildcard_match(pattern: &str, value: &str) -> bool {
    fn go(pattern: &[char], value: &[char]) -> bool {
        match pattern.first() {
            None => value.is_empty(),
            Some('*') if pattern.get(1) == Some(&'*') => {
                let rest = &pattern[2..];
                (0..=value.len()).any(|skip| go(rest, &value[skip..]))
            }
            Some('*') => {
                let rest = &pattern[1..];
                let limit = value
                    .iter()
                    .position(|c| *c == '/' || *c == '\\')
                    .unwrap_or(value.len());
                (0..=limit).any(|skip| go(rest, &value[skip..]))
            }
            Some('?') => value
                .first()
                .is_some_and(|c| *c != '/' && *c != '\\' && go(&pattern[1..], &value[1..])),
            Some(expected) => value
                .first()
                .is_some_and(|c| c == expected && go(&pattern[1..], &value[1..])),
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    go(&pattern, &value)
}

/// better-auth `matchesOriginPattern(url, pattern, { allowRelativePaths })`.
pub fn matches_origin_pattern(url: &str, pattern: &str, allow_relative_paths: bool) -> bool {
    if url.starts_with('/') {
        return allow_relative_paths && is_safe_relative_url(url);
    }
    if pattern.contains('*') || pattern.contains('?') {
        if pattern.contains("://") {
            let target = http_origin(url).unwrap_or_else(|| url.to_string());
            return wildcard_match(pattern, &target);
        }
        return url_host(url).is_some_and(|host| wildcard_match(pattern, &host));
    }
    let protocol = url_protocol(url);
    if matches!(protocol.as_deref(), Some("http:" | "https:") | None) {
        return http_origin(url).is_some_and(|origin| origin == pattern);
    }
    let (Some(parsed), Some(parsed_pattern)) = (
        parse_custom_scheme_origin(url),
        parse_custom_scheme_origin(pattern),
    ) else {
        return false;
    };
    if parsed.scheme != parsed_pattern.scheme {
        return false;
    }
    if !parsed_pattern.authority.is_empty() && parsed.authority != parsed_pattern.authority {
        return false;
    }
    if parsed_pattern.path.is_empty() {
        return true;
    }
    parsed.path == parsed_pattern.path
        || parsed
            .path
            .starts_with(&format!("{}/", parsed_pattern.path))
}

/// The request parts the origin checks read.
#[derive(Debug, Clone, Copy)]
pub struct OriginCheckRequest<'a> {
    pub method: &'a str,
    pub headers: &'a HeaderMap,
    /// The JSON body (`callbackURL`, `redirectTo`, `errorCallbackURL`, `newUserCallbackURL`).
    pub body: Option<&'a Value>,
    /// `?callbackURL=` of the query string.
    pub query_callback_url: Option<&'a str>,
}

fn forbidden(code: &str, message: &str) -> AuthApiError {
    let mut error = AuthApiError::new("FORBIDDEN", message);
    error.code = Some(code.to_string());
    error
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn is_trusted(url: &str, trusted: &[String], allow_relative_paths: bool) -> bool {
    trusted
        .iter()
        .any(|pattern| matches_origin_pattern(url, pattern, allow_relative_paths))
}

/// `validateOrigin(ctx, forceValidate)`: with cookies (or forced), the
/// `Origin` (else `Referer`) must match a trusted origin.
fn validate_origin(
    headers: &HeaderMap,
    trusted: &[String],
    force: bool,
) -> Result<(), AuthApiError> {
    if !(force || headers.contains_key("cookie")) {
        return Ok(());
    }
    let origin = header(headers, "origin")
        .filter(|origin| !origin.is_empty())
        .or_else(|| header(headers, "referer"))
        .unwrap_or_default();
    if origin.is_empty() || origin == "null" {
        return Err(forbidden(
            "MISSING_OR_NULL_ORIGIN",
            "Missing or null Origin",
        ));
    }
    if !trusted
        .iter()
        .any(|pattern| matches_origin_pattern(origin, pattern, false))
    {
        tracing::error!(origin, "Invalid origin");
        return Err(forbidden("INVALID_ORIGIN", "Invalid origin"));
    }
    Ok(())
}

/// better-auth `originCheckMiddleware`.
pub fn origin_check(
    request: &OriginCheckRequest<'_>,
    trusted: &[String],
) -> Result<(), AuthApiError> {
    if matches!(request.method, "GET" | "OPTIONS" | "HEAD") {
        return Ok(());
    }
    validate_origin(request.headers, trusted, false)?;
    let field = |name: &str| {
        request
            .body
            .and_then(|body| body.get(name))
            .filter(|value| truthy(value))
    };
    let callback = field("callbackURL").cloned().or_else(|| {
        request
            .query_callback_url
            .filter(|url| !url.is_empty())
            .map(|url| Value::String(url.to_string()))
    });
    let checks = [
        (callback, "callbackURL", "INVALID_CALLBACK_URL"),
        (
            field("redirectTo").cloned(),
            "redirectURL",
            "INVALID_REDIRECT_URL",
        ),
        (
            field("errorCallbackURL").cloned(),
            "errorCallbackURL",
            "INVALID_ERROR_CALLBACK_URL",
        ),
        (
            field("newUserCallbackURL").cloned(),
            "newUserCallbackURL",
            "INVALID_NEW_USER_CALLBACK_URL",
        ),
    ];
    for (value, label, code) in checks {
        let Some(value) = value else { continue };
        let Some(url) = value.as_str() else {
            return Err(AuthApiError::new(
                "BAD_REQUEST",
                &format!("Invalid {label}: expected a string"),
            ));
        };
        if !is_trusted(url, trusted, true) {
            tracing::error!(url, "Invalid {label}");
            return Err(forbidden(code, &format!("Invalid {label}")));
        }
    }
    Ok(())
}

/// JavaScript truthiness of a JSON value (`body?.callbackURL && ...`).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn non_blank(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

/// better-auth `formCsrfMiddleware` (sign-in / sign-up email).
pub fn form_csrf_check(headers: &HeaderMap, trusted: &[String]) -> Result<(), AuthApiError> {
    if headers.contains_key("cookie") {
        return validate_origin(headers, trusted, false);
    }
    let site = header(headers, "sec-fetch-site");
    let mode = header(headers, "sec-fetch-mode");
    let dest = header(headers, "sec-fetch-dest");
    if non_blank(site) || non_blank(mode) || non_blank(dest) {
        if site == Some("cross-site") && mode == Some("navigate") {
            tracing::error!("Blocked cross-site navigation login attempt (CSRF protection)");
            return Err(forbidden(
                "CROSS_SITE_NAVIGATION_LOGIN_BLOCKED",
                "Cross-site navigation login blocked. This request appears to be a CSRF attack.",
            ));
        }
        return validate_origin(headers, trusted, true);
    }
    if header(headers, "origin").is_some() || header(headers, "referer").is_some() {
        return validate_origin(headers, trusted, true);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use serde_json::json;

    use super::*;

    fn trusted() -> Vec<String> {
        context_trusted_origins(
            "http://app",
            vec!["chatbotxmobileapp://".to_string()],
            Some("https://*.partner.test"),
        )
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn check(headers: &HeaderMap, body: Option<&Value>) -> Result<(), String> {
        let request = OriginCheckRequest {
            method: "POST",
            headers,
            body,
            query_callback_url: None,
        };
        origin_check(&request, &trusted()).map_err(|error| error.code.unwrap_or_default())
    }

    #[test]
    fn origin_patterns_match_like_better_auth() {
        assert!(matches_origin_pattern(
            "http://app/path",
            "http://app",
            false
        ));
        assert!(!matches_origin_pattern(
            "http://evil.test",
            "http://app",
            false
        ));
        assert!(matches_origin_pattern(
            "chatbotxmobileapp://cb",
            "chatbotxmobileapp://",
            false
        ));
        assert!(!matches_origin_pattern(
            "otherapp://cb",
            "chatbotxmobileapp://",
            false
        ));
        assert!(matches_origin_pattern(
            "https://a.partner.test",
            "https://*.partner.test",
            false
        ));
        assert!(!matches_origin_pattern(
            "https://partner.test.evil",
            "https://*.partner.test",
            false
        ));
        assert!(matches_origin_pattern("/dashboard?x=1", "http://app", true));
        assert!(!matches_origin_pattern("/dashboard", "http://app", false));
        assert!(!matches_origin_pattern("//evil.test", "http://app", true));
        assert!(!matches_origin_pattern("/a%2f..", "http://app", true));
        assert!(!matches_origin_pattern(
            "myapp://host/cb/../evil",
            "myapp://host/cb",
            false
        ));
    }

    #[test]
    fn a_cookie_request_needs_a_trusted_origin() {
        let cookie = ("cookie", "better-auth.session_token=x");
        assert_eq!(
            check(&headers(&[cookie]), None),
            Err("MISSING_OR_NULL_ORIGIN".into())
        );
        assert_eq!(
            check(&headers(&[cookie, ("origin", "http://evil.test")]), None),
            Err("INVALID_ORIGIN".into())
        );
        assert_eq!(
            check(&headers(&[cookie, ("origin", "http://app")]), None),
            Ok(())
        );
        assert_eq!(
            check(&headers(&[cookie, ("referer", "http://app/sign-in")]), None),
            Ok(())
        );
        assert_eq!(
            check(&headers(&[("origin", "http://evil.test")]), None),
            Ok(())
        );
    }

    #[test]
    fn callback_urls_must_be_trusted_or_relative() {
        let none = headers(&[]);
        assert_eq!(check(&none, Some(&json!({ "callbackURL": "/" }))), Ok(()));
        assert_eq!(
            check(
                &none,
                Some(&json!({ "callbackURL": "https://evil.test/x" }))
            ),
            Err("INVALID_CALLBACK_URL".into())
        );
        assert_eq!(
            check(&none, Some(&json!({ "redirectTo": "//evil.test" }))),
            Err("INVALID_REDIRECT_URL".into())
        );
        assert_eq!(
            check(
                &none,
                Some(&json!({ "errorCallbackURL": "http://app/error" }))
            ),
            Ok(())
        );
        assert_eq!(
            check(
                &none,
                Some(&json!({ "newUserCallbackURL": "javascript:alert(1)" }))
            ),
            Err("INVALID_NEW_USER_CALLBACK_URL".into())
        );
    }

    #[test]
    fn form_csrf_blocks_cross_site_navigation_and_untrusted_origins() {
        let trusted = trusted();
        let navigate = headers(&[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
        ]);
        assert_eq!(
            form_csrf_check(&navigate, &trusted)
                .unwrap_err()
                .code
                .as_deref(),
            Some("CROSS_SITE_NAVIGATION_LOGIN_BLOCKED")
        );
        let evil = headers(&[("origin", "http://evil.test")]);
        assert_eq!(
            form_csrf_check(&evil, &trusted)
                .unwrap_err()
                .code
                .as_deref(),
            Some("INVALID_ORIGIN")
        );
        assert!(form_csrf_check(&headers(&[("origin", "http://app")]), &trusted).is_ok());
        assert!(form_csrf_check(&headers(&[]), &trusted).is_ok());
    }
}
