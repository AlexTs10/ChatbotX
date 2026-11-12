//! `packages/utils/src/request.ts`: the public origin of a request behind
//! proxies. `FORCE_PUBLIC_HTTPS` is passed in (read it once with
//! [`force_public_https`]) instead of read from `process.env` per call.

use http::HeaderMap;

use crate::env::Env;

/// The parts of a `Request` these helpers read.
#[derive(Debug, Clone, Copy)]
pub struct RequestParts<'a> {
    /// `request.url` (absolute).
    pub url: &'a str,
    pub headers: &'a HeaderMap,
}

/// `parseEnvBool(keys().FORCE_PUBLIC_HTTPS)`.
pub fn force_public_https(env: &dyn Env) -> bool {
    super::env::parse_env_bool_str(env.var("FORCE_PUBLIC_HTTPS").as_deref())
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn extract_forwarded_value(forwarded: Option<&str>, key: &str) -> Option<String> {
    let forwarded = forwarded.filter(|f| !f.is_empty())?;
    let first_entry = crate::js::trim(forwarded.split(',').next().unwrap_or(""));
    if first_entry.is_empty() {
        return None;
    }
    for pair in first_entry.split(';') {
        // `pair.split("=", 2)`: the value stops at a second `=`.
        let mut parts = pair.split('=');
        let raw_key = parts.next().unwrap_or("");
        let raw_value = parts.next().unwrap_or("");
        if raw_key.is_empty() || raw_value.is_empty() {
            continue;
        }
        if crate::js::trim(raw_key).to_lowercase() != key {
            continue;
        }
        let value = crate::js::trim(raw_value);
        let value = value.strip_prefix('"').unwrap_or(value);
        let value = value.strip_suffix('"').unwrap_or(value);
        return Some(value.to_owned());
    }
    None
}

fn normalize_host(host: Option<&str>) -> Option<String> {
    let host = host.filter(|h| !h.is_empty())?;
    Some(crate::js::trim(host).to_lowercase())
}

/// `getPublicProtocolFromRequest`.
pub fn get_public_protocol_from_request(
    request: &RequestParts<'_>,
    force_https: bool,
) -> &'static str {
    if force_https {
        return "https";
    }
    let forwarded = extract_forwarded_value(header(request.headers, "forwarded"), "proto");
    match forwarded.as_deref() {
        Some("http") => return "http",
        Some("https") => return "https",
        _ => {}
    }
    match header(request.headers, "x-forwarded-proto") {
        Some("http") => return "http",
        Some("https") => return "https",
        _ => {}
    }
    if request.url.starts_with("http://") {
        "http"
    } else {
        "https"
    }
}

/// `getRawPublicHostFromRequest`: `Forwarded` host, then the first
/// `X-Forwarded-Host`, then `Host`; lower-cased and trimmed; `None` when
/// nothing resolves.
pub fn get_raw_public_host_from_request(request: &RequestParts<'_>) -> Option<String> {
    if let Some(host) = normalize_host(
        extract_forwarded_value(header(request.headers, "forwarded"), "host").as_deref(),
    ) {
        return Some(host);
    }
    let x_forwarded = header(request.headers, "x-forwarded-host")
        .map(|h| crate::js::trim(h.split(',').next().unwrap_or("")).to_owned());
    if let Some(host) = normalize_host(x_forwarded.as_deref()) {
        return Some(host);
    }
    normalize_host(header(request.headers, "host"))
}

/// `getPublicHostFromRequest`: as raw, with the `localhost:3123` default.
pub fn get_public_host_from_request(request: &RequestParts<'_>) -> String {
    get_raw_public_host_from_request(request).unwrap_or_else(|| "localhost:3123".to_owned())
}

/// `getPublicOriginFromRequest`.
pub fn get_public_origin_from_request(request: &RequestParts<'_>, force_https: bool) -> String {
    format!(
        "{}://{}",
        get_public_protocol_from_request(request, force_https),
        get_public_host_from_request(request)
    )
}

/// `getPublicUrlFromRequest`: the request URL on the public host and
/// protocol, without a port.
pub fn get_public_url_from_request(
    request: &RequestParts<'_>,
    force_https: bool,
) -> Result<url::Url, url::ParseError> {
    let mut url = url::Url::parse(request.url)?;
    let host = get_public_host_from_request(request);
    // `url.host = …` takes `host[:port]`.
    let (hostname, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(']') || name.ends_with(']') => {
            (name.to_owned(), port.parse::<u16>().ok())
        }
        _ => (host.clone(), None),
    };
    let _ = url.set_host(Some(&hostname));
    let _ = url.set_port(port);
    let _ = url.set_scheme(get_public_protocol_from_request(request, force_https));
    let _ = url.set_port(None);
    Ok(url)
}
