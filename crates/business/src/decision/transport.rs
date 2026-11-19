//! `decision/transport.ts` + `compatible-url.ts`: the provider call (pinned
//! to the address the SSRF guard checked, no redirects, 2.5 s, 256 KB).

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
    let url = url::Url::parse(&endpoint).map_err(|e| e.to_string())?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    // ponytail: the original's undici follows a 3xx with a GET to the same
    // pinned address before rejecting it; this client never follows one.
    let client = reqwest::Client::builder()
        .resolve(&host, SocketAddr::new(ip, port))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let payload =
        serde_json::to_vec(&format_request_body(provider_kind, body)).map_err(|e| e.to_string())?;
    let mut response = client
        .post(url)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {authorization}"),
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
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
