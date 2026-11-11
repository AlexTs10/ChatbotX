//! Uploads (spec 1.7 - 1.9; JS `upload.ts`).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use crate::fetch::{
    FetchImpl, FetchOptions, FollowRedirectsInput, HttpFetcher, HttpResponse, SafeRedirectErrors,
    fetch_following_safe_redirects, read_body_with_limit,
};
use crate::id::create_id;
use crate::image::get_image_dimensions;
use crate::keys::{decode_uri_component, path_join};
use crate::mime::normalize_mime_type;
use crate::{Acl, DEFAULT_MIME_TYPE, FilesError, UploadedFile};

/// Options of a put (the subset of S3 `PutObjectCommandInput` callers use).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutOptions {
    pub acl: Option<Acl>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
}

/// Storage port used by the upload functions (what the JS tests mock as
/// `uploader.putObject`). [`crate::Uploader`] implements it.
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
    /// JS default `"public-read"` ([`crate::DEFAULT_ACL`]).
    pub acl: Acl,
    /// `None` = no limit.
    pub max_bytes: Option<u64>,
    /// When set, redirects are followed by hand and every hop is validated.
    pub validate_url: Option<ValidateUrl>,
    /// Bounds both the download and the put.
    pub cancel: Option<CancellationToken>,
}

impl Default for UploadFromUrlOptions {
    fn default() -> Self {
        Self {
            acl: crate::DEFAULT_ACL,
            max_bytes: None,
            validate_url: None,
            cancel: None,
        }
    }
}

/// A file to upload (JS DOM `File`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    pub name: String,
    pub mime_type: String,
    pub bytes: Bytes,
}

fn size_error(limit: u64) -> FilesError {
    FilesError::UploadValidation(format!(
        "File exceeds the maximum allowed size of {limit} bytes"
    ))
}

/// JS `Number.parseInt(text, 10)`: leading white space, an optional sign,
/// then the longest run of digits. No digits = `NaN` = `None`.
fn parse_int(text: &str) -> Option<i128> {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    // Saturate very long digit runs (JS would give a huge float).
    let value = digits[..end].parse::<i128>().unwrap_or(i128::MAX);
    Some(if negative { -value } else { value })
}

/// The file name from the last path segment of `url`, decoded. `None` when
/// the URL does not parse, the segment is empty, or decoding fails.
fn name_from_url(url: &str) -> Option<String> {
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(target: "filesystem:upload", err = %error, "uploadFileFromUrl: invalid URL");
            return None;
        }
    };
    let last = parsed.path().rsplit('/').next().unwrap_or_default();
    if last.is_empty() {
        return None;
    }
    match decode_uri_component(last) {
        Ok(name) => Some(name),
        Err(error) => {
            tracing::warn!(target: "filesystem:upload", err = %error, "uploadFileFromUrl: invalid URL");
            None
        }
    }
}

async fn until_cancelled<T>(
    cancel: Option<&CancellationToken>,
    future: impl Future<Output = Result<T, FilesError>>,
) -> Result<T, FilesError> {
    match cancel {
        Some(token) => token
            .run_until_cancelled(future)
            .await
            .unwrap_or(Err(FilesError::Cancelled)),
        None => future.await,
    }
}

