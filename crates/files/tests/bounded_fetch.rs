//! Port of `packages/filesystem/__tests__/bounded-fetch.test.ts` (P2.9).
//!
//! JS `Response` objects become `HttpResponse`; `vi.fn` spies become closures
//! that record their calls. JS `new Error(msg)` becomes `TestError(msg)`, and
//! `rejects.toThrow(msg)` becomes "the error message contains `msg`".

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use chatbotx_files::{
    Body, FetchError, FollowRedirectsInput, HttpResponse, SafeRedirectErrors,
    fetch_following_safe_redirects, read_body_with_limit,
};
use futures::FutureExt;
use futures::StreamExt;
use futures::future::BoxFuture;
use http::{HeaderMap, HeaderValue, header};

/// JS `new Error(message)`.
#[derive(Debug)]
struct TestError(String);

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<FetchError> for TestError {
    fn from(error: FetchError) -> Self {
        TestError(error.to_string())
    }
}

fn assert_message_contains(error: &TestError, expected: &str) {
    assert!(
        error.to_string().contains(expected),
        "expected error containing {expected:?}, got {:?}",
        error.to_string()
    );
}

/// JS `streamResponse(chunks, { status, headers })`.
fn stream_response(chunks: Vec<Bytes>, status: u16, headers: HeaderMap) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body: Body::Stream(futures::stream::iter(chunks.into_iter().map(Ok)).boxed()),
    }
}

/// JS `new Response(null, { status })`.
fn empty_response(status: u16, headers: HeaderMap) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body: Body::Buffered(Bytes::new()),
    }
}

/// JS `Response.redirect(url, 302)`.
fn redirect_response(location: &str) -> HttpResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(location).expect("valid header"),
    );
    empty_response(302, headers)
}

/// JS `(limit) => new Error(\`exceeded ${limit}\`)`.
fn exceeded(limit: u64) -> TestError {
    TestError(format!("exceeded {limit}"))
}

/// The `errors` object of the `fetchFollowingSafeRedirects` describe block.
fn errors() -> SafeRedirectErrors<TestError> {
    SafeRedirectErrors {
        too_many_redirects: Box::new(|| TestError("too many redirects".to_owned())),
        no_location_header: Box::new(|| TestError("no location".to_owned())),
        invalid_redirect_location: Box::new(|_location, _cause| {
            TestError("invalid location".to_owned())
        }),
    }
}

