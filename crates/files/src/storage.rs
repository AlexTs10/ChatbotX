//! Object storage (spec 1.10, 1.11, 3; JS `uploader.ts`, `node-upload.ts`
//! `createUpload`, `keys.ts`).
//!
//! [`Uploader`] runs over the `object_store` crate: the local file system by
//! default, S3 by config (DECISIONS P2.9). Differences from the JS, by
//! design:
//! - The ACL is not sent. On `local` the app serves `public/` keys without
//!   auth; on S3 a bucket policy on `public/` does it.
//! - The local backend does not keep `content-type`, `cache-control` or
//!   `content-disposition` (`object_store`'s `LocalFileSystem` has no
//!   attribute store). [`ObjectHead::content_type`] is `None` there.
//! - Presigned URLs on `local` are HMAC-SHA256-signed app URLs
//!   ([`LocalUrlSigner`]), with the same expiries as S3 (upload 300 s,
//!   download 3600 s by default).
//! - `list_objects` returns every matching key at once (no continuation
//!   token); the prefix is a raw string prefix, as on S3.
//! - Keys must be valid `object_store` paths: not empty, no leading or
//!   trailing `/`, no empty, `.` or `..` segment.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt, TryStreamExt};
use hmac::{Hmac, KeyInit, Mac};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::{
    Attribute, Attributes, GetOptions, ObjectStore, ObjectStoreExt, PutMultipartOptions,
    PutPayload, WriteMultipart,
};
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::keys::encode_uri_component;
use crate::mime::normalize_mime_type;
use crate::upload::{ObjectPutter, PutOptions, UploadDeps};
use crate::writer::{UploadHandle, UploadSink};
use crate::{FilesError, ReqwestFetcher};

/// Expiry of a presigned upload URL (JS: fixed 5 min).
pub const PRESIGNED_UPLOAD_EXPIRY: Duration = Duration::from_secs(5 * 60);

/// Default expiry of a presigned download URL (JS: 1 h).
pub const DEFAULT_PRESIGNED_DOWNLOAD_EXPIRY: Duration = Duration::from_secs(60 * 60);

/// Multipart part size and parallel part uploads (JS lib-storage defaults).
const PART_SIZE: usize = 5 * 1024 * 1024;
const PART_QUEUE_SIZE: usize = 4;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// `S3_ENDPOINT`. Set: path-style requests to it. Unset: AWS, virtual
    /// hosted style.
    pub endpoint: Option<String>,
    /// `S3_REGION` (required).
    pub region: String,
    /// `S3_BUCKET` (required).
    pub bucket: String,
    /// `S3_ACCESS_KEY_ID`; used only when the secret is set too, else the
    /// AWS environment / instance credentials are used.
    pub access_key_id: Option<String>,
    /// `S3_SECRET_ACCESS_KEY`.
    pub secret_access_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// `FILES_BACKEND=local` (default): files under `root`.
    Local { root: PathBuf },
    /// `FILES_BACKEND=s3`.
    S3(S3Config),
}

#[derive(Clone, PartialEq, Eq)]
pub struct FilesConfig {
    pub backend: Backend,
    /// `FILES_SIGNING_SECRET`: HMAC key of presigned URLs on the local
    /// backend. Unset: local presigning fails.
    pub signing_secret: Option<String>,
    /// `FILES_URL_BASE` (default `/files`): base of presigned URLs on the
    /// local backend. The route that serves them is a later unit.
    pub url_base: String,
}

impl std::fmt::Debug for FilesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            Backend::Local { root } => format!("Local {{ root: {root:?} }}"),
            Backend::S3(s3) => format!(
                "S3 {{ endpoint: {:?}, region: {:?}, bucket: {:?}, access_key_id: {:?} }}",
                s3.endpoint, s3.region, s3.bucket, s3.access_key_id
            ),
        };
        f.debug_struct("FilesConfig")
            .field("backend", &format_args!("{backend}"))
            .field(
                "signing_secret",
                &self.signing_secret.as_ref().map(|_| "***"),
            )
            .field("url_base", &self.url_base)
            .finish()
    }
}

