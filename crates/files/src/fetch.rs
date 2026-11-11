//! HTTP abstraction and bounded fetch (spec 1.6 and 5; JS
//! `bounded-fetch.ts`). The HTTP client is injected ([`HttpFetcher`]) so
//! tests need no global mock.

use std::time::Duration;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use http::{HeaderMap, header};
use tokio_util::sync::CancellationToken;

use crate::FilesError;

const DEFAULT_MAX_REDIRECT_HOPS: u32 = 5;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct FetchError(pub String);

impl From<FetchError> for FilesError {
    fn from(error: FetchError) -> Self {
        FilesError::Fetch(error.0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// `true` = JS `redirect: "follow"`; `false` = `redirect: "manual"`.
    pub follow_redirects: bool,
    /// JS `signal`.
    pub cancel: Option<CancellationToken>,
}

/// Response body. `Buffered` is the JS "no body stream" path
/// (`response.body == null`, read with `arrayBuffer()`).
pub enum Body {
    Stream(BoxStream<'static, Result<Bytes, FetchError>>),
    Buffered(Bytes),
}

impl Body {
    /// Reads the whole body with no limit (JS `arrayBuffer()`).
    pub async fn bytes(self) -> Result<Bytes, FetchError> {
        match self {
            Body::Buffered(bytes) => Ok(bytes),
            Body::Stream(mut stream) => {
                let mut out = BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    out.extend_from_slice(&chunk?);
                }
                Ok(out.freeze())
            }
        }
    }
}

pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Body,
}

impl HttpResponse {
    /// JS `response.ok` (status 200-299).
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// JS `headers.get(name)`: the value as text, or `None` when absent.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
    }
}

#[async_trait]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, url: &str, opts: FetchOptions) -> Result<HttpResponse, FetchError>;
}

// ---------------------------------------------------------------------------
// Bounded body read
// ---------------------------------------------------------------------------

/// Reads a body with a running byte count and stops as soon as the total is
/// greater than `max_bytes` (exactly `max_bytes` is allowed). The error is
/// the value `on_limit_exceeded(max_bytes)` returns. A buffered body (no
/// stream) is checked as a whole. Dropping the stream cancels the read.
pub async fn read_body_with_limit<E>(
    body: Body,
    max_bytes: u64,
    on_limit_exceeded: impl FnOnce(u64) -> E,
) -> Result<Bytes, E>
where
    E: From<FetchError>,
{
    match body {
        Body::Buffered(bytes) => {
            if bytes.len() as u64 > max_bytes {
                return Err(on_limit_exceeded(max_bytes));
            }
            Ok(bytes)
        }
        Body::Stream(mut stream) => {
            let mut out = BytesMut::new();
            let mut total: u64 = 0;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if chunk.is_empty() {
                    continue;
                }
                total = total.saturating_add(chunk.len() as u64);
                if total > max_bytes {
                    drop(stream);
                    return Err(on_limit_exceeded(max_bytes));
                }
                out.extend_from_slice(&chunk);
            }
            Ok(out.freeze())
        }
    }
}

// ---------------------------------------------------------------------------
// Safe redirects
// ---------------------------------------------------------------------------

/// `(location, cause) -> error`.
pub type InvalidRedirectLocationFn<E> = dyn Fn(&str, &str) -> E + Send + Sync;

pub struct SafeRedirectErrors<E> {
    pub invalid_redirect_location: Box<InvalidRedirectLocationFn<E>>,
    pub no_location_header: Box<dyn Fn() -> E + Send + Sync>,
    pub too_many_redirects: Box<dyn Fn() -> E + Send + Sync>,
}

pub type FetchImpl<E> = dyn Fn(String) -> BoxFuture<'static, Result<HttpResponse, E>> + Send + Sync;

pub type ValidateUrlFn<E> = dyn Fn(String) -> BoxFuture<'static, Result<(), E>> + Send + Sync;

pub struct FollowRedirectsInput<'a, E> {
    pub errors: &'a SafeRedirectErrors<E>,
    pub fetch_impl: &'a FetchImpl<E>,
    /// `None` = default 5.
    pub max_redirect_hops: Option<u32>,
    pub url: String,
    pub validate_url: &'a ValidateUrlFn<E>,
}

