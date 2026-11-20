//! `decision/transport.ts` + `compatible-url.ts`: the provider call (pinned
//! to the address the SSRF guard checked, redirects follow on that address,
//! 2.5 s, 256 KB).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use serde_json::Value;

use super::contract::format_request_body;
use crate::errors::{BusinessError, Result};
use crate::net::ssrf_guard::{DohFetch, SsrfCheck, check_ssrf_safety_with};

const TIMEOUT: Duration = Duration::from_millis(2500);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const TYPESAFE_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";

/// The provider kinds (`providerKind`).
pub const PROVIDER_KINDS: &[&str] = &["typesafe", "systemOneCompatible", "openrouterDecision"];

fn invalid_endpoint(message: &str) -> BusinessError {
    BusinessError::exception(message, "invalidDecisionEndpoint", 400)
}

/// `normalizeCompatibleEndpoint`: canonical HTTPS (no userinfo, query or
/// fragment, no trailing `/`) that resolves to a public address.
pub async fn normalize_compatible_endpoint(raw: &str, doh: &impl DohFetch) -> Result<String> {
    let mut endpoint = url::Url::parse(raw.trim())
        .map_err(|_| invalid_endpoint("Decision provider endpoint must be a valid HTTPS URL"))?;
    if endpoint.scheme() != "https"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some_and(|q| !q.is_empty())
        || endpoint.fragment().is_some_and(|f| !f.is_empty())
    {
        return Err(invalid_endpoint(
            "Decision provider endpoint must be a canonical HTTPS URL",
        ));
    }
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let path = endpoint.path().trim_end_matches('/').to_owned();
    endpoint.set_path(if path.is_empty() { "/" } else { &path });
    let normalized = endpoint.to_string();
    if check_ssrf_safety_with(&normalized, doh).await == SsrfCheck::Unsafe {
        return Err(BusinessError::exception(
            "Decision provider endpoint is not publicly reachable",
            "unsafeDecisionEndpoint",
            400,
        ));
    }
    Ok(normalized)
}

/// `resolveProviderEndpoint`.
fn provider_endpoint(provider_kind: &str, endpoint: Option<&str>) -> Option<String> {
    match provider_kind {
        "typesafe" => Some(TYPESAFE_ENDPOINT.to_owned()),
        "openrouterDecision" => Some(OPENROUTER_ENDPOINT.to_owned()),
        _ => endpoint.map(str::to_owned),
    }
}

/// `postDecisionRequest`: `(status, body)` of the provider answer.
pub async fn post_decision_request(
    doh: &impl DohFetch,
    provider_kind: &str,
    endpoint: Option<&str>,
    authorization: &str,
    body: &Value,
) -> std::result::Result<(u16, String), String> {
    let endpoint = provider_endpoint(provider_kind, endpoint)
        .ok_or("System One Compatible endpoint is missing")?;
    let SsrfCheck::Safe { resolved_ips } = check_ssrf_safety_with(&endpoint, doh).await else {
        return Err("Decision endpoint failed public-address validation".into());
    };
    let ip: IpAddr = resolved_ips
        .first()
        .and_then(|ip| ip.parse().ok())
        .ok_or("Decision endpoint failed public-address validation")?;
    let payload =
        serde_json::to_vec(&format_request_body(provider_kind, body)).map_err(|e| e.to_string())?;
    send_pinned(ip, &endpoint, authorization, payload).await
}

/// `fetch` through the pinned dispatcher: POSTs `payload` and follows a redirect
/// as undici does (at most 20; 301/302 of a POST and 303 become a body-less GET;
/// `Authorization` is dropped on a cross-origin hop). Every hop dials only `ip`,
/// so a redirect cannot reach an address the SSRF guard did not check. A final
/// 3xx is rejected, a body over 256 KB too.
pub async fn send_pinned(
    ip: IpAddr,
    endpoint: &str,
    authorization: &str,
    payload: Vec<u8>,
) -> std::result::Result<(u16, String), String> {
    const MAX_REDIRECTS: usize = 20;
    let mut url = url::Url::parse(endpoint).map_err(|e| e.to_string())?;
    let origin = url.origin();
    let mut method = reqwest::Method::POST;
    let mut body = Some(payload);
    let mut hops = 0;
    let mut response = loop {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("Decision provider redirect was rejected".into());
        }
        let host = url
            .host_str()
            .ok_or("Decision endpoint has no host")?
            .to_owned();
        let port = url.port_or_known_default().unwrap_or(443);
        let client = reqwest::Client::builder()
            .resolve(&host, SocketAddr::new(ip, port))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let mut request = client.request(method.clone(), url.clone());
        if url.origin() == origin {
            request = request.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {authorization}"),
            );
        }
        if let Some(bytes) = &body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes.clone());
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let Some(location) = location.filter(|_| matches!(status, 301 | 302 | 303 | 307 | 308))
        else {
            break response;
        };
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err("redirect count exceeded".into());
        }
        url = url.join(&location).map_err(|e| e.to_string())?;
        if status == 303 || (matches!(status, 301 | 302) && method == reqwest::Method::POST) {
            method = reqwest::Method::GET;
            body = None;
        }
    };
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        return Err("Decision provider redirect was rejected".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("Decision provider response exceeded size limit".into());
        }
    }
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ssrf_guard::DohReply;

    struct NoDns;
    impl DohFetch for NoDns {
        async fn get(&self, _url: &str) -> std::result::Result<DohReply, String> {
            Err("no dns".to_owned())
        }
    }

    async fn normalize(raw: &str) -> std::result::Result<String, String> {
        normalize_compatible_endpoint(raw, &NoDns)
            .await
            .map_err(|e| format!("{}: {}", e.code().unwrap_or(""), e.message()))
    }

    #[tokio::test]
    async fn compatible_endpoints_are_canonical_and_public() {
        assert_eq!(
            normalize("https://8.8.8.8/v1/decide///").await.unwrap(),
            "https://8.8.8.8/v1/decide"
        );
        assert_eq!(
            normalize("https://8.8.8.8").await.unwrap(),
            "https://8.8.8.8/"
        );
        for bad in [
            "http://8.8.8.8/v1",
            "https://8.8.8.8/v1?x=1",
            "https://8.8.8.8/v1#f",
            "https://u:p@8.8.8.8/v1",
        ] {
            assert!(
                normalize(bad)
                    .await
                    .unwrap_err()
                    .starts_with("invalidDecisionEndpoint")
            );
        }
        assert!(
            normalize("not a url")
                .await
                .unwrap_err()
                .contains("valid HTTPS URL")
        );
        for unsafe_url in [
            "https://127.0.0.1/v1",
            "https://10.1.2.3/v1",
            "https://localhost/v1",
        ] {
            assert!(
                normalize(unsafe_url)
                    .await
                    .unwrap_err()
                    .starts_with("unsafeDecisionEndpoint")
            );
        }
    }
}
