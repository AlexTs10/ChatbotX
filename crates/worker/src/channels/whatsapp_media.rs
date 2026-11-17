//! WhatsApp media: `handlers/message/incoming-media.ts` (`fetchMedia`: copy a
//! received file to our storage) and `outgoing-message/outbound-media.ts` +
//! `business/outbound-media/service.ts` (`prepareOutboundMedia`: re-host a
//! link whose host serves a type the Cloud API rejects).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use chatbotx_business::net::ssrf_guard::assert_public_url;
use chatbotx_core::utils::image_optimize::ImageOptimizeProfile;
use chatbotx_core::utils::image_optimize_settings::EncodeSlots;
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_core::utils::store_incoming_image::{
    IncomingImageUploader, PutObjectOptions, StoreIncomingImageInput, store_incoming_image,
    to_stored_media_fields,
};
use chatbotx_files::{
    Acl, ChannelMediaLocation, FilesError, PutOptions, ReqwestFetcher, UploadDeps,
    UploadFromUrlOptions, Uploader, get_channel_media_prefix, get_storage_prefix,
    upload_file_from_url,
};
use serde_json::Value;

use crate::channel::{Category, ChannelError, ChannelResult, IncomingAttachment};

const MB: u64 = 1024 * 1024;
/// WhatsApp's largest media (a document).
// ponytail: the original reads the whole body with no cap; 100 MB is the
// Cloud API document limit.
const MAX_INBOUND_BYTES: u64 = 100 * MB;
const MAX_REDIRECTS: u32 = 5;
const RESOLVED_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FAILED_TTL: Duration = Duration::from_secs(5 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// The file store and the tenant `storageUrl` (`ctx.uploader`,
/// `ctx.platform.storageUrl`).
#[derive(Clone)]
pub struct WhatsappMedia {
    pub files: Arc<Uploader>,
    pub storage_url: String,
}

/// `ctx.uploader` for `storeIncomingImage`.
struct FilesPutter<'a>(&'a Uploader);

impl IncomingImageUploader for FilesPutter<'_> {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutObjectOptions,
    ) -> Result<(), String> {
        let put = PutOptions {
            acl: options.acl.map(|_| Acl::PublicRead),
            content_type: Some(options.content_type),
            content_length: Some(options.content_length as u64),
            ..PutOptions::default()
        };
        self.0
            .put_object(path, body, put, None)
            .await
            .map_err(|e| e.to_string())
    }
}

fn media_error() -> ChannelError {
    ChannelError::new("Unable to fetch media info", Category::Unknown)
}

fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// GET `url` by hand: every hop is SSRF-checked, the token goes only to hops
/// on the first URL's origin, and the body stops at `max_bytes`.
async fn download(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    max_bytes: u64,
) -> Result<(Bytes, Option<String>), String> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        assert_public_url(&current, "WhatsApp media URL").await?;
        let mut request = client.get(&current).header("User-Agent", "node");
        if let Some(token) = token.filter(|_| same_origin(url, &current)) {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Redirect response has no Location header")?;
            current = url::Url::parse(&current)
                .and_then(|base| base.join(location))
                .map_err(|e| e.to_string())?
                .to_string();
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("media download answered {}", response.status()));
        }
        let declared = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut out = BytesMut::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if (out.len() + chunk.len()) as u64 > max_bytes {
                return Err(format!("media is larger than {max_bytes} bytes"));
            }
            out.extend_from_slice(&chunk);
        }
        return Ok((out.freeze(), declared));
    }
    Err("Too many redirects while fetching media".into())
}