/// Downloads `url` and stores it at `path`.
///
/// - With `validate_url`: redirects are followed by hand (5 hops), and
///   `validate_url` runs before every request. Without: one request that
///   follows redirects itself, with no validation.
/// - A final status outside 200-299 is an `UploadValidation` error.
/// - The MIME type is the `content-type` header without parameters,
///   normalized (`application/octet-stream` when absent or empty).
/// - A `content-length` above `max_bytes` is rejected before the body is
///   read; the body itself is also capped. `size` is the real byte count.
/// - The name is the decoded last path segment of the final URL, else a new id.
/// - Cancellation, network and storage errors are not validation errors.
pub async fn upload_file_from_url(
    deps: &UploadDeps,
    url: &str,
    path: &str,
    opts: UploadFromUrlOptions,
) -> Result<UploadedFile, FilesError> {
    let UploadFromUrlOptions {
        acl,
        max_bytes,
        validate_url,
        cancel,
    } = opts;

    let (response, final_url) = match validate_url {
        Some(validate_url) => {
            let errors = SafeRedirectErrors::<FilesError> {
                too_many_redirects: Box::new(|| {
                    FilesError::UploadValidation(
                        "Too many redirects while downloading file".to_owned(),
                    )
                }),
                no_location_header: Box::new(|| {
                    FilesError::UploadValidation(
                        "Redirect response has no Location header".to_owned(),
                    )
                }),
                invalid_redirect_location: Box::new(|location, _cause| {
                    FilesError::UploadValidation(format!(
                        "Redirect response has an invalid Location header: {location}"
                    ))
                }),
            };
            let fetcher = deps.fetcher.clone();
            let fetch_cancel = cancel.clone();
            let fetch_impl =
                move |candidate: String| -> BoxFuture<'static, Result<HttpResponse, FilesError>> {
                    let fetcher = fetcher.clone();
                    let cancel = fetch_cancel.clone();
                    async move {
                        let opts = FetchOptions {
                            follow_redirects: false,
                            cancel,
                        };
                        Ok(fetcher.fetch(&candidate, opts).await?)
                    }
                    .boxed()
                };
            let fetch_impl: &FetchImpl<FilesError> = &fetch_impl;
            let (final_url, response) = fetch_following_safe_redirects(FollowRedirectsInput {
                errors: &errors,
                fetch_impl,
                max_redirect_hops: None,
                url: url.to_owned(),
                validate_url: &*validate_url,
            })
            .await?;
            (response, final_url)
        }
        None => {
            let opts = FetchOptions {
                follow_redirects: true,
                cancel: cancel.clone(),
            };
            let response = deps.fetcher.fetch(url, opts).await?;
            (response, url.to_owned())
        }
    };

    if !response.ok() {
        return Err(FilesError::UploadValidation(format!(
            "Failed to download file: {}",
            response.status
        )));
    }

    let content_type = response
        .header("content-type")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MIME_TYPE.to_owned());
    let mime_type = normalize_mime_type(content_type.split(';').next().unwrap_or_default());

    let header_length = parse_int(
        &response
            .header("content-length")
            .unwrap_or_else(|| "0".to_owned()),
    );
    if let (Some(max), Some(length)) = (max_bytes, header_length) {
        if length > i128::from(max) {
            return Err(size_error(max));
        }
    }

    let name = name_from_url(&final_url).unwrap_or_else(create_id);

    let read = async {
        match max_bytes {
            None => Ok(response.body.bytes().await?),
            Some(max) => read_body_with_limit(response.body, max, size_error).await,
        }
    };
    let buffer = until_cancelled(cancel.as_ref(), read).await?;
    let size = buffer.len() as u64;

    deps.putter
        .put_object(
            path,
            buffer.clone(),
            PutOptions {
                acl: Some(acl),
                content_type: Some(mime_type.clone()),
                content_length: Some(size),
                ..PutOptions::default()
            },
            cancel,
        )
        .await?;

    let dims = get_image_dimensions(&mime_type, &buffer);
    Ok(UploadedFile {
        name,
        mime_type: dims.mime_type,
        origin_path: path.to_owned(),
        size,
        file_type: dims.file_type,
        width: dims.width,
        height: dims.height,
    })
}

/// Stores `file` at `path`. The MIME type is normalized; an empty type stays
/// empty (the JS does not apply the default here).
pub async fn upload_file(
    putter: &dyn ObjectPutter,
    file: &InputFile,
    path: &str,
    acl: Acl,
) -> Result<UploadedFile, FilesError> {
    let mime_type = normalize_mime_type(&file.mime_type);
    let size = file.bytes.len() as u64;
    putter
        .put_object(
            path,
            file.bytes.clone(),
            PutOptions {
                acl: Some(acl),
                content_type: Some(mime_type.clone()),
                content_length: Some(size),
                ..PutOptions::default()
            },
            None,
        )
        .await?;

    let dims = get_image_dimensions(&mime_type, &file.bytes);
    Ok(UploadedFile {
        name: file.name.clone(),
        mime_type: dims.mime_type,
        origin_path: path.to_owned(),
        size,
        file_type: dims.file_type,
        width: dims.width,
        height: dims.height,
    })
}

