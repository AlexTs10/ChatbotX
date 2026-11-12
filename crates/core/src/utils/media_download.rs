//! `packages/utils/src/media-download.ts`: bounded media downloads. HTTP is
//! behind the [`MediaFetcher`] / [`MediaBody`] traits (the server wires a
//! real client; tests use fakes).

use std::fmt;
use std::future::Future;

use http::HeaderMap;

/// `DEFAULT_TIMEOUT_MS`.
pub const DEFAULT_MEDIA_TIMEOUT_MS: u64 = 15_000;
/// `DEFAULT_MAX_BYTES`.
pub const DEFAULT_MEDIA_MAX_BYTES: usize = 10 * 1024 * 1024;

/// A media download failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaDownloadError {
    /// `MediaTooLargeError`.
    TooLarge(String),
    /// `MediaEmptyBodyError`.
    EmptyBody(String),
    /// The fetch or a body read failed.
    Transport(String),
}

impl fmt::Display for MediaDownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaDownloadError::TooLarge(m)
            | MediaDownloadError::EmptyBody(m)
            | MediaDownloadError::Transport(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for MediaDownloadError {}

/// A streamed response body.
pub trait MediaBody {
    /// The next chunk; `None` at the end.
    fn read(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, String>>;
    /// Releases the connection.
    fn cancel(&mut self) -> impl Future<Output = Result<(), String>>;
}

/// A response: status, headers and an optional body.
#[derive(Debug)]
pub struct MediaResponse<B> {
    pub ok: bool,
    pub headers: HeaderMap,
    pub body: Option<B>,
}

/// Issues the request (headers and a timeout).
pub trait MediaFetcher {
    type Body: MediaBody;
    fn fetch(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout_ms: u64,
    ) -> impl Future<Output = Result<MediaResponse<Self::Body>, String>>;
}

/// `cancelBodyQuietly`.
pub async fn cancel_body_quietly<B: MediaBody>(body: Option<&mut B>) {
    if let Some(body) = body {
        let _ = body.cancel().await;
    }
}

/// `readBodyWithCap`: a declared `content-length` over the cap is refused
/// without reading; otherwise the stream is cancelled once it crosses it.
pub async fn read_body_with_cap<B: MediaBody>(
    response: &mut MediaResponse<B>,
    max_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, MediaDownloadError> {
    if let Some(declared) = response
        .headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::js::parse_int)
    {
        if declared > max_bytes as f64 {
            cancel_body_quietly(response.body.as_mut()).await;
            return Err(MediaDownloadError::TooLarge(format!(
                "{label} exceeds size limit: {} bytes (max {max_bytes})",
                crate::js::number_to_string(declared)
            )));
        }
    }
    let Some(body) = response.body.as_mut() else {
        return Err(MediaDownloadError::EmptyBody(format!(
            "{label} has no response body"
        )));
    };
    let mut output = Vec::new();
    loop {
        let chunk = body.read().await.map_err(MediaDownloadError::Transport)?;
        let Some(chunk) = chunk else {
            break;
        };
        if output.len() + chunk.len() > max_bytes {
            let _ = body.cancel().await;
            return Err(MediaDownloadError::TooLarge(format!(
                "{label} body exceeds size limit: >{max_bytes} bytes"
            )));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

/// `FetchMediaOptions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchMediaOptions {
    pub headers: Vec<(String, String)>,
    pub timeout_ms: Option<u64>,
    pub max_bytes: Option<usize>,
    pub fallback_mime_type: Option<String>,
}

/// `FetchedMedia`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedMedia {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// `fetchMediaWithLimits`: `None` for a non-2xx response or no body (the
/// body is released); a body over the cap is an error.
pub async fn fetch_media_with_limits<F: MediaFetcher>(
    fetcher: &F,
    url: &str,
    options: &FetchMediaOptions,
) -> Result<Option<FetchedMedia>, MediaDownloadError> {
    let mut response = fetcher
        .fetch(
            url,
            &options.headers,
            options.timeout_ms.unwrap_or(DEFAULT_MEDIA_TIMEOUT_MS),
        )
        .await
        .map_err(MediaDownloadError::Transport)?;
    if !(response.ok && response.body.is_some()) {
        cancel_body_quietly(response.body.as_mut()).await;
        return Ok(None);
    }
    let bytes = read_body_with_cap(
        &mut response,
        options.max_bytes.unwrap_or(DEFAULT_MEDIA_MAX_BYTES),
        "Media",
    )
    .await?;
    let mime_type = response
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| options.fallback_mime_type.clone())
        .unwrap_or_else(|| "image/png".to_owned());
    Ok(Some(FetchedMedia { bytes, mime_type }))
}