/// JS `vi.fn(async () => undefined)` with a call log.
fn validate_ok(
    calls: Arc<Mutex<Vec<String>>>,
) -> impl Fn(String) -> BoxFuture<'static, Result<(), TestError>> + Send + Sync {
    move |url: String| -> BoxFuture<'static, Result<(), TestError>> {
        calls.lock().expect("lock").push(url);
        async { Ok(()) }.boxed()
    }
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:26 "readBodyWithLimit > returns the full buffer under the limit"
#[tokio::test]
async fn read_body_with_limit_returns_the_full_buffer_under_the_limit() {
    let chunk = Bytes::from_static(b"hello");
    let response = stream_response(vec![chunk.clone()], 200, HeaderMap::new());

    let result = read_body_with_limit(response.body, 1000, exceeded)
        .await
        .unwrap();

    assert_eq!(result.len(), chunk.len());
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:36 "readBodyWithLimit > invokes onLimitExceeded once the running total crosses maxBytes"
#[tokio::test]
async fn read_body_with_limit_invokes_on_limit_exceeded_once_the_running_total_crosses_max_bytes() {
    let chunk = Bytes::from(vec![0u8; 20]);
    let response = stream_response(vec![chunk], 200, HeaderMap::new());

    let error = read_body_with_limit(response.body, 10, exceeded)
        .await
        .expect_err("expected rejection");

    assert_message_contains(&error, "exceeded 10");
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:47 "readBodyWithLimit > falls back to arrayBuffer when the response has no body stream"
#[tokio::test]
async fn read_body_with_limit_falls_back_to_array_buffer_when_the_response_has_no_body_stream() {
    let response = empty_response(204, HeaderMap::new());

    let result = read_body_with_limit(response.body, 10, exceeded)
        .await
        .unwrap();

    assert_eq!(result.len(), 0);
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:57 "readBodyWithLimit > enforces the cap even when the response has no body stream"
#[tokio::test]
async fn read_body_with_limit_enforces_the_cap_even_when_the_response_has_no_body_stream() {
    // JS: `{ body: null, arrayBuffer: async () => new Uint8Array(20).buffer }`.
    let body = Body::Buffered(Bytes::from(vec![0u8; 20]));

    let error = read_body_with_limit(body, 10, exceeded)
        .await
        .expect_err("expected rejection");

    assert_message_contains(&error, "exceeded 10");
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:79 "fetchFollowingSafeRedirects > re-validates on every hop and returns the final response"
#[tokio::test]
async fn fetch_following_safe_redirects_re_validates_on_every_hop_and_returns_the_final_response() {
    let errors = errors();
    let validate_calls = Arc::new(Mutex::new(Vec::new()));
    let validate_url = validate_ok(validate_calls.clone());
    let call = Arc::new(AtomicUsize::new(0));
    let fetch_impl = {
        let call = call.clone();
        move |_url: String| -> BoxFuture<'static, Result<HttpResponse, TestError>> {
            let n = call.fetch_add(1, Ordering::SeqCst) + 1;
            let response = if n == 1 {
                redirect_response("https://example.com/b")
            } else {
                stream_response(vec![Bytes::from_static(b"ok")], 200, HeaderMap::new())
            };
            async move { Ok(response) }.boxed()
        }
    };

    let (final_url, _response) = fetch_following_safe_redirects(FollowRedirectsInput {
        errors: &errors,
        fetch_impl: &fetch_impl,
        max_redirect_hops: None,
        url: "https://example.com/a".to_owned(),
        validate_url: &validate_url,
    })
    .await
    .unwrap();

    assert_eq!(final_url, "https://example.com/b");
    let validate_calls = validate_calls.lock().expect("lock");
    assert_eq!(validate_calls.len(), 2);
    assert_eq!(validate_calls[0], "https://example.com/a");
    assert_eq!(validate_calls[1], "https://example.com/b");
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:103 "fetchFollowingSafeRedirects > throws tooManyRedirects past the configured hop limit"
#[tokio::test]
async fn fetch_following_safe_redirects_throws_too_many_redirects_past_the_configured_hop_limit() {
    let errors = errors();
    let validate_url = validate_ok(Arc::new(Mutex::new(Vec::new())));
    let fetch_impl = |_url: String| -> BoxFuture<'static, Result<HttpResponse, TestError>> {
        async { Ok(redirect_response("https://example.com/next")) }.boxed()
    };

    let error = fetch_following_safe_redirects(FollowRedirectsInput {
        errors: &errors,
        fetch_impl: &fetch_impl,
        max_redirect_hops: Some(2),
        url: "https://example.com/a".to_owned(),
        validate_url: &validate_url,
    })
    .await
    .err()
    .expect("expected rejection");

    assert_message_contains(&error, "too many redirects");
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:120 "fetchFollowingSafeRedirects > throws noLocationHeader when a redirect carries no Location"
#[tokio::test]
async fn fetch_following_safe_redirects_throws_no_location_header_when_a_redirect_carries_no_location()
 {
    let errors = errors();
    let validate_url = validate_ok(Arc::new(Mutex::new(Vec::new())));
    let fetch_impl = |_url: String| -> BoxFuture<'static, Result<HttpResponse, TestError>> {
        async { Ok(empty_response(302, HeaderMap::new())) }.boxed()
    };

    let error = fetch_following_safe_redirects(FollowRedirectsInput {
        errors: &errors,
        fetch_impl: &fetch_impl,
        max_redirect_hops: None,
        url: "https://example.com/a".to_owned(),
        validate_url: &validate_url,
    })
    .await
    .err()
    .expect("expected rejection");

    assert_message_contains(&error, "no location");
}

/// Port of: packages/filesystem/__tests__/bounded-fetch.test.ts:136 "fetchFollowingSafeRedirects > propagates a validateUrl rejection without fetching"
#[tokio::test]
async fn fetch_following_safe_redirects_propagates_a_validate_url_rejection_without_fetching() {
    let errors = errors();
    let validate_url = |_url: String| -> BoxFuture<'static, Result<(), TestError>> {
        async { Err(TestError("blocked".to_owned())) }.boxed()
    };
    let fetch_calls = Arc::new(AtomicUsize::new(0));
    let fetch_impl = {
        let fetch_calls = fetch_calls.clone();
        move |_url: String| -> BoxFuture<'static, Result<HttpResponse, TestError>> {
            fetch_calls.fetch_add(1, Ordering::SeqCst);
            async { Err(TestError("fetchImpl must not be called".to_owned())) }.boxed()
        }
    };

    let error = fetch_following_safe_redirects(FollowRedirectsInput {
        errors: &errors,
        fetch_impl: &fetch_impl,
        max_redirect_hops: None,
        url: "https://example.com/a".to_owned(),
        validate_url: &validate_url,
    })
    .await
    .err()
    .expect("expected rejection");

    assert_message_contains(&error, "blocked");
    assert_eq!(fetch_calls.load(Ordering::SeqCst), 0);
}
