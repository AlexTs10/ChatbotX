//! File storage.
//!
//! P2.9 port of `packages/filesystem` (see `conversion/specs/P2.9.md` and the
//! "P2.9 files crate" entry of `conversion/DECISIONS.md`).
//!
//! - [`mime`]: MIME normalization and file-type guess.
//! - [`keys`]: storage key and prefix helpers, `build_copy_source`.
//! - [`image`]: image dimensions (raster formats plus a std-only SVG parse).
//! - [`fetch`]: the HTTP abstraction, bounded body reads and safe redirects.
//! - [`upload`]: `upload_file`, `upload_multiple_files`, `upload_file_from_url`.
//! - [`writer`]: the streaming upload writer (`to_upload_writer`).
//! - [`storage`]: `FilesConfig`, `Uploader` over `object_store` (local file
//!   system by default, S3 by config) and presigned URLs.

pub mod fetch;
pub mod image;
pub mod keys;
pub mod mime;
pub mod storage;
pub mod upload;
pub mod writer;

mod id;

pub use fetch::{
    Body, FetchError, FetchImpl, FetchOptions, FollowRedirectsInput, HttpFetcher, HttpResponse,
    InvalidRedirectLocationFn, ReqwestFetcher, SafeRedirectErrors, ValidateUrlFn,
    fetch_following_safe_redirects, read_body_with_limit,
};
pub use image::{ImageDimensions, get_image_dimensions};
pub use keys::{
    build_copy_source, get_channel_media_prefix, get_channel_media_prefix_at,
    get_mirrored_channel_media_prefix, get_mirrored_channel_media_prefix_at, get_storage_prefix,
    get_storage_prefix_at, path_join,
};
pub use mime::{guess_file_type_from_mime_type, normalize_mime_type};
pub use storage::{
    Backend, DEFAULT_PRESIGNED_DOWNLOAD_EXPIRY, FilesConfig, LocalUrlSigner,
    PRESIGNED_UPLOAD_EXPIRY, PresignMethod, S3Config, Uploader,
};
pub use upload::{
    InputFile, ObjectPutter, PutOptions, UploadDeps, UploadFromUrlOptions, ValidateUrl,
    upload_file, upload_file_from_url, upload_multiple_files,
};
pub use writer::{UploadDone, UploadHandle, UploadSink, UploadWriter, to_upload_writer};

// ---------------------------------------------------------------------------
// Constants and types (spec 1.1)
// ---------------------------------------------------------------------------

pub const DEFAULT_MIME_TYPE: &str = "application/octet-stream";

pub const DEFAULT_ACL: Acl = Acl::PublicRead;

/// Canned ACL of a stored object. Kept for API parity: the local backend
/// serves keys under `public/` without auth, and on S3 a bucket policy on
/// `public/` makes them public (DECISIONS P2.9). The value is not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acl {
    PublicRead,
    Private,
}

impl Acl {
    /// The S3 canned ACL name.
    pub fn as_str(self) -> &'static str {
        match self {
            Acl::PublicRead => "public-read",
            Acl::Private => "private",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Image,
    Video,
    Audio,
    File,
}

impl FileType {
    /// The JS string value (`"image" | "video" | "audio" | "file"`).
    pub fn as_str(self) -> &'static str {
        match self {
            FileType::Image => "image",
            FileType::Video => "video",
            FileType::Audio => "audio",
            FileType::File => "file",
        }
    }
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
    /// The caller's cancellation token fired (JS `AbortError`). Not a
    /// validation error.
    #[error("cancelled")]
    Cancelled,
}

impl FilesError {
    /// JS `error instanceof UploadValidationError`.
    pub fn is_upload_validation(&self) -> bool {
        matches!(self, FilesError::UploadValidation(_))
    }
}