/// Fetches `url`, following redirects by hand so `validate_url` runs before
/// every hop is requested. Any 3xx status is a redirect. With the default
/// 5 hops, 5 redirects are followed and the 6th redirect response fails with
/// `too_many_redirects` (checked before the `Location` header). Returns
/// `(final_url, response)`; the first URL is returned as given, a redirect
/// target in its normalized (WHATWG `href`) form.
pub async fn fetch_following_safe_redirects<E>(
    input: FollowRedirectsInput<'_, E>,
) -> Result<(String, HttpResponse), E> {
    let FollowRedirectsInput {
        errors,
        fetch_impl,
        max_redirect_hops,
        mut url,
        validate_url,
    } = input;
    let mut redirects_left = max_redirect_hops.unwrap_or(DEFAULT_MAX_REDIRECT_HOPS);

    loop {
        validate_url(url.clone()).await?;
        let response = fetch_impl(url.clone()).await?;
        if !(300..400).contains(&response.status) {
            return Ok((url, response));
        }
        if redirects_left == 0 {
            return Err((errors.too_many_redirects)());
        }
        let location = response
            .header(header::LOCATION.as_str())
            .filter(|location| !location.is_empty())
            .ok_or_else(|| (errors.no_location_header)())?;
        let next = url::Url::parse(&url)
            .and_then(|base| base.join(&location))
            .map_err(|cause| (errors.invalid_redirect_location)(&location, &cause.to_string()))?;
        url = next.to_string();
        redirects_left -= 1;
    }
}

// ---------------------------------------------------------------------------
// Production fetcher
// ---------------------------------------------------------------------------

/// [`HttpFetcher`] over `reqwest`. `follow_redirects` uses the platform
/// default (up to 10 hops); manual mode returns 3xx responses as they are.
/// No timeout is set here: callers bound the call with their token or a
/// `tokio::time::timeout`, like the JS `AbortSignal.timeout(...)`.
#[derive(Debug, Clone)]
pub struct ReqwestFetcher {
    follow: reqwest::Client,
    manual: reqwest::Client,
}

impl ReqwestFetcher {
    pub fn new() -> Result<Self, FetchError> {
        let build = |policy| {
            reqwest::Client::builder()
                .redirect(policy)
                .connect_timeout(Duration::from_secs(30))
                .build()
                .map_err(|error| FetchError(error.to_string()))
        };
        Ok(Self {
            follow: build(reqwest::redirect::Policy::default())?,
            manual: build(reqwest::redirect::Policy::none())?,
        })
    }
}

