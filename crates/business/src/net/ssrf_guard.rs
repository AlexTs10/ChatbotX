//! `net/ssrf-guard.ts`: refuse URLs that point at private, loopback or
//! metadata addresses. Host names resolve over DNS-over-HTTPS
//! (`https://1.1.1.1/dns-query`); the HTTP call is injected ([`DohFetch`]) so
//! tests need no global `fetch` mock.

use std::future::Future;

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

impl DohFetch for HttpDoh {
    async fn get(&self, _url: &str) -> Result<DohReply, String> {
        todo!("P3.6-D2")
    }
}

/// `SsrfCheckResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrfCheck {
    Unsafe,
    Safe { resolved_ips: Vec<String> },
}

/// `checkSsrfSafety` over `doh`.
pub async fn check_ssrf_safety_with(_raw_url: &str, _doh: &impl DohFetch) -> SsrfCheck {
    todo!("P3.6-D2")
}

/// `isSsrfUnsafeUrl` over `doh`.
pub async fn is_ssrf_unsafe_url_with(raw_url: &str, doh: &impl DohFetch) -> bool {
    check_ssrf_safety_with(raw_url, doh).await == SsrfCheck::Unsafe
}

/// `assertPublicUrl` over `doh`: `Err` = `[ssrf-guard] <context> is not
/// allowed: <url>`.
pub async fn assert_public_url_with(
    _raw_url: &str,
    _context: &str,
    _doh: &impl DohFetch,
) -> Result<(), String> {
    todo!("P3.6-D2")
}

/// `assertPublicUrl` with the real DoH client.
pub async fn assert_public_url(raw_url: &str, context: &str) -> Result<(), String> {
    assert_public_url_with(raw_url, context, &HttpDoh).await
}
