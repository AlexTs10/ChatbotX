//! Port of `packages/filesystem/__tests__/upload-from-url.test.ts` (P2.9).
//!
//! Mock mapping (spec section 5):
//! - `vi.mock(uploader)` with `putObject: vi.fn(async () => undefined)` ->
//!   `RecordingPutter` (implements `ObjectPutter`, records every call, returns
//!   `Ok(())`). A new one per test replaces `mockPutObject.mockClear()`.
//! - `globalThis.fetch = vi.fn(...)` -> `FakeFetcher` (implements
//!   `HttpFetcher`, records `(url, FetchOptions)`, returns canned responses).
//! - The logger mock is not needed (`tracing` events are not asserted).
//! - `AbortController.signal` -> `CancellationToken`.
//! - `toHaveBeenCalledWith(...)` -> "at least one recorded call matches".
//! - `rejects.toThrow(UploadValidationError)` -> `FilesError::UploadValidation`;
//!   `rejects.toThrow(msg)` -> the error message contains `msg`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use chatbotx_files::{
    Acl, Body, DEFAULT_ACL, FetchError, FetchOptions, FileType, FilesError, HttpFetcher,
    HttpResponse, ObjectPutter, PutOptions, UploadDeps, UploadFromUrlOptions, ValidateUrl,
    upload_file_from_url,
};
use futures::FutureExt;
use futures::StreamExt;
use futures::future::BoxFuture;
use http::{HeaderMap, HeaderName, HeaderValue};
use tokio_util::sync::CancellationToken;

// --- fakes -----------------------------------------------------------------

type Respond = Box<dyn Fn(usize) -> HttpResponse + Send + Sync>;

/// `globalThis.fetch = vi.fn(...)`. `respond` gets the 1-based call number.
struct FakeFetcher {
    calls: Mutex<Vec<(String, FetchOptions)>>,
    respond: Respond,
}

impl FakeFetcher {
    fn new(respond: impl Fn(usize) -> HttpResponse + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            respond: Box::new(respond),
        })
    }

    fn calls(&self) -> Vec<(String, FetchOptions)> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl HttpFetcher for FakeFetcher {
    async fn fetch(&self, url: &str, opts: FetchOptions) -> Result<HttpResponse, FetchError> {
        let call = {
            let mut calls = self.calls.lock().expect("lock");
            calls.push((url.to_owned(), opts));
            calls.len()
        };
        Ok((self.respond)(call))
    }
}

#[derive(Clone)]
struct PutCall {
    path: String,
    #[allow(dead_code)]
    body: Bytes,
    options: PutOptions,
    cancel: Option<CancellationToken>,
}

/// `uploader.putObject = vi.fn(async () => undefined)`.
#[derive(Default)]
struct RecordingPutter {
    calls: Mutex<Vec<PutCall>>,
}

impl RecordingPutter {
    fn calls(&self) -> Vec<PutCall> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ObjectPutter for RecordingPutter {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutOptions,
        cancel: Option<CancellationToken>,
    ) -> Result<(), FilesError> {
        self.calls.lock().expect("lock").push(PutCall {
            path: path.to_owned(),
            body,
            options,
            cancel,
        });
        Ok(())
    }
}

fn deps(fetcher: &Arc<FakeFetcher>, putter: &Arc<RecordingPutter>) -> UploadDeps {
    UploadDeps {
        fetcher: fetcher.clone(),
        putter: putter.clone(),
    }
}

/// `vi.fn(async () => undefined)` used as `validateUrl`, with a call log.
fn validate_spy() -> (ValidateUrl, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let log = calls.clone();
    let validate: ValidateUrl = Arc::new(
        move |url: String| -> BoxFuture<'static, Result<(), FilesError>> {
            log.lock().expect("lock").push(url);
            async { Ok(()) }.boxed()
        },
    );
    (validate, calls)
}

// --- responses -------------------------------------------------------------

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).expect("valid header"),
        );
    }
    map
}

/// JS `streamResponse(chunks, { status, headers })` (status defaults to 200).
fn stream_response(chunks: Vec<Bytes>, headers: HeaderMap) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers,
        body: Body::Stream(futures::stream::iter(chunks.into_iter().map(Ok)).boxed()),
    }
}