impl FilesConfig {
    /// Reads the process environment. See [`FilesConfig::from_lookup`].
    pub fn from_env() -> Result<Self, FilesError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the config through `get` (tests pass a map).
    ///
    /// - `FILES_BACKEND`: `local` (default) or `s3`.
    /// - `FILES_LOCAL_ROOT`: default `$DATA_DIR/files` (`DATA_DIR` default
    ///   `./data`).
    /// - `s3` needs `S3_REGION` and `S3_BUCKET`; `S3_ENDPOINT` must be an
    ///   http(s) URL. Set `S3_*` values must not be empty (JS `min(1)`).
    /// - `FILES_SIGNING_SECRET`, `FILES_URL_BASE` (default `/files`).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, FilesError> {
        let backend = match get("FILES_BACKEND").as_deref().map(str::trim) {
            None | Some("") | Some("local") => {
                let root = match get("FILES_LOCAL_ROOT").filter(|v| !v.is_empty()) {
                    Some(root) => PathBuf::from(root),
                    None => {
                        let data = get("DATA_DIR")
                            .filter(|v| !v.is_empty())
                            .unwrap_or_else(|| "./data".to_owned());
                        PathBuf::from(data).join("files")
                    }
                };
                Backend::Local { root }
            }
            Some("s3") => {
                let optional = |name: &str| -> Result<Option<String>, FilesError> {
                    match get(name) {
                        Some(value) if value.is_empty() => {
                            Err(FilesError::Config(format!("{name} must not be empty")))
                        }
                        other => Ok(other),
                    }
                };
                let required = |name: &str| -> Result<String, FilesError> {
                    optional(name)?.ok_or_else(|| FilesError::Config(format!("{name} is required")))
                };
                let endpoint = optional("S3_ENDPOINT")?;
                if let Some(endpoint) = &endpoint {
                    let url = url::Url::parse(endpoint).map_err(|error| {
                        FilesError::Config(format!("S3_ENDPOINT is not a URL: {error}"))
                    })?;
                    if url.scheme() != "http" && url.scheme() != "https" {
                        return Err(FilesError::Config(
                            "S3_ENDPOINT must be an http or https URL".to_owned(),
                        ));
                    }
                }
                Backend::S3(S3Config {
                    endpoint,
                    region: required("S3_REGION")?,
                    bucket: required("S3_BUCKET")?,
                    access_key_id: optional("S3_ACCESS_KEY_ID")?,
                    secret_access_key: optional("S3_SECRET_ACCESS_KEY")?,
                })
            }
            Some(other) => {
                return Err(FilesError::Config(format!(
                    "FILES_BACKEND must be local or s3, got {other:?}"
                )));
            }
        };
        Ok(Self {
            backend,
            signing_secret: get("FILES_SIGNING_SECRET").filter(|v| !v.is_empty()),
            url_base: get("FILES_URL_BASE")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "/files".to_owned()),
        })
    }
}

// ---------------------------------------------------------------------------
// Local presigned URLs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresignMethod {
    Get,
    Put,
}

impl PresignMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            PresignMethod::Get => "GET",
            PresignMethod::Put => "PUT",
        }
    }
}

/// HMAC-SHA256 signer of app URLs for the local backend:
/// `{base}/{key, each segment encodeURIComponent-ed}?expires={unix s}&signature={hex}`.
/// The signature covers `"{METHOD}\n{key}\n{expires}"`, so a download URL
/// cannot be used to upload. The serving route calls [`LocalUrlSigner::verify`].
#[derive(Clone)]
pub struct LocalUrlSigner {
    secret: Vec<u8>,
    base: String,
}

impl std::fmt::Debug for LocalUrlSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalUrlSigner")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

type HmacSha256 = Hmac<Sha256>;

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

impl LocalUrlSigner {
    /// `secret` must not be empty. A trailing `/` of `base` is dropped.
    pub fn new(secret: impl AsRef<[u8]>, base: impl Into<String>) -> Result<Self, FilesError> {
        let secret = secret.as_ref().to_vec();
        if secret.is_empty() {
            return Err(FilesError::Config("the signing secret is empty".to_owned()));
        }
        let base = base.into().trim_end_matches('/').to_owned();
        Ok(Self { secret, base })
    }