/// The redirect-free client [`download`] needs.
pub fn media_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// `fetchMedia` + `readMediaReply`: `retrieveMedia(id)`, the bytes, then
/// `storeIncomingImage` under the channel media prefix. The declared MIME
/// type stays unless the bytes changed format. Any failure fails the job.
pub async fn store_inbound_media(
    media: &WhatsappMedia,
    client: &reqwest::Client,
    graph_url: &str,
    token: &str,
    loc: &ChannelMediaLocation,
    attachment: &mut IncomingAttachment,
) -> ChannelResult<()> {
    let Some(media_id) = attachment.source_id.clone() else {
        return Ok(());
    };
    let info: Value = async {
        let response = client
            .get(format!(
                "{graph_url}/{}/{media_id}",
                super::whatsapp::DEFAULT_API_VERSION
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let text = response.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())
    }
    .await
    .map_err(|error: String| {
        tracing::error!(%error, media_id, "Unable to fetch media info:");
        media_error()
    })?;
    let (Some(url), Some(mime_type)) = (info["url"].as_str(), info["mime_type"].as_str()) else {
        tracing::error!(media_id, mediaResponse = %info, "Unable to fetch media:");
        return Err(media_error().into());
    };
    copy_to_storage(
        media,
        client,
        url,
        Some(token),
        Some(mime_type),
        loc,
        attachment,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, media_id, "Unable to fetch media info:");
        media_error().into()
    })
}

