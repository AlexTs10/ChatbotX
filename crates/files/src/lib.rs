//! File storage.
//!
//! P2.9 port of `packages/filesystem` (see `conversion/specs/P2.9.md`).
//!
//! STATUS: declarations only. The test porter added the public signatures
//! that the ported tests use, with `todo!()` bodies, so the tests compile and
//! fail. The implementer replaces every `todo!()` and adds the rest of the
//! API from spec section 8 (`Uploader`, `FilesConfig`, `object_store`, ...).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::NaiveDate;
use futures::future::{BoxFuture, Shared};
use futures::stream::BoxStream;
use http::HeaderMap;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Constants and types (spec 1.1)
// ---------------------------------------------------------------------------

pub const DEFAULT_MIME_TYPE: &str = "application/octet-stream";

pub const DEFAULT_ACL: Acl = Acl::PublicRead;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acl {
    PublicRead,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub name: String,
    pub mime_type: String,
    pub origin_path: String,
    pub size: u64,
    pub file_type: FileType,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMediaLocation {
    pub channel: String,
    pub workspace_id: String,
    pub integration_id: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum FilesError {
    /// Caller-fault failure (JS `UploadValidationError`). `Display` is the
    /// message only.
    #[error("{0}")]
    UploadValidation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("config: {0}")]
    Config(String),
    #[error("presign: {0}")]
    Presign(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("fetch: {0}")]
    Fetch(String),
}

// ---------------------------------------------------------------------------
// MIME and key helpers (spec 1.2, 1.3, 1.5)
// ---------------------------------------------------------------------------

pub fn normalize_mime_type(_mime: &str) -> String {
    todo!("P2.9: normalize_mime_type")
}

pub fn get_channel_media_prefix_at(_loc: &ChannelMediaLocation, _date: NaiveDate) -> String {
    todo!("P2.9: get_channel_media_prefix_at")
}

pub fn get_mirrored_channel_media_prefix_at(
    _loc: &ChannelMediaLocation,
    _date: NaiveDate,
) -> String {
    todo!("P2.9: get_mirrored_channel_media_prefix_at")
}

pub fn build_copy_source(
    _source_path: &str,
    _bucket: &str,
    _endpoint: Option<&str>,
) -> Result<String, FilesError> {
    todo!("P2.9: build_copy_source")
}

// ---------------------------------------------------------------------------
// HTTP abstraction (spec 5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct FetchError(pub String);

impl From<FetchError> for FilesError {
    fn from(_error: FetchError) -> Self {
        todo!("P2.9: From<FetchError> for FilesError")
    }
}

#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    pub follow_redirects: bool,
    pub cancel: Option<CancellationToken>,
}

/// Response body. `Buffered` is the JS "no body stream" path
/// (`response.body == null`, read with `arrayBuffer()`).
pub enum Body {
    Stream(BoxStream<'static, Result<Bytes, FetchError>>),
    Buffered(Bytes),
}

pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Body,
}

#[async_trait]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, url: &str, opts: FetchOptions) -> Result<HttpResponse, FetchError>;
}

// ---------------------------------------------------------------------------
// Bounded fetch (spec 1.6)
// ---------------------------------------------------------------------------

pub async fn read_body_with_limit<E>(
    _body: Body,
    _max_bytes: u64,
    _on_limit_exceeded: impl FnOnce(u64) -> E,
) -> Result<Bytes, E>
where
    E: From<FetchError>,
{
    todo!("P2.9: read_body_with_limit")
}

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

/// Returns `(final_url, response)`.
pub async fn fetch_following_safe_redirects<E>(
    _input: FollowRedirectsInput<'_, E>,
) -> Result<(String, HttpResponse), E> {
    todo!("P2.9: fetch_following_safe_redirects")
}

// ---------------------------------------------------------------------------
// Uploads (spec 1.7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutOptions {
    pub acl: Option<Acl>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
}

/// Storage port used by the upload functions (what the JS tests mock as
/// `uploader.putObject`).
#[async_trait]
pub trait ObjectPutter: Send + Sync {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutOptions,
        cancel: Option<CancellationToken>,
    ) -> Result<(), FilesError>;
}

pub struct UploadDeps {
    pub fetcher: Arc<dyn HttpFetcher>,
    pub putter: Arc<dyn ObjectPutter>,
}

pub type ValidateUrl =
    Arc<dyn Fn(String) -> BoxFuture<'static, Result<(), FilesError>> + Send + Sync>;

pub struct UploadFromUrlOptions {
    pub acl: Acl,
    pub max_bytes: Option<u64>,
    pub validate_url: Option<ValidateUrl>,
    pub cancel: Option<CancellationToken>,
}

pub async fn upload_file_from_url(
    _deps: &UploadDeps,
    _url: &str,
    _path: &str,
    _opts: UploadFromUrlOptions,
) -> Result<UploadedFile, FilesError> {
    todo!("P2.9: upload_file_from_url")
}

// ---------------------------------------------------------------------------
// Streaming upload writer (spec 1.11, 1.12)
// ---------------------------------------------------------------------------

/// The write side of a streaming upload (JS `PassThrough`).
#[async_trait]
pub trait UploadSink: Send {
    /// Resolves when the chunk is accepted (JS write callback).
    async fn write(&mut self, chunk: Bytes) -> std::io::Result<()>;
    /// JS `stream.end()`.
    fn close(&mut self);
    /// JS `stream.destroy()`.
    fn destroy(&mut self);
}

/// Outcome of the upload (JS `done` promise). Cloneable so the owner of the
/// upload can also await it.
pub type UploadDone = Shared<BoxFuture<'static, Result<(), FilesError>>>;

pub struct UploadHandle {
    pub sink: Box<dyn UploadSink>,
    pub done: UploadDone,
}

pub struct UploadWriter {}

pub fn to_upload_writer(_upload: UploadHandle) -> UploadWriter {
    todo!("P2.9: to_upload_writer")
}

impl UploadWriter {
    pub async fn write(&mut self, _chunk: &str) -> Result<(), FilesError> {
        todo!("P2.9: UploadWriter::write")
    }

    pub fn bytes_written(&self) -> u64 {
        todo!("P2.9: UploadWriter::bytes_written")
    }

    pub async fn finish(self) -> Result<(), FilesError> {
        todo!("P2.9: UploadWriter::finish")
    }

    pub async fn abort(self) {
        todo!("P2.9: UploadWriter::abort")
    }
}