/// Uploads every file in parallel to `{prefix}/{new id}` (no extension).
/// Fails fast; files already stored are not removed. Result order = input
/// order.
pub async fn upload_multiple_files(
    putter: &dyn ObjectPutter,
    files: &[InputFile],
    prefix: &str,
    acl: Acl,
) -> Result<Vec<UploadedFile>, FilesError> {
    futures::future::try_join_all(files.iter().map(|file| {
        let path = path_join(&[prefix, &create_id()]);
        async move { upload_file(putter, file, &path, acl).await }
    }))
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::StreamExt;
    use http::HeaderMap;

    use super::*;
    use crate::fetch::{Body, FetchError};
    use crate::{DEFAULT_ACL, FileType};

    #[derive(Default)]
    struct Putter {
        calls: Mutex<Vec<(String, Bytes, PutOptions)>>,
        fail: bool,
    }

    #[async_trait]
    impl ObjectPutter for Putter {
        async fn put_object(
            &self,
            path: &str,
            body: Bytes,
            options: PutOptions,
            _cancel: Option<CancellationToken>,
        ) -> Result<(), FilesError> {
            if self.fail {
                return Err(FilesError::Storage("down".into()));
            }
            self.calls
                .lock()
                .expect("lock")
                .push((path.to_owned(), body, options));
            Ok(())
        }
    }

    struct Fetcher(Box<dyn Fn() -> HttpResponse + Send + Sync>);

    #[async_trait]
    impl HttpFetcher for Fetcher {
        async fn fetch(&self, _url: &str, opts: FetchOptions) -> Result<HttpResponse, FetchError> {
            assert!(opts.follow_redirects);
            Ok((self.0)())
        }
    }

    fn response(
        status: u16,
        headers: &[(&'static str, &'static str)],
        body: &'static [u8],
    ) -> HttpResponse {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(*name, value.parse().expect("value"));
        }
        HttpResponse {
            status,
            headers: map,
            body: Body::Stream(futures::stream::iter(vec![Ok(Bytes::from_static(body))]).boxed()),
        }
    }

    fn deps(
        respond: impl Fn() -> HttpResponse + Send + Sync + 'static,
        putter: Arc<Putter>,
    ) -> UploadDeps {
        UploadDeps {
            fetcher: Arc::new(Fetcher(Box::new(respond))),
            putter,
        }
    }

    #[test]
    fn parse_int_mimics_js() {
        assert_eq!(parse_int("12abc"), Some(12));
        assert_eq!(parse_int("  -3"), Some(-3));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int(""), None);
    }

    #[test]
    fn name_from_url_decodes_the_last_segment() {
        assert_eq!(
            name_from_url("https://x.test/a/My%20File.pdf?x=1#f").as_deref(),
            Some("My File.pdf")
        );
        assert_eq!(name_from_url("https://x.test/a/"), None);
        assert_eq!(name_from_url("https://x.test"), None);
        assert_eq!(name_from_url("https://x.test/100%"), None);
        assert_eq!(name_from_url("not a url"), None);
    }

    #[tokio::test]
    async fn a_failed_status_is_a_validation_error() {
        let putter = Arc::new(Putter::default());
        let error = upload_file_from_url(
            &deps(|| response(404, &[], b""), putter.clone()),
            "https://x.test/f",
            "k",
            UploadFromUrlOptions::default(),
        )
        .await
        .expect_err("404");
        assert!(error.is_upload_validation());
        assert_eq!(error.to_string(), "Failed to download file: 404");
        assert!(putter.calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn a_large_content_length_is_rejected_before_the_body() {
        let error = upload_file_from_url(
            &deps(
                || response(200, &[("content-length", "11")], b"x"),
                Arc::new(Putter::default()),
            ),
            "https://x.test/f",
            "k",
            UploadFromUrlOptions {
                max_bytes: Some(10),
                ..UploadFromUrlOptions::default()
            },
        )
        .await
        .expect_err("too big");
        assert_eq!(
            error.to_string(),
            "File exceeds the maximum allowed size of 10 bytes"
        );
    }

    #[tokio::test]
    async fn defaults_name_type_and_a_broken_image_on_the_success_path() {
        let putter = Arc::new(Putter::default());
        let file = upload_file_from_url(
            &deps(
                || response(200, &[("content-type", "image/x-png")], b"nope"),
                putter.clone(),
            ),
            "https://x.test/dir/",
            "public/k",
            UploadFromUrlOptions::default(),
        )
        .await
        .expect("upload");
        // A failed probe forces the result (not the stored object) to a file.
        assert_eq!(file.file_type, FileType::File);
        assert_eq!(file.mime_type, DEFAULT_MIME_TYPE);
        assert!(
            file.name.parse::<u64>().is_ok(),
            "generated id: {}",
            file.name
        );
        {
            let calls = putter.calls.lock().expect("lock");
            assert_eq!(calls[0].2.content_type.as_deref(), Some("image/png"));
            assert_eq!(calls[0].2.acl, Some(DEFAULT_ACL));
        }

        let file = upload_file_from_url(
            &deps(
                || response(200, &[("content-type", "")], b"data"),
                Arc::new(Putter::default()),
            ),
            "https://x.test/report.csv",
            "k",
            UploadFromUrlOptions::default(),
        )
        .await
        .expect("upload");
        assert_eq!(file.mime_type, DEFAULT_MIME_TYPE);
        assert_eq!(file.name, "report.csv");
        assert_eq!(file.size, 4);
    }

    #[tokio::test]
    async fn storage_errors_are_not_validation_errors() {
        let putter = Arc::new(Putter {
            fail: true,
            ..Putter::default()
        });
        let error = upload_file_from_url(
            &deps(|| response(200, &[], b"x"), putter),
            "https://x.test/f",
            "k",
            UploadFromUrlOptions::default(),
        )
        .await
        .expect_err("down");
        assert!(!error.is_upload_validation());
    }

    #[tokio::test]
    async fn a_cancelled_token_stops_the_body_read() {
        let token = CancellationToken::new();
        token.cancel();
        let respond = || HttpResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Body::Stream(futures::stream::pending().boxed()),
        };
        let error = upload_file_from_url(
            &deps(respond, Arc::new(Putter::default())),
            "https://x.test/f",
            "k",
            UploadFromUrlOptions {
                cancel: Some(token),
                ..UploadFromUrlOptions::default()
            },
        )
        .await
        .expect_err("cancelled");
        assert!(matches!(error, FilesError::Cancelled));
    }

    #[tokio::test]
    async fn upload_file_keeps_an_empty_type_and_multiple_files_keep_order() {
        let putter = Putter::default();
        let empty = InputFile {
            name: "a.bin".into(),
            mime_type: String::new(),
            bytes: Bytes::from_static(b"abc"),
        };
        let file = upload_file(&putter, &empty, "p/a", Acl::Private)
            .await
            .expect("upload");
        assert_eq!(file.mime_type, "");
        assert_eq!(file.file_type, FileType::File);
        assert_eq!(file.size, 3);

        let files = vec![
            InputFile {
                name: "1.txt".into(),
                mime_type: "TEXT/plain".into(),
                bytes: Bytes::from_static(b"1"),
            },
            InputFile {
                name: "2.txt".into(),
                mime_type: "text/plain".into(),
                bytes: Bytes::from_static(b"22"),
            },
        ];
        let uploaded = upload_multiple_files(&putter, &files, "public/ws/1/", Acl::PublicRead)
            .await
            .expect("upload");
        assert_eq!(uploaded[0].name, "1.txt");
        assert_eq!(uploaded[0].mime_type, "text/plain");
        assert_eq!(uploaded[1].size, 2);
        assert!(uploaded[0].origin_path.starts_with("public/ws/1/"));
        assert!(!uploaded[0].origin_path.contains("//"));
        assert_ne!(uploaded[0].origin_path, uploaded[1].origin_path);
    }
}
