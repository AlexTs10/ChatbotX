//! Port of `packages/utils/__tests__/media-download.test.ts`. The stubbed
//! `fetch` / `Response` become a fake `MediaFetcher` / `MediaBody` that
//! count reads and cancels; the abort signal is the timeout passed to fetch.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chatbotx_core::utils::media_download::{
    DEFAULT_MEDIA_TIMEOUT_MS, FetchMediaOptions, MediaBody, MediaDownloadError, MediaFetcher,
    MediaResponse, fetch_media_with_limits, read_body_with_cap,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use regex::Regex;

#[derive(Default)]
struct Counters {
    reads: Cell<usize>,
    cancels: Cell<usize>,
}

struct FakeBody {
    chunks: Vec<Vec<u8>>,
    index: usize,
    counters: Rc<Counters>,
    cancel_fails: bool,
}

impl MediaBody for FakeBody {
    async fn read(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.counters.reads.set(self.counters.reads.get() + 1);
        let chunk = self.chunks.get(self.index).cloned();
        self.index += 1;
        Ok(chunk)
    }

    async fn cancel(&mut self) -> Result<(), String> {
        self.counters.cancels.set(self.counters.cancels.get() + 1);
        if self.cancel_fails {
            Err("socket already destroyed".to_owned())
        } else {
            Ok(())
        }
    }
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn build_response(
    chunks: Option<Vec<Vec<u8>>>,
    ok: bool,
    header_pairs: &[(&str, &str)],
) -> (MediaResponse<FakeBody>, Rc<Counters>) {
    let counters = Rc::new(Counters::default());
    let body = chunks.map(|chunks| FakeBody {
        chunks,
        index: 0,
        counters: counters.clone(),
        cancel_fails: false,
    });
    (
        MediaResponse {
            ok,
            headers: headers(header_pairs),
            body,
        },
        counters,
    )
}

struct FakeFetcher {
    response: RefCell<Option<MediaResponse<FakeBody>>>,
    calls: RefCell<Vec<(String, Vec<(String, String)>, u64)>>,
}

impl FakeFetcher {
    fn new(response: MediaResponse<FakeBody>) -> FakeFetcher {
        FakeFetcher {
            response: RefCell::new(Some(response)),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl MediaFetcher for FakeFetcher {
    type Body = FakeBody;

    async fn fetch(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> Result<MediaResponse<FakeBody>, String> {
        self.calls
            .borrow_mut()
            .push((url.to_owned(), headers.to_vec(), timeout_ms));
        Ok(self.response.borrow_mut().take().expect("one fetch"))
    }
}

fn declared_size_error(err: &MediaDownloadError) -> bool {
    matches!(err, MediaDownloadError::TooLarge(_))
        && Regex::new("^Media exceeds size limit")
            .unwrap()
            .is_match(&err.to_string())
}

fn body_size_error(err: &MediaDownloadError) -> bool {
    Regex::new("body exceeds size limit")
        .unwrap()
        .is_match(&err.to_string())
}

/// Port of: packages/utils/__tests__/media-download.test.ts:62 "readBodyWithCap > streams chunks into a single ArrayBuffer"
#[tokio::test]
async fn readbodywithcap_streams_chunks_into_a_single_arraybuffer() {
    let (mut response, _) = build_response(Some(vec![vec![1, 2], vec![3, 4]]), true, &[]);
    let bytes = read_body_with_cap(&mut response, 100, "x").await.unwrap();
    assert_eq!(bytes, vec![1, 2, 3, 4]);
}

/// Port of: packages/utils/__tests__/media-download.test.ts:73 "readBodyWithCap > rejects up front and releases the body when declared content-length exceeds the cap"
#[tokio::test]
async fn readbodywithcap_rejects_up_front_and_releases_the_body_when_declared_content_length_exceeds_the_cap()
 {
    let (mut response, counters) =
        build_response(Some(vec![vec![1]]), true, &[("content-length", "50")]);
    let err = read_body_with_cap(&mut response, 8, "Media")
        .await
        .unwrap_err();
    assert!(matches!(err, MediaDownloadError::TooLarge(_)));
    assert_eq!(counters.reads.get(), 0);
    assert_eq!(counters.cancels.get(), 1);
}

/// Port of: packages/utils/__tests__/media-download.test.ts:90 "readBodyWithCap > throws MediaTooLargeError even when the reader cancel rejects"
#[tokio::test]
async fn readbodywithcap_throws_mediatoolargeerror_even_when_the_reader_cancel_rejects() {
    let counters = Rc::new(Counters::default());
    let mut response = MediaResponse {
        ok: true,
        headers: HeaderMap::new(),
        body: Some(FakeBody {
            chunks: vec![vec![0; 20]],
            index: 0,
            counters,
            cancel_fails: true,
        }),
    };
    let err = read_body_with_cap(&mut response, 8, "Media")
        .await
        .unwrap_err();
    assert!(matches!(err, MediaDownloadError::TooLarge(_)), "{err:?}");
}

/// Port of: packages/utils/__tests__/media-download.test.ts:116 "readBodyWithCap > aborts a streamed body without content-length when it crosses the cap"
#[tokio::test]
async fn readbodywithcap_aborts_a_streamed_body_without_content_length_when_it_crosses_the_cap() {
    let (mut response, counters) = build_response(Some(vec![vec![0; 6], vec![0; 6]]), true, &[]);
    let err = read_body_with_cap(&mut response, 8, "Media")
        .await
        .unwrap_err();
    assert!(body_size_error(&err), "{err}");
    assert_eq!(counters.cancels.get(), 1);
}

/// Port of: packages/utils/__tests__/media-download.test.ts:128 "readBodyWithCap > throws MediaEmptyBodyError when there is no body"
#[tokio::test]
async fn readbodywithcap_throws_mediaemptybodyerror_when_there_is_no_body() {
    let (mut response, _) = build_response(None, true, &[]);
    let err = read_body_with_cap(&mut response, 8, "Media")
        .await
        .unwrap_err();
    assert!(matches!(err, MediaDownloadError::EmptyBody(_)));
}

/// Port of: packages/utils/__tests__/media-download.test.ts:136 "readBodyWithCap > ignores a NaN content-length and streams the body"
#[tokio::test]
async fn readbodywithcap_ignores_a_nan_content_length_and_streams_the_body() {
    let (mut response, _) = build_response(
        Some(vec![vec![9]]),
        true,
        &[("content-length", "not-a-number")],
    );
    assert_eq!(
        read_body_with_cap(&mut response, 8, "x").await.unwrap(),
        vec![9]
    );
}

/// Port of: packages/utils/__tests__/media-download.test.ts:148 "fetchMediaWithLimits > returns bytes and content-type for a successful response"
#[tokio::test]
async fn fetchmediawithlimits_returns_bytes_and_content_type_for_a_successful_response() {
    let (response, _) = build_response(
        Some(vec![vec![1, 2, 3, 4]]),
        true,
        &[("content-type", "image/jpeg"), ("content-length", "4")],
    );
    let fetcher = FakeFetcher::new(response);
    let result = fetch_media_with_limits(
        &fetcher,
        "https://cdn.example/pic.jpg",
        &FetchMediaOptions::default(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.mime_type, "image/jpeg");
    assert_eq!(result.bytes, vec![1, 2, 3, 4]);
}

/// Port of: packages/utils/__tests__/media-download.test.ts:163 "fetchMediaWithLimits > falls back to the default mime type when content-type is absent"
#[tokio::test]
async fn fetchmediawithlimits_falls_back_to_the_default_mime_type_when_content_type_is_absent() {
    let (response, _) = build_response(Some(vec![vec![1, 2]]), true, &[]);
    let fetcher = FakeFetcher::new(response);
    let result = fetch_media_with_limits(
        &fetcher,
        "https://cdn.example/pic",
        &FetchMediaOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap().mime_type, "image/png");
}

/// Port of: packages/utils/__tests__/media-download.test.ts:171 "fetchMediaWithLimits > honours a custom fallback mime type"
#[tokio::test]
async fn fetchmediawithlimits_honours_a_custom_fallback_mime_type() {
    let (response, _) = build_response(Some(vec![vec![1, 2]]), true, &[]);
    let fetcher = FakeFetcher::new(response);
    let options = FetchMediaOptions {
        fallback_mime_type: Some("image/webp".to_owned()),
        ..FetchMediaOptions::default()
    };
    let result = fetch_media_with_limits(&fetcher, "https://cdn.example/pic", &options)
        .await
        .unwrap();
    assert_eq!(result.unwrap().mime_type, "image/webp");
}

/// Port of: packages/utils/__tests__/media-download.test.ts:181 "fetchMediaWithLimits > returns null and cancels the body on a non-2xx response"
#[tokio::test]
async fn fetchmediawithlimits_returns_null_and_cancels_the_body_on_a_non_2xx_response() {
    let (response, counters) = build_response(Some(vec![vec![1]]), false, &[]);
    let fetcher = FakeFetcher::new(response);
    assert_eq!(
        fetch_media_with_limits(
            &fetcher,
            "https://cdn.example/pic",
            &FetchMediaOptions::default()
        )
        .await
        .unwrap(),
        None
    );
    assert_eq!(counters.cancels.get(), 1);
}

/// Port of: packages/utils/__tests__/media-download.test.ts:191 "fetchMediaWithLimits > returns null when the response has no body"
#[tokio::test]
async fn fetchmediawithlimits_returns_null_when_the_response_has_no_body() {
    let (response, _) = build_response(None, true, &[]);
    let fetcher = FakeFetcher::new(response);
    assert_eq!(
        fetch_media_with_limits(
            &fetcher,
            "https://cdn.example/pic",
            &FetchMediaOptions::default()
        )
        .await
        .unwrap(),
        None
    );
}

/// Port of: packages/utils/__tests__/media-download.test.ts:197 "fetchMediaWithLimits > throws when the declared content-length exceeds the cap"
#[tokio::test]
async fn fetchmediawithlimits_throws_when_the_declared_content_length_exceeds_the_cap() {
    let (response, _) = build_response(Some(vec![vec![1, 2]]), true, &[("content-length", "20")]);
    let fetcher = FakeFetcher::new(response);
    let options = FetchMediaOptions {
        max_bytes: Some(8),
        ..FetchMediaOptions::default()
    };
    let err = fetch_media_with_limits(&fetcher, "https://cdn.example/pic", &options)
        .await
        .unwrap_err();
    assert!(declared_size_error(&err), "{err}");
}

/// Port of: packages/utils/__tests__/media-download.test.ts:209 "fetchMediaWithLimits > throws when the streamed body exceeds the cap without a declared length"
#[tokio::test]
async fn fetchmediawithlimits_throws_when_the_streamed_body_exceeds_the_cap_without_a_declared_length()
 {
    let (response, _) = build_response(Some(vec![vec![0; 6], vec![0; 6]]), true, &[]);
    let fetcher = FakeFetcher::new(response);
    let options = FetchMediaOptions {
        max_bytes: Some(8),
        ..FetchMediaOptions::default()
    };
    let err = fetch_media_with_limits(&fetcher, "https://cdn.example/pic", &options)
        .await
        .unwrap_err();
    assert!(body_size_error(&err), "{err}");
}

/// Port of: packages/utils/__tests__/media-download.test.ts:217 "fetchMediaWithLimits > forwards headers and an abort signal to fetch"
#[tokio::test]
async fn fetchmediawithlimits_forwards_headers_and_an_abort_signal_to_fetch() {
    let (response, _) = build_response(Some(vec![vec![1]]), true, &[]);
    let fetcher = FakeFetcher::new(response);
    let options = FetchMediaOptions {
        headers: vec![("Authorization".to_owned(), "Bearer token".to_owned())],
        ..FetchMediaOptions::default()
    };
    fetch_media_with_limits(&fetcher, "https://cdn.example/pic", &options)
        .await
        .unwrap();
    assert_eq!(
        *fetcher.calls.borrow(),
        vec![(
            "https://cdn.example/pic".to_owned(),
            vec![("Authorization".to_owned(), "Bearer token".to_owned())],
            DEFAULT_MEDIA_TIMEOUT_MS
        )]
    );
}