/// The download + `storeIncomingImage` half of `fetchMedia`, shared by the
/// channels that copy a provider file URL (Telegram, TikTok, Zalo): the
/// bytes under the channel media prefix, the attachment's stored fields.
/// `bearer` goes only to the first URL's origin. `mime_type: None` takes
/// the response `content-type` (`image/png` when absent), as Zalo does.
pub async fn copy_to_storage(
    media: &WhatsappMedia,
    client: &reqwest::Client,
    url: &str,
    bearer: Option<&str>,
    mime_type: Option<&str>,
    loc: &ChannelMediaLocation,
    attachment: &mut IncomingAttachment,
) -> Result<(), String> {
    let (bytes, declared) = download(client, url, bearer, MAX_INBOUND_BYTES).await?;
    let mime_type = mime_type
        .map(str::to_owned)
        .or(declared)
        .unwrap_or_else(|| "image/png".into());
    let mime_type = mime_type.as_str();
    if attachment.mime_type.is_empty() {
        attachment.mime_type = mime_type.to_owned();
    }
    let key = format!(
        "{}/{}",
        get_channel_media_prefix(loc),
        chatbotx_core::utils::id::create_id()
    );
    let profile = if attachment.is_photo_message {
        ImageOptimizeProfile::Media
    } else {
        ImageOptimizeProfile::Original
    };
    let putter = FilesPutter(&media.files);
    let stored = store_incoming_image(
        StoreIncomingImageInput {
            uploader: Some(&putter),
            key: &key,
            bytes,
            mime_type,
            profile,
            acl: Some("public-read"),
        },
        &chatbotx_core::env::ProcessEnv,
        &EncodeSlots::global(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let fields = to_stored_media_fields(&stored);
    attachment.origin_path = fields.origin_path;
    attachment.size = Some(fields.size as i64);
    // The provider's own dimensions stand in when the bytes could not be read.
    if let Some(width) = fields.width {
        attachment.width = Some(i64::from(width));
    }
    if let Some(height) = fields.height {
        attachment.height = Some(i64::from(height));
    }
    if stored.optimized {
        attachment.mime_type = fields.mime_type;
    }
    Ok(())
}

// ---- outbound (`prepareOutboundMedia`) -------------------------------------

/// `WHATSAPP_MEDIA_RULES`: the types and size caps the Cloud API accepts.
fn media_rules(kind: &str) -> Option<(&'static [&'static str], u64)> {
    match kind {
        "image" => Some((&["image/jpeg", "image/png", "image/webp"], 5 * MB)),
        "video" => Some((&["video/mp4", "video/3gpp"], 16 * MB)),
        "audio" => Some((
            &[
                "audio/aac",
                "audio/amr",
                "audio/mpeg",
                "audio/mp4",
                "audio/ogg",
            ],
            16 * MB,
        )),
        _ => None,
    }
}

/// `collectMediaLinks`: every `{ link }` object under an `image` / `video` /
/// `audio` key, as a JSON pointer and its kind.
pub fn collect_media_links(value: &Value) -> Vec<(String, &'static str)> {
    fn visit(
        value: &Value,
        path: String,
        parent: Option<&str>,
        out: &mut Vec<(String, &'static str)>,
    ) {
        if value.get("link").is_some_and(Value::is_string)
            && let Some(kind) =
                parent.and_then(|p| ["image", "video", "audio"].into_iter().find(|k| *k == p))
        {
            out.push((path.clone(), kind));
        }
        match value {
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    visit(item, format!("{path}/{i}"), parent, out);
                }
            }
            Value::Object(map) => {
                for (key, child) in map {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    visit(child, format!("{path}/{escaped}"), Some(key), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    visit(value, String::new(), None, &mut out);
    out
}

/// `HTTP_URL_RE`.
fn is_http_url(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `isOnStorage`: on a path boundary of `storage_url`.
fn is_on_storage(url: &str, storage_url: &str) -> bool {
    if storage_url.is_empty() {
        return false;
    }
    let base = if storage_url.ends_with('/') {
        storage_url.to_owned()
    } else {
        format!("{storage_url}/")
    };
    url.starts_with(&base)
}

/// `servedBaseMimeType`: not normalized (`image/jpg` stays as served).
fn served_base_mime_type(content_type: Option<&str>) -> Option<String> {
    content_type.map(|c| c.split(';').next().unwrap_or("").trim().to_lowercase())
}

/// `distributedStore` entries of `outbound-media:v1:<sha>`.
// ponytail: an in-process cache (one server process); no single flight, so
// two concurrent first sends of one URL may both probe.
static CACHE: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_put(key: &str, url: &str, ttl: Duration) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, (_, expires)| *expires > Instant::now());
        cache.insert(key.to_owned(), (url.to_owned(), Instant::now() + ttl));
    }
}

fn cache_get(key: &str) -> Option<String> {
    let cache = CACHE.lock().ok()?;
    cache
        .get(key)
        .filter(|(_, expires)| *expires > Instant::now())
        .map(|(url, _)| url.clone())
}

fn sha256_hex(input: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, input.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `probeMimeType`: HEAD with SSRF-checked manual redirects.
async fn probe_mime_type(client: &reqwest::Client, url: &str) -> Result<Option<String>, String> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        assert_public_url(&current, "Outbound media URL")
            .await
            .map_err(|_| "The media URL is not allowed".to_owned())?;
        let response = client
            .head(&current)
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .filter(|l| !l.is_empty())
                .ok_or("Redirect response has no Location header")?;
            current = url::Url::parse(&current)
                .and_then(|base| base.join(location))
                .map_err(|_| format!("Invalid redirect location: {location}"))?
                .to_string();
            continue;
        }
        if !response.status().is_success() {
            return Ok(None);
        }
        return Ok(served_base_mime_type(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        ));
    }
    Err("Too many redirects while probing media".into())
}

/// `rehost`: the stored copy's public URL, or `None` when the file is not an
/// accepted type even after normalization.
async fn rehost(
    media: &WhatsappMedia,
    url: &str,
    path: &str,
    allowed: &[&str],
    max_bytes: u64,
) -> Result<Option<String>, String> {
    let deps = UploadDeps {
        fetcher: Arc::new(ReqwestFetcher::new().map_err(|e| e.to_string())?),
        putter: Arc::clone(&media.files) as Arc<dyn chatbotx_files::ObjectPutter>,
    };
    let validate: chatbotx_files::ValidateUrl = Arc::new(|candidate: String| {
        Box::pin(async move {
            assert_public_url(&candidate, "Outbound media URL")
                .await
                .map_err(|_| FilesError::UploadValidation("The media URL is not allowed".into()))
        })
    });
    let upload = upload_file_from_url(
        &deps,
        url,
        path,
        UploadFromUrlOptions {
            acl: Acl::PublicRead,
            max_bytes: Some(max_bytes),
            validate_url: Some(validate),
            cancel: None,
        },
    );
    let uploaded = tokio::time::timeout(DOWNLOAD_TIMEOUT, upload)
        .await
        .map_err(|_| "The operation was aborted".to_owned())?
        .map_err(|e| e.to_string())?;
    if !allowed.contains(&uploaded.mime_type.as_str()) {
        return Ok(None);
    }
    get_public_file_url(path, &media.storage_url)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// `resolveOutboundMediaUrl`: never fails; on any error the link is kept.
async fn resolve_outbound_media_url(
    media: &WhatsappMedia,
    client: &reqwest::Client,
    url: &str,
    kind: &str,
    storage_prefix: &str,
) -> String {
    let Some((allowed, max_bytes)) = media_rules(kind) else {
        return url.to_owned();
    };
    if !is_http_url(url) || is_on_storage(url, &media.storage_url) {
        return url.to_owned();
    }
    let key = sha256_hex(&format!("{storage_prefix}\n{}\n{url}", allowed.join(",")));
    let cache_key = format!("outbound-media:v1:{key}");
    if let Some(cached) = cache_get(&cache_key) {
        return cached;
    }
    let resolved = async {
        let served = probe_mime_type(client, url).await?;
        if served.as_deref().is_some_and(|s| allowed.contains(&s)) {
            return Ok(url.to_owned());
        }
        let path = format!("{storage_prefix}/outbound-media/{key}");
        let rehosted = rehost(media, url, &path, allowed, max_bytes).await?;
        if rehosted.is_some() {
            tracing::info!(
                servedType = ?served,
                path,
                "Re-hosted outbound media served with an unsupported content type"
            );
        }
        Ok::<_, String>(rehosted.unwrap_or_else(|| url.to_owned()))
    }
    .await;
    match resolved {
        Ok(resolved) => {
            cache_put(&cache_key, &resolved, RESOLVED_TTL);
            resolved
        }
        Err(error) => {
            tracing::warn!(%error, "Unable to verify outbound media URL");
            cache_put(&cache_key, url, FAILED_TTL);
            url.to_owned()
        }
    }
}

/// `prepareOutboundMedia`: rewrites each media link of `body` in place.
pub async fn prepare_outbound_media(
    media: &WhatsappMedia,
    client: &reqwest::Client,
    workspace_id: i64,
    body: &mut Value,
) {
    if media.storage_url.is_empty() {
        return;
    }
    let storage_prefix = get_storage_prefix(&workspace_id.to_string());
    for (pointer, kind) in collect_media_links(body) {
        let Some(link) = body
            .pointer(&format!("{pointer}/link"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let resolved =
            resolve_outbound_media_url(media, client, &link, kind, &storage_prefix).await;
        if let Some(slot) = body.pointer_mut(&format!("{pointer}/link")) {
            *slot = Value::String(resolved);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn collects_links_under_media_keys() {
        let body = json!({
            "type": "image", "image": { "link": "https://a/x.jpg" },
            "interactive": { "header": { "type": "video", "video": { "link": "https://a/v.mp4" } } },
            "template": { "components": [{ "parameters": [{ "type": "document", "document": { "link": "https://a/d.pdf" } }] }] },
        });
        let mut links = collect_media_links(&body);
        links.sort();
        assert_eq!(
            links,
            vec![
                ("/image".to_owned(), "image"),
                ("/interactive/header/video".to_owned(), "video"),
            ]
        );
    }

    #[test]
    fn storage_and_mime_helpers() {
        assert!(is_on_storage("https://s/bucket/x.jpg", "https://s/bucket"));
        assert!(!is_on_storage(
            "https://s/bucket-public/x.jpg",
            "https://s/bucket"
        ));
        assert!(!is_on_storage("https://s/x.jpg", ""));
        assert_eq!(
            served_base_mime_type(Some("Image/JPG; charset=x")).as_deref(),
            Some("image/jpg")
        );
        assert!(is_http_url("HTTPS://x") && !is_http_url("ftp://x"));
        assert!(
            same_origin("https://a/x", "https://a/y") && !same_origin("https://a/x", "https://b/x")
        );
    }
}