    fn mac(&self, method: PresignMethod, key: &str, expires: u64) -> HmacSha256 {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.secret)
            .expect("HMAC accepts keys of any length");
        mac.update(format!("{}\n{key}\n{expires}", method.as_str()).as_bytes());
        mac
    }

    /// A URL for `method` on `key`, valid for `expires_in` from `now`.
    pub fn sign_url(
        &self,
        method: PresignMethod,
        key: &str,
        expires_in: Duration,
        now: SystemTime,
    ) -> String {
        let expires = unix_seconds(now).saturating_add(expires_in.as_secs());
        let signature: String = self
            .mac(method, key, expires)
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let encoded_key = key
            .split('/')
            .map(encode_uri_component)
            .collect::<Vec<_>>()
            .join("/");
        format!(
            "{}/{encoded_key}?expires={expires}&signature={signature}",
            self.base
        )
    }

    /// Checks a URL made by [`sign_url`](Self::sign_url) (`key` decoded).
    /// Constant-time signature compare; expired at `now >= expires`.
    pub fn verify(
        &self,
        method: PresignMethod,
        key: &str,
        expires: u64,
        signature: &str,
        now: SystemTime,
    ) -> Result<(), FilesError> {
        let signature = hex_decode(signature)
            .ok_or_else(|| FilesError::Presign("invalid signature".to_owned()))?;
        self.mac(method, key, expires)
            .verify_slice(&signature)
            .map_err(|_| FilesError::Presign("invalid signature".to_owned()))?;
        if unix_seconds(now) >= expires {
            return Err(FilesError::Presign("the URL has expired".to_owned()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Uploader
// ---------------------------------------------------------------------------

/// JS `headObject` output (the fields callers use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectHead {
    pub content_length: u64,
    pub content_type: Option<String>,
    pub last_modified: DateTime<Utc>,
    pub e_tag: Option<String>,
}

/// JS `getObjectStream` output.
pub struct ObjectStream {
    pub stream: BoxStream<'static, Result<Bytes, FilesError>>,
    pub content_length: Option<u64>,
}

enum Presigner {
    S3 {
        store: Arc<AmazonS3>,
        has_endpoint: bool,
    },
    Local(Option<LocalUrlSigner>),
}

/// The file store. Build it once at startup and share it as `Arc<Uploader>`.
pub struct Uploader {
    store: Arc<dyn ObjectStore>,
    /// `false` on `LocalFileSystem`, which rejects object attributes.
    stores_attributes: bool,
    presigner: Presigner,
    bucket: Option<String>,
    endpoint: Option<String>,
    region: Option<String>,
}

impl std::fmt::Debug for Uploader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Uploader")
            .field("store", &format_args!("{}", self.store))
            .field("bucket", &self.bucket)
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

fn storage_error(key: &str, error: object_store::Error) -> FilesError {
    match error {
        object_store::Error::NotFound { .. } => FilesError::NotFound(key.to_owned()),
        other => FilesError::Storage(other.to_string()),
    }
}

/// A storage key as an `object_store` path (strict, see the module docs).
fn to_path(key: &str) -> Result<Path, FilesError> {
    if key.is_empty() || key.starts_with('/') || key.ends_with('/') {
        return Err(FilesError::InvalidKey(key.to_owned()));
    }
    Path::parse(key).map_err(|error| FilesError::InvalidKey(format!("{key}: {error}")))
}

impl Uploader {
    /// Builds the store for `config`. The local root is created when missing.
    pub fn new(config: &FilesConfig) -> Result<Self, FilesError> {
        match &config.backend {
            Backend::Local { root } => {
                std::fs::create_dir_all(root).map_err(|error| {
                    FilesError::Config(format!("cannot create {}: {error}", root.display()))
                })?;
                let store = LocalFileSystem::new_with_prefix(root)
                    .map_err(|error| FilesError::Config(error.to_string()))?
                    .with_automatic_cleanup(true);
                let signer = config
                    .signing_secret
                    .as_ref()
                    .map(|secret| LocalUrlSigner::new(secret, config.url_base.clone()))
                    .transpose()?;
                Ok(Self {
                    store: Arc::new(store),
                    stores_attributes: false,
                    presigner: Presigner::Local(signer),
                    bucket: None,
                    endpoint: None,
                    region: None,
                })
            }
            Backend::S3(s3) => {
                let mut builder = AmazonS3Builder::from_env()
                    .with_region(&s3.region)
                    .with_bucket_name(&s3.bucket);
                builder = match &s3.endpoint {
                    Some(endpoint) => builder
                        .with_endpoint(endpoint)
                        .with_virtual_hosted_style_request(false)
                        .with_allow_http(endpoint.starts_with("http://")),
                    None => builder.with_virtual_hosted_style_request(true),
                };
                if let (Some(key), Some(secret)) = (&s3.access_key_id, &s3.secret_access_key) {
                    builder = builder
                        .with_access_key_id(key)
                        .with_secret_access_key(secret);
                }
                let store = Arc::new(
                    builder
                        .build()
                        .map_err(|error| FilesError::Config(error.to_string()))?,
                );
                Ok(Self {
                    store: store.clone(),
                    stores_attributes: true,
                    presigner: Presigner::S3 {
                        store,
                        has_endpoint: s3.endpoint.is_some(),
                    },
                    bucket: Some(s3.bucket.clone()),
                    endpoint: s3.endpoint.clone(),
                    region: Some(s3.region.clone()),
                })
            }
        }
    }

    /// An uploader over any store that keeps attributes (for example
    /// `InMemory` in tests). Presigning needs [`with_url_signer`](Self::with_url_signer).
    pub fn with_store(store: Arc<dyn ObjectStore>) -> Self {
        Self {
            store,
            stores_attributes: true,
            presigner: Presigner::Local(None),
            bucket: None,
            endpoint: None,
            region: None,
        }
    }

    /// An in-memory uploader (tests of other crates).
    pub fn in_memory() -> Self {
        Self::with_store(Arc::new(InMemory::new()))
    }

    /// Presign with HMAC app URLs (non-S3 stores).
    pub fn with_url_signer(mut self, signer: LocalUrlSigner) -> Self {
        if let Presigner::Local(slot) = &mut self.presigner {
            *slot = Some(signer);
        }
        self
    }

    /// The S3 bucket (`None` on other backends).
    pub fn bucket_name(&self) -> Option<&str> {
        self.bucket.as_deref()
    }

    /// `S3_ENDPOINT` (`None` when unset or not S3).
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// `S3_REGION` (`None` when not S3).
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The underlying store.
    pub fn store(&self) -> &Arc<dyn ObjectStore> {
        &self.store
    }

    fn attributes(&self, options: &PutOptions) -> Attributes {
        let mut attributes = Attributes::new();
        if !self.stores_attributes {
            return attributes;
        }
        if let Some(content_type) = &options.content_type {
            attributes.insert(
                Attribute::ContentType,
                normalize_mime_type(content_type).into(),
            );
        }
        if let Some(cache_control) = &options.cache_control {
            attributes.insert(Attribute::CacheControl, cache_control.clone().into());
        }
        if let Some(disposition) = &options.content_disposition {
            attributes.insert(Attribute::ContentDisposition, disposition.clone().into());
        }
        attributes
    }

    /// Stores `body` at `path`. The content type is normalized. `acl` and
    /// `content_length` are not sent (see the module docs).
    pub async fn put_object(
        &self,
        path: &str,
        body: impl Into<Bytes>,
        options: PutOptions,
        cancel: Option<CancellationToken>,
    ) -> Result<(), FilesError> {
        let location = to_path(path)?;
        let payload = PutPayload::from_bytes(body.into());
        let opts = object_store::PutOptions::from(self.attributes(&options));
        let put = async {
            self.store
                .put_opts(&location, payload, opts)
                .await
                .map(|_| ())
                .map_err(|error| storage_error(path, error))
        };
        match cancel {
            Some(token) => token
                .run_until_cancelled(put)
                .await
                .unwrap_or(Err(FilesError::Cancelled)),
            None => put.await,
        }
    }

    async fn presign(
        &self,
        method: PresignMethod,
        path: &str,
        expires_in: Duration,
    ) -> Result<String, FilesError> {
        let location = to_path(path)?;
        match &self.presigner {
            Presigner::S3 {
                store,
                has_endpoint,
            } => {
                if !has_endpoint {
                    // The JS builds `${S3_ENDPOINT}/...` and would return
                    // "undefined/..." here.
                    return Err(FilesError::Presign("S3_ENDPOINT is not set".to_owned()));
                }
                let method = match method {
                    PresignMethod::Get => http::Method::GET,
                    PresignMethod::Put => http::Method::PUT,
                };
                store
                    .signed_url(method, &location, expires_in)
                    .await
                    .map(|url| url.to_string())
                    .map_err(|error| FilesError::Presign(error.to_string()))
            }
            Presigner::Local(Some(signer)) => {
                Ok(signer.sign_url(method, path, expires_in, SystemTime::now()))
            }
            Presigner::Local(None) => Err(FilesError::Presign(
                "FILES_SIGNING_SECRET is not set".to_owned(),
            )),
        }
    }

    /// A URL to `PUT` the object, valid for 5 minutes. Nothing about the
    /// content type or size is signed.
    pub async fn get_presigned_upload(&self, path: &str) -> Result<String, FilesError> {
        self.presign(PresignMethod::Put, path, PRESIGNED_UPLOAD_EXPIRY)
            .await
    }

    /// A URL to `GET` the object, valid for `expires_in_seconds` (default
    /// 3600). Not clamped (S3 itself caps at 7 days).
    pub async fn get_presigned_download(
        &self,
        path: &str,
        expires_in_seconds: Option<u64>,
    ) -> Result<String, FilesError> {
        let expires_in = expires_in_seconds
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_PRESIGNED_DOWNLOAD_EXPIRY);
        self.presign(PresignMethod::Get, path, expires_in).await
    }

    /// Size and type of the object. Missing = [`FilesError::NotFound`].
    pub async fn head_object(&self, path: &str) -> Result<ObjectHead, FilesError> {
        let location = to_path(path)?;
        let result = self
            .store
            .get_opts(&location, GetOptions::default().with_head(true))
            .await
            .map_err(|error| storage_error(path, error))?;
        Ok(ObjectHead {
            content_length: result.meta.size,
            content_type: result
                .attributes
                .get(&Attribute::ContentType)
                .map(|value| value.to_string()),
            last_modified: result.meta.last_modified,
            e_tag: result.meta.e_tag.clone(),
        })
    }

    /// The whole object (no size limit).
    pub async fn get_object(&self, path: &str) -> Result<Bytes, FilesError> {
        let location = to_path(path)?;
        self.store
            .get(&location)
            .await
            .map_err(|error| storage_error(path, error))?
            .bytes()
            .await
            .map_err(|error| storage_error(path, error))
    }

    /// The object as a stream of chunks, with its size.
    pub async fn get_object_stream(&self, path: &str) -> Result<ObjectStream, FilesError> {
        let location = to_path(path)?;
        let result = self
            .store
            .get(&location)
            .await
            .map_err(|error| storage_error(path, error))?;
        let content_length = Some(result.meta.size);
        let key = path.to_owned();
        let stream = result
            .into_stream()
            .map_err(move |error| storage_error(&key, error))
            .boxed();
        Ok(ObjectStream {
            stream,
            content_length,
        })
    }

    /// Server-side copy. Metadata is kept (S3 `COPY` directive); the ACL is
    /// not copied (same as the JS).
    pub async fn copy_object(&self, source: &str, destination: &str) -> Result<(), FilesError> {
        let from = to_path(source)?;
        let to = to_path(destination)?;
        self.store
            .copy(&from, &to)
            .await
            .map_err(|error| storage_error(source, error))
    }

    /// Deletes the object. A missing object is not an error (S3 semantics).
    pub async fn delete_object(&self, path: &str) -> Result<(), FilesError> {
        let location = to_path(path)?;
        match self.store.delete(&location).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(path, error)),
        }
    }

    /// Every key that starts with `prefix` (a raw string prefix, as on S3;
    /// `a/b` also matches `a/bc/x`). All pages at once.
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, FilesError> {
        let directory = match prefix.rfind('/') {
            Some(index) => &prefix[..index],
            None => "",
        };
        let list_root = if directory.is_empty() {
            None
        } else {
            match Path::parse(directory) {
                Ok(path) => Some(path),
                // No stored key can be under an invalid directory.
                Err(_) => return Ok(Vec::new()),
            }
        };
        let metas: Vec<_> = self
            .store
            .list(list_root.as_ref())
            .try_collect()
            .await
            .map_err(|error| storage_error(prefix, error))?;
        let mut keys: Vec<String> = metas
            .into_iter()
            .map(|meta| meta.location.as_ref().to_owned())
            .filter(|key| key.starts_with(prefix))
            .collect();
        keys.sort();
        Ok(keys)
    }

    /// A streaming (multipart) upload to `path` (JS `createUpload`). Write
    /// through [`crate::to_upload_writer`]. Parts of 5 MiB, 4 in parallel;
    /// on failure or [`UploadSink::destroy`] the multipart upload is aborted.
    /// Dropping the sink without `close` also aborts. Must be called inside
    /// a Tokio runtime (the upload runs as a task).
    pub fn create_upload(
        &self,
        path: &str,
        content_type: Option<&str>,
    ) -> Result<UploadHandle, FilesError> {
        let location = to_path(path)?;
        let attributes = self.attributes(&PutOptions {
            content_type: content_type.map(str::to_owned),
            ..PutOptions::default()
        });
        let (tx, rx) = mpsc::channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let abort = CancellationToken::new();
        let task = tokio::spawn(run_upload(
            self.store.clone(),
            location,
            attributes,
            rx,
            finished.clone(),
            abort.clone(),
        ));
        let done = async move {
            task.await
                .unwrap_or_else(|error| Err(FilesError::Storage(error.to_string())))
        }
        .boxed()
        .shared();
        Ok(UploadHandle {
            sink: Box::new(ChannelSink {
                tx: Some(tx),
                finished,
                abort,
            }),
            done,
        })
    }

    /// Upload dependencies over this store and a [`ReqwestFetcher`].
    pub fn upload_deps(self: &Arc<Self>) -> Result<UploadDeps, FilesError> {
        Ok(UploadDeps {
            fetcher: Arc::new(ReqwestFetcher::new()?),
            putter: self.clone(),
        })
    }
}

