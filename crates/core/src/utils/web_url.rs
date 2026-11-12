//! `packages/utils/src/web-url.ts`.

/// `toSafeWebUrl`: the link as a normalized `http(s)` URL, or `None` (a
/// `javascript:` / `data:` / scheme-less link is not a web URL).
pub fn to_safe_web_url(value: Option<&str>) -> Option<String> {
    let value = value.filter(|v| !v.is_empty())?;
    let url = url::Url::parse(crate::js::trim(value)).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// `joinAppUrl`: `path` (starting with `/`) on the app origin, whatever the
/// number of trailing slashes on `app_url`.
pub fn join_app_url(app_url: &str, path: &str) -> String {
    format!("{}{path}", app_url.trim_end_matches('/'))
}
