//! `net/ssrf-guard.ts`: refuse URLs that point at private, loopback or
//! metadata addresses. Host names resolve over DNS-over-HTTPS
//! (`https://1.1.1.1/dns-query`); the HTTP call is injected ([`DohFetch`]) so
//! tests need no global `fetch` mock.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

const DOH_TIMEOUT: Duration = Duration::from_millis(3000);
const DNS_RECORD_TYPE_A: u64 = 1;
const DNS_RECORD_TYPE_AAAA: u64 = 28;

const BLOCKED_HOSTNAMES: &[&str] = &["localhost", "localhost.localdomain"];

const BLOCKED_IPV4_RANGES: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

/// The DoH endpoint (hard-coded, like the original).
pub const DOH_ENDPOINT: &str = "https://1.1.1.1/dns-query";

/// One DoH HTTP answer (`response.status`, `response.text()`).
#[derive(Debug, Clone)]
pub struct DohReply {
    pub status: u16,
    pub body: String,
}

/// The HTTP GET of a DoH lookup (`fetch(url, { headers: { accept:
/// "application/dns-json" } })`). `Err` = the fetch rejected.
pub trait DohFetch: Send + Sync {
    fn get(&self, url: &str) -> impl Future<Output = Result<DohReply, String>> + Send;
}

/// The real DoH client (reqwest, 3 s timeout).
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpDoh;

fn http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(DOH_TIMEOUT)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl DohFetch for HttpDoh {
    async fn get(&self, url: &str) -> Result<DohReply, String> {
        let response = http_client()?
            .get(url)
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|e| e.to_string())?;
        Ok(DohReply { status, body })
    }
}

/// `IPV4_PATTERN` (four decimal octets, no leading zeros).
fn parse_ipv4(value: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0_u8; 4];
    for (slot, part) in out.iter_mut().zip(parts) {
        if part.is_empty()
            || part.len() > 3
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(out)
}

fn is_ipv6_literal(value: &str) -> bool {
    value.contains(':') && value.parse::<std::net::Ipv6Addr>().is_ok()
}

fn is_blocked_ipv4(ip: [u8; 4]) -> bool {
    let value = u32::from_be_bytes(ip);
    BLOCKED_IPV4_RANGES.iter().any(|(range, prefix)| {
        let mask = if *prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        value & mask == u32::from_be_bytes(*range) & mask
    })
}

/// `extractEmbeddedIpv4`: the last two groups as a dotted quad.
fn extract_embedded_ipv4(normalized: &str) -> Option<[u8; 4]> {
    let groups: Vec<&str> = normalized.split(':').collect();
    let last = *groups.last()?;
    let second_last = *groups.get(groups.len().checked_sub(2)?)?;
    if last.is_empty() || second_last.is_empty() || last.len() > 4 || second_last.len() > 4 {
        return None;
    }
    let high = u16::from_str_radix(second_last, 16).ok()?;
    let low = u16::from_str_radix(last, 16).ok()?;
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Some([a, b, c, d])
}

fn is_blocked_ipv6(ip: &str) -> bool {
    let normalized = ip.to_lowercase();
    if normalized == "::1"
        || normalized == "::"
        || normalized.starts_with("fe80:")
        || normalized.starts_with("fc")
        || normalized.starts_with("fd")
    {
        return true;
    }
    if normalized.starts_with("::ffff:")
        || normalized.starts_with("0:0:0:0:0:ffff:")
        || normalized.starts_with("64:ff9b::")
    {
        return extract_embedded_ipv4(&normalized).is_none_or(is_blocked_ipv4);
    }
    false
}

fn is_blocked_ip(ip: &str) -> bool {
    if let Some(v4) = parse_ipv4(ip) {
        return is_blocked_ipv4(v4);
    }
    if is_ipv6_literal(ip) {
        return is_blocked_ipv6(ip);
    }
    true
}

/// `resolveHostname`: the A / AAAA answers of a DoH `type=A` lookup.
async fn resolve_hostname(hostname: &str, doh: &impl DohFetch) -> Result<Vec<String>, String> {
    let mut url = url::Url::parse(DOH_ENDPOINT).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("name", hostname)
        .append_pair("type", "A");
    let reply = doh.get(url.as_str()).await?;
    if !(200..300).contains(&reply.status) {
        return Err(format!("DoH lookup failed with status {}", reply.status));
    }
    let body: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    Ok(body
        .get("Answer")
        .and_then(Value::as_array)
        .map(|answers| {
            answers
                .iter()
                .filter(|answer| {
                    matches!(
                        answer.get("type").and_then(Value::as_u64),
                        Some(DNS_RECORD_TYPE_A | DNS_RECORD_TYPE_AAAA)
                    )
                })
                .filter_map(|answer| answer.get("data").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

/// `SsrfCheckResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrfCheck {
    Unsafe,
    Safe { resolved_ips: Vec<String> },
}

/// `checkSsrfSafety` over `doh`.
pub async fn check_ssrf_safety_with(raw_url: &str, doh: &impl DohFetch) -> SsrfCheck {
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return SsrfCheck::Unsafe;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return SsrfCheck::Unsafe;
    }
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let hostname = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    if BLOCKED_HOSTNAMES.contains(&hostname.as_str()) {
        return SsrfCheck::Unsafe;
    }
    if parse_ipv4(&hostname).is_some() || is_ipv6_literal(&hostname) {
        return if is_blocked_ip(&hostname) {
            SsrfCheck::Unsafe
        } else {
            SsrfCheck::Safe {
                resolved_ips: vec![hostname],
            }
        };
    }
    match resolve_hostname(&hostname, doh).await {
        Ok(ips) if !ips.is_empty() && !ips.iter().any(|ip| is_blocked_ip(ip)) => {
            SsrfCheck::Safe { resolved_ips: ips }
        }
        _ => SsrfCheck::Unsafe,
    }
}

/// `isSsrfUnsafeUrl` over `doh`.
pub async fn is_ssrf_unsafe_url_with(raw_url: &str, doh: &impl DohFetch) -> bool {
    check_ssrf_safety_with(raw_url, doh).await == SsrfCheck::Unsafe
}

/// `assertPublicUrl` over `doh`: `Err` = `[ssrf-guard] <context> is not
/// allowed: <url>`.
pub async fn assert_public_url_with(
    raw_url: &str,
    context: &str,
    doh: &impl DohFetch,
) -> Result<(), String> {
    match check_ssrf_safety_with(raw_url, doh).await {
        SsrfCheck::Unsafe => Err(format!("[ssrf-guard] {context} is not allowed: {raw_url}")),
        SsrfCheck::Safe { .. } => Ok(()),
    }
}

/// `assertPublicUrl` with the real DoH client.
pub async fn assert_public_url(raw_url: &str, context: &str) -> Result<(), String> {
    assert_public_url_with(raw_url, context, &HttpDoh).await
}