#[async_trait]
impl HttpFetcher for ReqwestFetcher {
    async fn fetch(&self, url: &str, opts: FetchOptions) -> Result<HttpResponse, FetchError> {
        let client = if opts.follow_redirects {
            &self.follow
        } else {
            &self.manual
        };
        let send = client.get(url).send();
        let response = match &opts.cancel {
            Some(token) => token
                .run_until_cancelled(send)
                .await
                .ok_or_else(|| FetchError("The operation was aborted".to_owned()))?,
            None => send.await,
        }
        .map_err(|error| FetchError(error.to_string()))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let stream = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|error| FetchError(error.to_string())));
        let body = match opts.cancel {
            Some(token) => Body::Stream(cancellable(stream.boxed(), token)),
            None => Body::Stream(stream.boxed()),
        };
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Ends `stream` with an error once `token` is cancelled (JS: an aborted
/// signal errors the body stream).
fn cancellable(
    stream: BoxStream<'static, Result<Bytes, FetchError>>,
    token: CancellationToken,
) -> BoxStream<'static, Result<Bytes, FetchError>> {
    futures::stream::unfold(Some((stream, token)), |state| async move {
        let (mut stream, token) = state?;
        match token.run_until_cancelled(stream.next()).await {
            None => Some((
                Err(FetchError("The operation was aborted".to_owned())),
                None,
            )),
            Some(None) => None,
            Some(Some(item)) => Some((item, Some((stream, token)))),
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// One-shot HTTP server on 127.0.0.1 that answers every connection with
    /// `responses[n]` in order.
    fn serve(responses: Vec<&'static str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf);
                let _ = socket.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn manual_mode_returns_the_redirect_and_streams_the_body() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let base = serve(vec![
                "HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            ]);
            let fetcher = ReqwestFetcher::new().expect("client");
            let first = fetcher
                .fetch(&format!("{base}/a"), FetchOptions::default())
                .await
                .expect("first");
            assert_eq!(first.status, 302);
            assert_eq!(first.header("location").as_deref(), Some("/next"));

            let second = fetcher
                .fetch(
                    &format!("{base}/next"),
                    FetchOptions {
                        follow_redirects: false,
                        cancel: Some(CancellationToken::new()),
                    },
                )
                .await
                .expect("second");
            assert!(second.ok());
            let body = read_body_with_limit(second.body, 5, |_| FetchError("limit".into()))
                .await
                .expect("body");
            assert_eq!(&body[..], b"hello");
        });
    }

    #[test]
    fn a_cancelled_token_aborts_the_fetch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let token = CancellationToken::new();
            token.cancel();
            let fetcher = ReqwestFetcher::new().expect("client");
            let error = fetcher
                .fetch(
                    "http://127.0.0.1:9/never",
                    FetchOptions {
                        follow_redirects: true,
                        cancel: Some(token),
                    },
                )
                .await
                .err()
                .expect("aborted");
            assert!(error.0.contains("aborted"));
        });
    }

    #[test]
    fn a_cancelled_stream_ends_with_an_error() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let token = CancellationToken::new();
            let inner = futures::stream::pending::<Result<Bytes, FetchError>>().boxed();
            let mut stream = cancellable(inner, token.clone());
            token.cancel();
            assert!(stream.next().await.expect("item").is_err());
            assert!(stream.next().await.is_none());
        });
    }

    #[test]
    fn redirect_targets_are_resolved_against_the_current_url() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let errors = SafeRedirectErrors::<FetchError> {
                invalid_redirect_location: Box::new(|location, _| {
                    FetchError(format!("invalid {location}"))
                }),
                no_location_header: Box::new(|| FetchError("none".into())),
                too_many_redirects: Box::new(|| FetchError("too many".into())),
            };
            let fetch_impl = |url: String| -> BoxFuture<'static, Result<HttpResponse, FetchError>> {
                Box::pin(async move {
                    let mut headers = HeaderMap::new();
                    let status = if url.ends_with("/a/b") {
                        headers.insert(header::LOCATION, "../c?x=1".parse().expect("value"));
                        301
                    } else {
                        200
                    };
                    Ok(HttpResponse {
                        status,
                        headers,
                        body: Body::Buffered(Bytes::new()),
                    })
                })
            };
            let validate = |_url: String| -> BoxFuture<'static, Result<(), FetchError>> {
                Box::pin(async { Ok(()) })
            };
            let (final_url, response) = fetch_following_safe_redirects(FollowRedirectsInput {
                errors: &errors,
                fetch_impl: &fetch_impl,
                max_redirect_hops: Some(1),
                url: "https://example.com/a/b".to_owned(),
                validate_url: &validate,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(final_url, "https://example.com/c?x=1");
            assert_eq!(response.status, 200);

            // Zero hops: the first redirect already fails.
            let error = fetch_following_safe_redirects(FollowRedirectsInput {
                errors: &errors,
                fetch_impl: &fetch_impl,
                max_redirect_hops: Some(0),
                url: "https://example.com/a/b".to_owned(),
                validate_url: &validate,
            })
            .await
            .err()
            .expect("error");
            assert_eq!(error.0, "too many");
        });
    }

    #[test]
    fn exactly_max_bytes_is_allowed_and_empty_chunks_are_skipped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let chunks = vec![
                Ok(Bytes::from_static(b"12345")),
                Ok(Bytes::new()),
                Ok(Bytes::from_static(b"67890")),
            ];
            let body = Body::Stream(futures::stream::iter(chunks).boxed());
            let bytes = read_body_with_limit(body, 10, |_| FetchError("limit".into()))
                .await
                .expect("body");
            assert_eq!(bytes.len(), 10);

            let chunks = vec![
                Ok(Bytes::from_static(b"1")),
                Err(FetchError("reset".into())),
            ];
            let body = Body::Stream(futures::stream::iter(chunks).boxed());
            let error = read_body_with_limit(body, 10, |_| FetchError("limit".into()))
                .await
                .expect_err("error");
            assert_eq!(error.0, "reset");
        });
    }
}