#[async_trait]
impl ObjectPutter for Uploader {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutOptions,
        cancel: Option<CancellationToken>,
    ) -> Result<(), FilesError> {
        Uploader::put_object(self, path, body, options, cancel).await
    }
}

// ---------------------------------------------------------------------------
// Streaming upload task
// ---------------------------------------------------------------------------

/// Dropping the sink without `close` ends the channel with `finished`
/// unset, so the task aborts the upload.
struct ChannelSink {
    tx: Option<mpsc::Sender<Bytes>>,
    finished: Arc<AtomicBool>,
    abort: CancellationToken,
}

#[async_trait]
impl UploadSink for ChannelSink {
    async fn write(&mut self, chunk: Bytes) -> io::Result<()> {
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "write after end"))?;
        tx.send(chunk)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the upload has stopped"))
    }

    fn close(&mut self) {
        if self.tx.is_some() {
            self.finished.store(true, Ordering::SeqCst);
            self.tx = None;
        }
    }

    fn destroy(&mut self) {
        self.abort.cancel();
        self.tx = None;
    }
}

async fn run_upload(
    store: Arc<dyn ObjectStore>,
    location: Path,
    attributes: Attributes,
    mut rx: mpsc::Receiver<Bytes>,
    finished: Arc<AtomicBool>,
    abort: CancellationToken,
) -> Result<(), FilesError> {
    let key = location.to_string();
    let options = PutMultipartOptions {
        attributes,
        ..PutMultipartOptions::default()
    };
    let upload = store
        .put_multipart_opts(&location, options)
        .await
        .map_err(|error| storage_error(&key, error))?;
    let mut writer = WriteMultipart::new_with_chunk_size(upload, PART_SIZE);
    loop {
        match abort.run_until_cancelled(rx.recv()).await {
            None => {
                let _ = writer.abort().await;
                return Err(FilesError::Cancelled);
            }
            Some(Some(chunk)) => {
                if let Err(error) = writer.wait_for_capacity(PART_QUEUE_SIZE).await {
                    let _ = writer.abort().await;
                    return Err(storage_error(&key, error));
                }
                writer.put(chunk);
            }
            Some(None) => break,
        }
    }
    if !finished.load(Ordering::SeqCst) {
        let _ = writer.abort().await;
        return Err(FilesError::Storage(
            "the upload stream was dropped before it was closed".to_owned(),
        ));
    }
    writer
        .finish()
        .await
        .map(|_| ())
        .map_err(|error| storage_error(&key, error))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{Acl, to_upload_writer};

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "chatbotx-files-{name}-{}-{}",
                std::process::id(),
                crate::id::create_id()
            ));
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn local(dir: &TempDir, secret: Option<&str>) -> Uploader {
        let config = FilesConfig {
            backend: Backend::Local {
                root: dir.0.clone(),
            },
            signing_secret: secret.map(str::to_owned),
            url_base: "https://app.test/files/".to_owned(),
        };
        Uploader::new(&config).expect("uploader")
    }

    fn s3_config(endpoint: Option<&str>) -> FilesConfig {
        FilesConfig {
            backend: Backend::S3(S3Config {
                endpoint: endpoint.map(str::to_owned),
                region: "us-west-2".to_owned(),
                bucket: "test".to_owned(),
                access_key_id: Some("AKIDEXAMPLE".to_owned()),
                secret_access_key: Some("secret".to_owned()),
            }),
            signing_secret: None,
            url_base: "/files".to_owned(),
        }
    }

    #[test]
    fn config_defaults_to_the_local_backend() {
        let config = FilesConfig::from_lookup(lookup(&[])).expect("config");
        assert_eq!(
            config.backend,
            Backend::Local {
                root: PathBuf::from("./data/files")
            }
        );
        assert_eq!(config.url_base, "/files");
        assert_eq!(config.signing_secret, None);

        let config = FilesConfig::from_lookup(lookup(&[
            ("DATA_DIR", "/var/lib/x"),
            ("FILES_SIGNING_SECRET", "k"),
        ]))
        .expect("config");
        assert_eq!(
            config.backend,
            Backend::Local {
                root: PathBuf::from("/var/lib/x/files")
            }
        );
        assert!(!format!("{config:?}").contains("\"k\""));

        let config = FilesConfig::from_lookup(lookup(&[
            ("FILES_BACKEND", "local"),
            ("FILES_LOCAL_ROOT", "/srv/files"),
            ("S3_BUCKET", "ignored"),
        ]))
        .expect("config");
        assert_eq!(
            config.backend,
            Backend::Local {
                root: PathBuf::from("/srv/files")
            }
        );
    }

    #[test]
    fn config_reads_and_validates_s3() {
        let config = FilesConfig::from_lookup(lookup(&[
            ("FILES_BACKEND", "s3"),
            ("S3_ENDPOINT", "http://localhost:9000"),
            ("S3_REGION", "us-west-2"),
            ("S3_BUCKET", "chatbotx"),
            ("S3_ACCESS_KEY_ID", "id"),
            ("S3_SECRET_ACCESS_KEY", "secret"),
        ]))
        .expect("config");
        assert_eq!(
            config.backend,
            Backend::S3(S3Config {
                endpoint: Some("http://localhost:9000".to_owned()),
                region: "us-west-2".to_owned(),
                bucket: "chatbotx".to_owned(),
                access_key_id: Some("id".to_owned()),
                secret_access_key: Some("secret".to_owned()),
            })
        );
        assert!(!format!("{config:?}").contains("\"secret\""));

        let missing = FilesConfig::from_lookup(lookup(&[("FILES_BACKEND", "s3")]));
        assert!(matches!(missing, Err(FilesError::Config(m)) if m.contains("S3_REGION")));
        let bad_url = FilesConfig::from_lookup(lookup(&[
            ("FILES_BACKEND", "s3"),
            ("S3_ENDPOINT", "localhost"),
            ("S3_REGION", "r"),
            ("S3_BUCKET", "b"),
        ]));
        assert!(matches!(bad_url, Err(FilesError::Config(_))));
        let empty = FilesConfig::from_lookup(lookup(&[
            ("FILES_BACKEND", "s3"),
            ("S3_REGION", "r"),
            ("S3_BUCKET", "b"),
            ("S3_ACCESS_KEY_ID", ""),
        ]));
        assert!(matches!(empty, Err(FilesError::Config(_))));
        let unknown = FilesConfig::from_lookup(lookup(&[("FILES_BACKEND", "gcs")]));
        assert!(matches!(unknown, Err(FilesError::Config(_))));
    }

    #[test]
    fn keys_are_strict_paths() {
        for bad in ["", "/a", "a/", "a//b", "a/../b", "a/./b"] {
            assert!(
                matches!(to_path(bad), Err(FilesError::InvalidKey(_))),
                "{bad:?} must be rejected"
            );
        }
        assert_eq!(
            to_path("public/ws/1/a b#c.png").expect("valid").as_ref(),
            "public/ws/1/a b#c.png"
        );
    }

    #[test]
    fn local_signer_signs_and_verifies() {
        let signer = LocalUrlSigner::new("secret", "https://app.test/files/").expect("signer");
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let url = signer.sign_url(
            PresignMethod::Put,
            "public/a b.png",
            Duration::from_secs(300),
            now,
        );
        assert!(
            url.starts_with("https://app.test/files/public/a%20b.png?expires=1300&signature="),
            "{url}"
        );
        let signature = url.rsplit("signature=").next().expect("signature");
        assert_eq!(signature.len(), 64);

        let key = "public/a b.png";
        signer
            .verify(PresignMethod::Put, key, 1300, signature, now)
            .expect("valid");
        // Another method, key, expiry, secret, or a late request fails.
        assert!(
            signer
                .verify(PresignMethod::Get, key, 1300, signature, now)
                .is_err()
        );
        assert!(
            signer
                .verify(PresignMethod::Put, "public/b.png", 1300, signature, now)
                .is_err()
        );
        assert!(
            signer
                .verify(PresignMethod::Put, key, 1301, signature, now)
                .is_err()
        );
        assert!(
            signer
                .verify(PresignMethod::Put, key, 1300, "zz", now)
                .is_err()
        );
        let other = LocalUrlSigner::new("other", "/files").expect("signer");
        assert!(
            other
                .verify(PresignMethod::Put, key, 1300, signature, now)
                .is_err()
        );
        let late = UNIX_EPOCH + Duration::from_secs(1_300);
        assert!(matches!(
            signer.verify(PresignMethod::Put, key, 1300, signature, late),
            Err(FilesError::Presign(m)) if m.contains("expired")
        ));
        assert!(LocalUrlSigner::new("", "/files").is_err());
    }

    #[tokio::test]
    async fn local_presigned_urls_use_the_same_expiries() {
        let dir = TempDir::new("presign");
        let uploader = local(&dir, Some("secret"));
        let before = unix_seconds(SystemTime::now());
        let expires = |url: &str| -> u64 {
            url.split("expires=")
                .nth(1)
                .expect("expires")
                .split('&')
                .next()
                .expect("value")
                .parse()
                .expect("number")
        };
        let upload = uploader
            .get_presigned_upload("public/a.png")
            .await
            .expect("url");
        assert!(upload.starts_with("https://app.test/files/public/a.png?"));
        let delta = expires(&upload) - before;
        assert!((300..=302).contains(&delta), "{delta}");
        let download = uploader
            .get_presigned_download("public/a.png", None)
            .await
            .expect("url");
        let delta = expires(&download) - before;
        assert!((3600..=3602).contains(&delta), "{delta}");
        let short = uploader
            .get_presigned_download("public/a.png", Some(60))
            .await
            .expect("url");
        let delta = expires(&short) - before;
        assert!((60..=62).contains(&delta), "{delta}");

        let unsigned = local(&TempDir::new("nosecret"), None);
        assert!(matches!(
            unsigned.get_presigned_upload("a").await,
            Err(FilesError::Presign(_))
        ));
    }

    #[tokio::test]
    async fn s3_presigned_urls_are_path_style_sigv4() {
        let uploader = Uploader::new(&s3_config(Some("http://localhost:9000"))).expect("uploader");
        assert_eq!(uploader.bucket_name(), Some("test"));
        assert_eq!(uploader.endpoint(), Some("http://localhost:9000"));
        assert_eq!(uploader.region(), Some("us-west-2"));

        let upload = uploader
            .get_presigned_upload("public/ws/1/a.png")
            .await
            .expect("url");
        assert!(
            upload.starts_with("http://localhost:9000/test/public/ws/1/a.png?"),
            "{upload}"
        );
        for part in [
            "X-Amz-Algorithm=AWS4-HMAC-SHA256",
            "X-Amz-Credential=AKIDEXAMPLE",
            "X-Amz-Expires=300",
            "X-Amz-SignedHeaders=host",
            "X-Amz-Signature=",
        ] {
            assert!(upload.contains(part), "{part} missing in {upload}");
        }
        let download = uploader
            .get_presigned_download("public/ws/1/a.png", None)
            .await
            .expect("url");
        assert!(download.contains("X-Amz-Expires=3600"), "{download}");
        let short = uploader
            .get_presigned_download("public/ws/1/a.png", Some(300))
            .await
            .expect("url");
        assert!(short.contains("X-Amz-Expires=300"), "{short}");

        let aws = Uploader::new(&s3_config(None)).expect("uploader");
        assert!(matches!(
            aws.get_presigned_upload("a").await,
            Err(FilesError::Presign(m)) if m.contains("S3_ENDPOINT")
        ));
    }

    #[tokio::test]
    async fn local_backend_round_trip() {
        let dir = TempDir::new("roundtrip");
        let uploader = local(&dir, None);
        let options = PutOptions {
            acl: Some(Acl::PublicRead),
            content_type: Some("image/jpg".to_owned()),
            content_length: Some(5),
            cache_control: Some("no-cache".to_owned()),
            content_disposition: None,
        };
        uploader
            .put_object(
                "public/ws/1/a.txt",
                Bytes::from_static(b"hello"),
                options,
                None,
            )
            .await
            .expect("put");
        uploader
            .put_object("public/ws/10/b.txt", "bb", PutOptions::default(), None)
            .await
            .expect("put");
        uploader
            .put_object("workspace/c.txt", "c", PutOptions::default(), None)
            .await
            .expect("put");
        assert!(dir.0.join("public/ws/1/a.txt").is_file());

        let head = uploader
            .head_object("public/ws/1/a.txt")
            .await
            .expect("head");
        assert_eq!(head.content_length, 5);
        assert_eq!(head.content_type, None, "local keeps no attributes");
        assert_eq!(
            &uploader.get_object("public/ws/1/a.txt").await.expect("get")[..],
            b"hello"
        );
        let stream = uploader
            .get_object_stream("public/ws/10/b.txt")
            .await
            .expect("stream");
        assert_eq!(stream.content_length, Some(2));
        let chunks: Vec<Bytes> = stream.stream.try_collect().await.expect("chunks");
        assert_eq!(chunks.concat(), b"bb");

        // Raw string prefix, like S3.
        assert_eq!(
            uploader.list_objects("public/ws/1").await.expect("list"),
            vec!["public/ws/1/a.txt", "public/ws/10/b.txt"]
        );
        assert_eq!(
            uploader.list_objects("public/ws/1/").await.expect("list"),
            vec!["public/ws/1/a.txt"]
        );
        assert_eq!(uploader.list_objects("").await.expect("list").len(), 3);
        assert!(
            uploader
                .list_objects("nothing/here")
                .await
                .expect("list")
                .is_empty()
        );

        uploader
            .copy_object("public/ws/1/a.txt", "workspace/copy.txt")
            .await
            .expect("copy");
        assert_eq!(
            &uploader
                .get_object("workspace/copy.txt")
                .await
                .expect("get")[..],
            b"hello"
        );
        uploader
            .delete_object("workspace/copy.txt")
            .await
            .expect("delete");
        uploader
            .delete_object("workspace/copy.txt")
            .await
            .expect("idempotent delete");
        assert!(matches!(
            uploader.get_object("workspace/copy.txt").await,
            Err(FilesError::NotFound(key)) if key == "workspace/copy.txt"
        ));
        assert!(matches!(
            uploader.head_object("missing").await,
            Err(FilesError::NotFound(_))
        ));
        assert!(matches!(
            uploader
                .put_object("a//b", "x", PutOptions::default(), None)
                .await,
            Err(FilesError::InvalidKey(_))
        ));
    }

    #[tokio::test]
    async fn in_memory_keeps_the_normalized_content_type() {
        let uploader = Uploader::in_memory();
        uploader
            .put_object(
                "public/p.jpg",
                "x",
                PutOptions {
                    content_type: Some("IMAGE/JPG".to_owned()),
                    ..PutOptions::default()
                },
                None,
            )
            .await
            .expect("put");
        let head = uploader.head_object("public/p.jpg").await.expect("head");
        assert_eq!(head.content_type.as_deref(), Some("image/jpeg"));
    }

    #[tokio::test]
    async fn a_cancelled_put_is_not_stored() {
        let uploader = Uploader::in_memory();
        let token = CancellationToken::new();
        token.cancel();
        let result = uploader
            .put_object("a", "x", PutOptions::default(), Some(token))
            .await;
        assert!(matches!(result, Err(FilesError::Cancelled)));
        assert!(matches!(
            uploader.head_object("a").await,
            Err(FilesError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn streaming_upload_writes_the_object_on_finish() {
        let dir = TempDir::new("stream");
        let uploader = local(&dir, None);
        let upload = uploader
            .create_upload("exports/a.csv", Some("text/csv; charset=utf-8"))
            .expect("upload");
        let mut writer = to_upload_writer(upload);
        writer.write("\u{FEFF}a,b\n").await.expect("write");
        writer.write("1,2\n").await.expect("write");
        assert_eq!(writer.bytes_written(), 11);
        writer.finish().await.expect("finish");
        assert_eq!(
            &uploader.get_object("exports/a.csv").await.expect("get")[..],
            "\u{FEFF}a,b\n1,2\n".as_bytes()
        );

        // In memory, the content type is kept.
        let memory = Uploader::in_memory();
        let mut writer = to_upload_writer(
            memory
                .create_upload("exports/b.csv", Some("TEXT/CSV"))
                .expect("upload"),
        );
        writer.write("x").await.expect("write");
        writer.finish().await.expect("finish");
        let head = memory.head_object("exports/b.csv").await.expect("head");
        assert_eq!(head.content_type.as_deref(), Some("text/csv"));
    }

    #[tokio::test]
    async fn an_aborted_or_dropped_streaming_upload_stores_nothing() {
        let uploader = Uploader::in_memory();
        let mut writer = to_upload_writer(
            uploader
                .create_upload("exports/x.csv", None)
                .expect("upload"),
        );
        writer.write("data").await.expect("write");
        writer.abort().await;
        assert!(matches!(
            uploader.head_object("exports/x.csv").await,
            Err(FilesError::NotFound(_))
        ));

        let upload = uploader
            .create_upload("exports/y.csv", None)
            .expect("upload");
        let done = upload.done.clone();
        drop(upload);
        assert!(done.await.is_err());
        assert!(matches!(
            uploader.head_object("exports/y.csv").await,
            Err(FilesError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn uploader_is_an_object_putter() {
        let uploader = Arc::new(Uploader::in_memory());
        let putter: &dyn ObjectPutter = uploader.as_ref();
        putter
            .put_object("k", Bytes::from_static(b"v"), PutOptions::default(), None)
            .await
            .expect("put");
        assert_eq!(&uploader.get_object("k").await.expect("get")[..], b"v");
        let deps = uploader.upload_deps().expect("deps");
        drop(deps);
    }
}