/// JS `new Response(null, { status, headers })`.
fn empty_response(status: u16, headers: HeaderMap) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body: Body::Buffered(Bytes::new()),
    }
}

fn assert_message_contains(error: &FilesError, expected: &str) {
    assert!(
        error.to_string().contains(expected),
        "expected error containing {expected:?}, got {:?}",
        error.to_string()
    );
}

// --- tests -----------------------------------------------------------------

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:51 "uploadFileFromUrl byte cap and redirect handling > throws when the body exceeds maxBytes even though content-length lies small"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_throws_when_the_body_exceeds_max_bytes_even_though_content_length_lies_small()
 {
    let big_chunk = Bytes::from(vec![0u8; 20]);
    let fetcher = FakeFetcher::new(move |_| {
        stream_response(
            vec![big_chunk.clone()],
            headers(&[("content-length", "1"), ("content-type", "text/plain")]),
        )
    });
    let putter = Arc::new(RecordingPutter::default());

    let error = upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/file.txt",
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(10),
            validate_url: None,
            cancel: None,
        },
    )
    .await
    .expect_err("expected rejection");

    assert!(
        matches!(error, FilesError::UploadValidation(_)),
        "expected FilesError::UploadValidation, got {error:?}"
    );
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:69 "uploadFileFromUrl byte cap and redirect handling > records the real streamed byte count as size on the success path"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_records_the_real_streamed_byte_count_as_size_on_the_success_path()
 {
    let chunk = Bytes::from_static(b"hello world");
    let fetcher = {
        let chunk = chunk.clone();
        FakeFetcher::new(move |_| {
            stream_response(
                vec![chunk.clone()],
                headers(&[("content-length", "1"), ("content-type", "text/plain")]),
            )
        })
    };
    let putter = Arc::new(RecordingPutter::default());

    let result = upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/file.txt",
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(1000),
            validate_url: None,
            cancel: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.size, chunk.len() as u64);
    // toHaveBeenCalledWith("path/to/file", anything, objectContaining({ ContentLength }), undefined)
    assert!(
        putter.calls().iter().any(|call| call.path == "path/to/file"
            && call.options.content_length == Some(chunk.len() as u64)
            && call.cancel.is_none()),
        "putObject was not called with the expected arguments"
    );
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:96 "uploadFileFromUrl byte cap and redirect handling > stores and returns image/jpeg when the host serves the non-standard image/jpg"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_stores_and_returns_image_jpeg_when_the_host_serves_the_non_standard_image_jpg()
 {
    // 1x1 baseline JPEG, so the dimension probe succeeds and keeps the type.
    let jpeg = Bytes::from(
        base64::engine::general_purpose::STANDARD
            .decode("/9j/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAj/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AKpAB//Z")
            .expect("valid base64"),
    );
    let fetcher = FakeFetcher::new(move |_| {
        stream_response(
            vec![jpeg.clone()],
            headers(&[("content-type", "image/jpg; charset=binary")]),
        )
    });
    let putter = Arc::new(RecordingPutter::default());

    // JS: uploadFileFromUrl(url, path) with every optional argument left out.
    let result = upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/photo.jpg",
        "path/to/photo",
        UploadFromUrlOptions {
            acl: DEFAULT_ACL,
            max_bytes: None,
            validate_url: None,
            cancel: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.mime_type, "image/jpeg");
    assert_eq!(result.file_type, FileType::Image);
    // toHaveBeenCalledWith("path/to/photo", anything, objectContaining({ ContentType: "image/jpeg" }), undefined)
    assert!(
        putter
            .calls()
            .iter()
            .any(|call| call.path == "path/to/photo"
                && call.options.content_type.as_deref() == Some("image/jpeg")
                && call.cancel.is_none()),
        "putObject was not called with the expected arguments"
    );
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:125 "uploadFileFromUrl byte cap and redirect handling > forwards the abort signal to both the download and the upload"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_forwards_the_abort_signal_to_both_the_download_and_the_upload()
 {
    let fetcher = FakeFetcher::new(|_| {
        stream_response(
            vec![Bytes::from_static(b"hi")],
            headers(&[("content-length", "1"), ("content-type", "text/plain")]),
        )
    });
    let putter = Arc::new(RecordingPutter::default());
    let controller = CancellationToken::new();

    upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/file.txt",
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(1000),
            validate_url: None,
            cancel: Some(controller.clone()),
        },
    )
    .await
    .unwrap();

    // The deadline must bound both halves: the download fetch and the S3 put.
    // "Same signal" = a recorded token that is cancelled by cancelling
    // `controller` (and not cancelled before).
    let fetch_tokens: Vec<CancellationToken> = fetcher
        .calls()
        .into_iter()
        .filter(|(url, _)| url == "https://example.com/file.txt")
        .filter_map(|(_, opts)| opts.cancel)
        .collect();
    let put_tokens: Vec<CancellationToken> = putter
        .calls()
        .into_iter()
        .filter(|call| call.path == "path/to/file")
        .filter_map(|call| call.cancel)
        .collect();
    assert!(
        !fetch_tokens.is_empty(),
        "fetch was not called with the url and a signal"
    );
    assert!(
        !put_tokens.is_empty(),
        "putObject was not called with the path and a signal"
    );
    assert!(fetch_tokens.iter().all(|t| !t.is_cancelled()));
    assert!(put_tokens.iter().all(|t| !t.is_cancelled()));
    controller.cancel();
    assert!(
        fetch_tokens.iter().any(CancellationToken::is_cancelled),
        "fetch did not get the caller's signal"
    );
    assert!(
        put_tokens.iter().any(CancellationToken::is_cancelled),
        "putObject did not get the caller's signal"
    );
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:157 "uploadFileFromUrl byte cap and redirect handling > re-invokes validateUrl on each redirect hop, including the final one"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_re_invokes_validate_url_on_each_redirect_hop_including_the_final_one()
 {
    let (validate_url, validate_calls) = validate_spy();
    let hop1 = "https://example.com/hop1";
    let hop2 = "https://example.com/hop2";
    let final_url = "https://example.com/final";

    let fetcher = FakeFetcher::new(move |call| match call {
        1 => empty_response(302, headers(&[("location", hop2)])),
        2 => empty_response(302, headers(&[("location", final_url)])),
        _ => stream_response(
            vec![Bytes::from_static(b"ok")],
            headers(&[("content-type", "text/plain")]),
        ),
    });
    let putter = Arc::new(RecordingPutter::default());

    upload_file_from_url(
        &deps(&fetcher, &putter),
        hop1,
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(1000),
            validate_url: Some(validate_url),
            cancel: None,
        },
    )
    .await
    .unwrap();

    let validate_calls = validate_calls.lock().expect("lock");
    assert_eq!(validate_calls.len(), 3);
    assert_eq!(validate_calls[0], hop1);
    assert_eq!(validate_calls[1], hop2);
    assert_eq!(validate_calls[2], final_url);
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:191 "uploadFileFromUrl byte cap and redirect handling > throws 'Too many redirects' past hop 6"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_throws_too_many_redirects_past_hop_6()
{
    let (validate_url, _validate_calls) = validate_spy();
    let fetcher = FakeFetcher::new(|call| {
        let location = format!("https://example.com/hop{call}");
        empty_response(302, headers(&[("location", location.as_str())]))
    });
    let putter = Arc::new(RecordingPutter::default());

    let error = upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/hop0",
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(1000),
            validate_url: Some(validate_url),
            cancel: None,
        },
    )
    .await
    .expect_err("expected rejection");

    assert_message_contains(&error, "Too many redirects");
}

/// Port of: packages/filesystem/__tests__/upload-from-url.test.ts:215 "uploadFileFromUrl byte cap and redirect handling > throws when a redirect response has no Location header"
#[tokio::test]
async fn upload_file_from_url_byte_cap_and_redirect_handling_throws_when_a_redirect_response_has_no_location_header()
 {
    let (validate_url, _validate_calls) = validate_spy();
    let fetcher = FakeFetcher::new(|_| empty_response(302, HeaderMap::new()));
    let putter = Arc::new(RecordingPutter::default());

    let error = upload_file_from_url(
        &deps(&fetcher, &putter),
        "https://example.com/file",
        "path/to/file",
        UploadFromUrlOptions {
            acl: Acl::Private,
            max_bytes: Some(1000),
            validate_url: Some(validate_url),
            cancel: None,
        },
    )
    .await
    .expect_err("expected rejection");

    assert_message_contains(&error, "Redirect response has no Location header");
}
