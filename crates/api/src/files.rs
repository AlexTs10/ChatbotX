//! The local file storage route (P3.6-D2 files service): `PUT` / `GET`
//! `/files/{key}` for the presigned URLs of the local backend
//! (`chatbotx_files::LocalUrlSigner`). The original hands out S3 presigned
//! URLs; on the local backend the app itself serves them.
//!
//! - `PUT` stores the body at `key` (signature for `PUT`); `200` empty body,
//!   like S3.
//! - `GET` returns the object (signature for `GET`); `404` when missing.
//! - A bad or missing signature is `403`, an expired URL `403`, a key that is
//!   not a valid storage key (empty / `..` / `.` segments) `400`. Errors are
//!   plain text (S3 answers XML; no client reads the body).
//! - Bodies up to [`MAX_UPLOAD_BYTES`].

use std::sync::Arc;
use std::time::SystemTime;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_files::{FilesError, PresignMethod, PutOptions, Uploader};

/// The largest accepted upload body (64 MiB).
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// `X-Amz-Expires` of a presigned upload (`PRESIGNED_UPLOAD_EXPIRY`, 300 s).
pub const PRESIGNED_UPLOAD_EXPIRES: u64 = 300;

/// The `/files/{*key}` routes over `files` (the same store as
/// `ApiState::files`). A store without a URL signer (S3) answers `404`.
pub fn router(files: Arc<Uploader>) -> Router {
    Router::new()
        .route("/files/{*key}", get(download).put(upload))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .with_state(files)
}

/// `uploader.getPresignedUpload(path)`. A local URL also carries
/// `X-Amz-Expires=300` (clients read the expiry from it, as on S3; the
/// signature does not cover it).
pub async fn presigned_upload_url(uploader: &Uploader, key: &str) -> Result<String, FilesError> {
    let url = uploader.get_presigned_upload(key).await?;
    if url.contains("X-Amz-Expires=") {
        return Ok(url);
    }
    Ok(url.replacen(
        '?',
        &format!("?X-Amz-Expires={PRESIGNED_UPLOAD_EXPIRES}&"),
        1,
    ))
}

fn text(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.to_string(),
    )
        .into_response()
}

/// `expires` and `signature` of the query string.
fn signed_params(query: Option<&str>) -> Option<(u64, String)> {
    let mut expires = None;
    let mut signature = None;
    for pair in query.unwrap_or_default().split('&') {
        match pair.split_once('=') {
            Some(("expires", value)) => expires = value.parse::<u64>().ok(),
            Some(("signature", value)) => signature = Some(value.to_string()),
            _ => {}
        }
    }
    Some((expires?, signature?))
}

/// `None` when the URL is valid for `method`, else the error answer.
fn authorize(
    uploader: &Uploader,
    method: PresignMethod,
    key: &str,
    query: Option<&str>,
) -> Option<Response> {
    let Some(signer) = uploader.url_signer() else {
        return Some(text(StatusCode::NOT_FOUND, "Not Found"));
    };
    let Some((expires, signature)) = signed_params(query) else {
        return Some(text(StatusCode::FORBIDDEN, "Missing signature"));
    };
    if let Err(error) = signer.verify(method, key, expires, &signature, SystemTime::now()) {
        return Some(text(StatusCode::FORBIDDEN, &error.to_string()));
    }
    None
}

/// The content type of a stored key by its extension (the local backend
/// keeps no object attributes).
fn content_type_of(key: &str) -> &'static str {
    let extension = key
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "csv" => "text/csv",
        "json" => "application/json",
        "txt" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

fn storage_error(error: &FilesError) -> Response {
    match error {
        FilesError::InvalidKey(_) => text(StatusCode::BAD_REQUEST, "Invalid key"),
        FilesError::NotFound(_) => text(StatusCode::NOT_FOUND, "Not Found"),
        other => {
            tracing::warn!(error = %other, "file storage request failed");
            text(StatusCode::INTERNAL_SERVER_ERROR, "Storage error")
        }
    }
}

async fn upload(
    State(uploader): State<Arc<Uploader>>,
    Path(key): Path<String>,
    RawQuery(query): RawQuery,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = authorize(&uploader, PresignMethod::Put, &key, query.as_deref()) {
        return response;
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let options = PutOptions {
        content_type,
        ..PutOptions::default()
    };
    match uploader.put_object(&key, body, options, None).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => storage_error(&error),
    }
}

async fn download(
    State(uploader): State<Arc<Uploader>>,
    Path(key): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(response) = authorize(&uploader, PresignMethod::Get, &key, query.as_deref()) {
        return response;
    }
    match uploader.get_object(&key).await {
        Ok(bytes) => {
            let mime = content_type_of(&key);
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(error) => storage_error(&error),
    }
}
