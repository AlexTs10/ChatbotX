//! Port of `packages/business/__tests__/net/ssrf-guard.test.ts`. The global
//! `fetch` mock is a [`FakeDoh`] passed to the `*_with` functions.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_business::net::ssrf_guard::{
    DohFetch, DohReply, SsrfCheck, assert_public_url_with, check_ssrf_safety_with,
    is_ssrf_unsafe_url_with,
};
use serde_json::json;

const A_RECORD: u8 = 1;

/// `fetchMock`: one canned answer, and the number of calls.
struct FakeDoh {
    reply: Mutex<Option<Result<DohReply, String>>>,
    calls: AtomicUsize,
}

impl FakeDoh {
    fn new() -> FakeDoh {
        FakeDoh {
            reply: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }
    fn answering(reply: Result<DohReply, String>) -> FakeDoh {
        let fake = FakeDoh::new();
        *fake.reply.lock().unwrap() = Some(reply);
        fake
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DohFetch for FakeDoh {
    async fn get(&self, _url: &str) -> Result<DohReply, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.reply
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Err("fetch is not mocked".to_string()))
    }
}

/// `dohJsonResponse`.
fn doh_json_response(records: &[(u8, &str)]) -> Result<DohReply, String> {
    let answer: Vec<_> = records
        .iter()
        .map(|(kind, data)| json!({ "type": kind, "data": data }))
        .collect();
    Ok(DohReply {
        status: 200,
        body: json!({ "Answer": answer }).to_string(),
    })
}

// ts:27 describe("isSsrfUnsafeUrl")

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_non_http_s_schemes() {
    let fetch = FakeDoh::new();
    assert!(is_ssrf_unsafe_url_with("ftp://example.com", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("file:///etc/passwd", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("not a url", &fetch).await);
    assert_eq!(fetch.calls(), 0);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_localhost() {
    let fetch = FakeDoh::new();
    assert!(is_ssrf_unsafe_url_with("http://localhost/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://localhost:8080/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_loopback_and_cloud_metadata_ips_directly() {
    let fetch = FakeDoh::new();
    assert!(is_ssrf_unsafe_url_with("http://127.0.0.1/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://169.254.169.254/latest/meta-data", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://[::1]/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_private_ipv4_ranges_directly() {
    let fetch = FakeDoh::new();
    assert!(is_ssrf_unsafe_url_with("http://10.0.0.5/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://172.16.0.5/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://192.168.1.1/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_a_hostname_that_resolves_to_a_private_ip_dns_rebinding() {
    let fetch = FakeDoh::answering(doh_json_response(&[(A_RECORD, "169.254.169.254")]));
    assert!(is_ssrf_unsafe_url_with("http://attacker.example.com/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_when_dns_resolution_fails() {
    let fetch = FakeDoh::answering(Err("network error".to_string()));
    assert!(is_ssrf_unsafe_url_with("http://does-not-resolve.example.com/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_when_the_doh_response_is_not_ok() {
    let fetch = FakeDoh::answering(Ok(DohReply {
        status: 500,
        body: String::new(),
    }));
    assert!(is_ssrf_unsafe_url_with("http://does-not-resolve.example.com/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_when_dns_returns_no_records() {
    let fetch = FakeDoh::answering(doh_json_response(&[]));
    assert!(is_ssrf_unsafe_url_with("http://no-records.example.com/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_allows_a_public_ip_resolved_via_dns() {
    let fetch = FakeDoh::answering(doh_json_response(&[(A_RECORD, "93.184.216.34")]));
    assert!(!is_ssrf_unsafe_url_with("https://public.example.com/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_allows_a_public_ip_passed_directly() {
    let fetch = FakeDoh::new();
    assert!(!is_ssrf_unsafe_url_with("https://93.184.216.34/", &fetch).await);
    assert_eq!(fetch.calls(), 0);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_respects_private_range_boundaries_precisely() {
    let fetch = FakeDoh::new();
    // 172.16.0.0/12 spans 172.16.0.0 - 172.31.255.255
    assert!(!is_ssrf_unsafe_url_with("http://172.15.255.255/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://172.31.255.255/", &fetch).await);
    assert!(!is_ssrf_unsafe_url_with("http://172.32.0.0/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_rejects_ipv4_mapped_ipv6_literals_for_loopback_and_private_ranges() {
    // The WHATWG URL parser canonicalizes these to hex form ("::ffff:7f00:1").
    let fetch = FakeDoh::new();
    assert!(is_ssrf_unsafe_url_with("http://[::ffff:127.0.0.1]/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://[::ffff:10.0.0.1]/", &fetch).await);
    assert!(is_ssrf_unsafe_url_with("http://[::ffff:169.254.169.254]/", &fetch).await);
}

#[tokio::test]
async fn is_ssrf_unsafe_url_allows_an_ipv4_mapped_ipv6_literal_for_a_public_address() {
    let fetch = FakeDoh::new();
    assert!(!is_ssrf_unsafe_url_with("http://[::ffff:93.184.216.34]/", &fetch).await);
}

// ts:113 describe("checkSsrfSafety")

#[tokio::test]
async fn check_ssrf_safety_returns_the_resolved_ips_alongside_a_safe_verdict() {
    let fetch = FakeDoh::answering(doh_json_response(&[(A_RECORD, "93.184.216.34")]));
    let result = check_ssrf_safety_with("https://public.example.com/", &fetch).await;
    assert_eq!(
        result,
        SsrfCheck::Safe {
            resolved_ips: vec!["93.184.216.34".to_string()]
        }
    );
}

#[tokio::test]
async fn check_ssrf_safety_returns_the_literal_ip_when_the_url_already_contains_one() {
    let fetch = FakeDoh::new();
    let result = check_ssrf_safety_with("https://93.184.216.34/", &fetch).await;
    assert_eq!(
        result,
        SsrfCheck::Safe {
            resolved_ips: vec!["93.184.216.34".to_string()]
        }
    );
    assert_eq!(fetch.calls(), 0);
}

// ts:129 describe("assertPublicUrl")

#[tokio::test]
async fn assert_public_url_resolves_without_throwing_for_a_public_url() {
    let fetch = FakeDoh::new();
    assert_eq!(
        assert_public_url_with("https://93.184.216.34/", "URL", &fetch).await,
        Ok(())
    );
}

#[tokio::test]
async fn assert_public_url_throws_with_the_given_context_for_a_blocked_url() {
    let fetch = FakeDoh::new();
    let error = assert_public_url_with("http://127.0.0.1/", "image URL", &fetch)
        .await
        .expect_err("blocked");
    assert!(error.contains("image URL"), "{error}");
}
