//! `incoming-flow-media.ts` + `flow-media-download.ts`: the files a customer
//! attached to a WhatsApp Flow answer (PhotoPicker / DocumentPicker). Meta puts
//! only `{ id, file_name, mime_type, sha256 }` in `response_json`, so each file
//! is fetched at once by its media id and stored like any inbound media. The
//! entity gets `files` (one `FlowResponseFile` per answer key) and every stored
//! file becomes an attachment.
//!
//! ponytail: the files of one response download one after another (the
//! original runs 3 side by side); add a JoinSet when big multi-file forms
//! miss the 45 s deadline.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use chatbotx_core::utils::image_optimize::ImageOptimizeProfile;
use chatbotx_core::utils::sanitize_file_name::sanitize_file_name;
use chatbotx_core::utils::store_incoming_image::to_stored_media_fields;
use chatbotx_files::{ChannelMediaLocation, get_channel_media_prefix};
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;
use tokio::time::Instant;

use super::whatsapp_media::{DownloadError, WhatsappMedia, download_checked, store_bytes};
use crate::channel::IncomingAttachment;

const MIB: u64 = 1024 * 1024;
const MAX_FILES: usize = 10;
const MAX_TOTAL_BYTES: u64 = 100 * MIB;
const MAX_FILE_BYTES: u64 = 25 * MIB;
const MAX_ATTEMPTS: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_millis(500);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const DEADLINE: Duration = Duration::from_secs(45);
const MAX_DISPLAY_NAME_LENGTH: usize = 255;

/// `MAX_FILES_IN_FLIGHT_PER_PROCESS`: files held in memory at once.
static FILE_SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(3));

const ALLOWED_MIME_TYPES: &[&str] = &[
    "application/gzip",
    "application/msword",
    "application/pdf",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/vnd.oasis.opendocument.presentation",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/x-7z-compressed",
    "application/zip",
    "image/avif",
    "image/gif",
    "image/heic",
    "image/heif",
    "image/jpeg",
    "image/png",
    "image/tiff",
    "image/webp",
    "text/plain",
    "video/mp4",
    "video/mpeg",
];

/// `FlowResponseFileFailureReason` (or a retry hint for a transient answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    TooLarge,
    DownloadFailed,
    UnsupportedType,
    /// `TransientFlowMediaError`: worth another attempt.
    Transient,
}

impl Failure {
    fn reason(self) -> &'static str {
        match self {
            Failure::TooLarge => "too_large",
            Failure::UnsupportedType => "unsupported_type",
            Failure::DownloadFailed | Failure::Transient => "download_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: String,
    file_name: String,
    mime_type: String,
}

fn normalize_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

fn is_allowed_mime_type(mime_type: &str) -> bool {
    ALLOWED_MIME_TYPES.contains(&normalize_mime_type(mime_type).as_str())
}

/// `toMediaId`: a string, or a JSON number that is a safe integer.
fn media_id(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => n
            .as_i64()
            .filter(|v| v.unsigned_abs() < (1u64 << 53))
            .map(|v| v.to_string()),
        _ => None,
    }
}

fn entry_of(value: &Value) -> Option<Entry> {
    let text = |key: &str| {
        value[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some(Entry {
        id: media_id(&value["id"])?,
        file_name: text("file_name")?,
        mime_type: text("mime_type")?,
    })
}

/// `extractFlowMediaGroups`: the answers that are non-empty lists of files
/// (every item has the file shape).
fn media_groups(flow_response: &Value) -> Vec<(String, Vec<Entry>)> {
    let Some(map) = flow_response.as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let items = value.as_array().filter(|a| !a.is_empty())?;
            let entries: Option<Vec<Entry>> = items.iter().map(entry_of).collect();
            Some((key.clone(), entries?))
        })
        .collect()
}

/// `toDisplayName`: no path parts, controls or bidi overrides.
fn display_name(file_name: &str) -> String {
    let hidden = |c: char| {
        let cp = c as u32;
        matches!(c, '/' | '\\')
            || cp <= 0x1f
            || (0x7f..=0x9f).contains(&cp)
            || (0x200e..=0x200f).contains(&cp)
            || (0x202a..=0x202e).contains(&cp)
            || (0x2066..=0x2069).contains(&cp)
    };
    let cleaned: String = file_name.chars().filter(|c| !hidden(*c)).collect();
    let cleaned: String = cleaned
        .trim()
        .chars()
        .take(MAX_DISPLAY_NAME_LENGTH)
        .collect();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned
    }
}

fn file_type(mime_type: &str) -> &'static str {
    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("video/") {
        "video"
    } else {
        "file"
    }
}

fn failed(entry: &Entry, failure: Failure) -> Value {
    json!({
        "status": "failed",
        "mediaId": entry.id,
        "fileName": display_name(&entry.file_name),
        "mimeType": entry.mime_type,
        "reason": failure.reason(),
    })
}

fn remaining(deadline: Instant) -> Result<Duration, Failure> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(Failure::DownloadFailed)
    } else {
        Ok(left)
    }
}

/// `isRetryableHttpStatus` (+ a CDN 404: a fresh URL may work).
fn is_transient(status: u16, cdn: bool) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500 || (cdn && status == 404)
}

/// The Graph and CDN access of one response.
pub(crate) struct Request<'a> {
    pub client: &'a reqwest::Client,
    pub graph_url: &'a str,
    pub token: &'a str,
    pub phone_number_id: &'a str,
}

/// `fetchMediaUrl` + the CDN download (`downloadOnce`).
async fn download_once(
    request: &Request<'_>,
    entry: &Entry,
    max_bytes: u64,
    deadline: Instant,
) -> Result<(bytes::Bytes, String), Failure> {
    let timeout = REQUEST_TIMEOUT.min(remaining(deadline)?);
    let mut url = url::Url::parse(&format!(
        "{}/{}/",
        request.graph_url,
        super::whatsapp::DEFAULT_API_VERSION
    ))
    .map_err(|_| Failure::DownloadFailed)?;
    url.path_segments_mut()
        .map_err(|()| Failure::DownloadFailed)?
        .pop_if_empty()
        .push(&entry.id);
    url.query_pairs_mut()
        .append_pair("phone_number_id", request.phone_number_id);
    let response = request
        .client
        .get(url.as_str())
        .bearer_auth(request.token)
        .header("User-Agent", "node")
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| Failure::Transient)?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(if is_transient(status, false) {
            Failure::Transient
        } else {
            Failure::DownloadFailed
        });
    }
    let body = response.bytes().await.map_err(|_| Failure::Transient)?;
    let info: Value = serde_json::from_slice(&body).map_err(|_| Failure::DownloadFailed)?;
    let (Some(url), Some(mime_type)) = (info["url"].as_str(), info["mime_type"].as_str()) else {
        return Err(Failure::DownloadFailed);
    };
    if !is_allowed_mime_type(mime_type) {
        return Err(Failure::UnsupportedType);
    }
    let timeout = REQUEST_TIMEOUT.min(remaining(deadline)?);
    let downloaded = tokio::time::timeout(
        timeout,
        download_checked(
            request.client,
            url,
            Some(request.token),
            max_bytes,
            "WhatsApp Flow media URL",
        ),
    )
    .await
    .map_err(|_| Failure::Transient)?;
    match downloaded {
        Ok((bytes, _)) => Ok((bytes, normalize_mime_type(mime_type))),
        Err(DownloadError::TooLarge) => Err(Failure::TooLarge),
        Err(DownloadError::Status(status)) if is_transient(status.as_u16(), true) => {
            Err(Failure::Transient)
        }
        Err(DownloadError::Other(_)) => Err(Failure::Transient),
        Err(DownloadError::Status(_) | DownloadError::Unsafe(_)) => Err(Failure::DownloadFailed),
    }
}

/// `downloadFlowMedia`: a transient failure is tried once more.
async fn download(
    request: &Request<'_>,
    entry: &Entry,
    max_bytes: u64,
    deadline: Instant,
) -> Result<(bytes::Bytes, String), Failure> {
    let mut attempt = 1;
    loop {
        match download_once(request, entry, max_bytes, deadline).await {
            Err(Failure::Transient)
                if attempt < MAX_ATTEMPTS
                    && RETRY_DELAY < deadline.saturating_duration_since(Instant::now()) =>
            {
                tokio::time::sleep(RETRY_DELAY).await;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// `keepEntry`: download within the byte budget, then store it.
async fn keep(
    media: &WhatsappMedia,
    loc: &ChannelMediaLocation,
    request: &Request<'_>,
    entry: &Entry,
    budget: &mut u64,
    deadline: Instant,
) -> Result<(Value, IncomingAttachment), Failure> {
    let limit = MAX_FILE_BYTES.min(*budget);
    if limit == 0 {
        return Err(Failure::TooLarge);
    }
    let (bytes, mime_type) = download(request, entry, limit, deadline).await?;
    let size = bytes.len() as u64;
    if size > *budget {
        return Err(Failure::TooLarge);
    }
    *budget -= size;
    let key = format!(
        "{}/{}-{}",
        get_channel_media_prefix(loc),
        chatbotx_core::utils::id::create_id(),
        sanitize_file_name(&entry.file_name)
    );
    let stored = tokio::time::timeout(
        remaining(deadline)?,
        store_bytes(
            media,
            &key,
            bytes,
            &mime_type,
            ImageOptimizeProfile::Original,
        ),
    )
    .await
    .map_err(|_| Failure::DownloadFailed)?
    .map_err(|_| Failure::DownloadFailed)?;
    let fields = to_stored_media_fields(&stored);
    let name = display_name(&entry.file_name);
    let file = json!({
        "status": "stored",
        "mediaId": entry.id,
        "fileName": name,
        "mimeType": fields.mime_type,
        "size": fields.size,
    });
    let attachment = IncomingAttachment {
        source_id: Some(entry.id.clone()),
        file_type: file_type(&fields.mime_type).into(),
        mime_type: fields.mime_type.clone(),
        origin_path: fields.origin_path,
        size: Some(fields.size as i64),
        width: fields.width.map(i64::from),
        height: fields.height.map(i64::from),
        name: Some(name),
        is_photo_message: false,
        optimized: stored.optimized,
    };
    Ok((file, attachment))
}

/// `ingestEntry`: never fails; a file that cannot be kept is a `failed` entry.
async fn ingest(
    media: Option<&WhatsappMedia>,
    loc: &ChannelMediaLocation,
    request: &Request<'_>,
    entry: &Entry,
    budget: &mut u64,
    deadline: Instant,
) -> (Value, Option<IncomingAttachment>) {
    if !is_allowed_mime_type(&entry.mime_type) {
        return (failed(entry, Failure::UnsupportedType), None);
    }
    let Some(media) = media else {
        return (failed(entry, Failure::DownloadFailed), None);
    };
    let wait = deadline.saturating_duration_since(Instant::now());
    let Ok(Ok(_slot)) = tokio::time::timeout(wait, FILE_SLOTS.acquire()).await else {
        return (failed(entry, Failure::DownloadFailed), None);
    };
    match keep(media, loc, request, entry, budget, deadline).await {
        Ok((file, attachment)) => (file, Some(attachment)),
        Err(failure) => {
            tracing::warn!(
                media_id = entry.id,
                reason = failure.reason(),
                "Unable to keep WhatsApp Flow media"
            );
            (failed(entry, failure), None)
        }
    }
}

/// `readFlowResponseMedia`: sets `files` on a Flow response entity and
/// returns the stored files' attachments. Nothing happens for another entity
/// or a response without files. `phone_number_id: None` = nothing is fetched.
pub(crate) async fn read_flow_response_media(
    media: Option<&WhatsappMedia>,
    loc: &ChannelMediaLocation,
    request: &Request<'_>,
    entity: &mut Value,
) -> Vec<IncomingAttachment> {
    if entity["type"] != "whatsapp_flow_response" {
        return Vec::new();
    }
    let groups = media_groups(&entity["flowResponse"]);
    if groups.is_empty() {
        return Vec::new();
    }
    // Each media id once, first occurrence wins.
    let mut entries: Vec<&Entry> = Vec::new();
    for entry in groups.iter().flat_map(|(_, e)| e) {
        if !entries.iter().any(|known| known.id == entry.id) {
            entries.push(entry);
        }
    }
    let deadline = Instant::now() + DEADLINE;
    let mut budget = MAX_TOTAL_BYTES;
    let mut outcomes: HashMap<String, Value> = HashMap::new();
    let mut attachments = Vec::new();
    if request.phone_number_id.is_empty() {
        tracing::warn!(
            files = entries.len().min(MAX_FILES),
            "WhatsApp Flow files not downloaded: the receiving phone number is unknown"
        );
    }
    for (index, entry) in entries.iter().enumerate() {
        let file = if index >= MAX_FILES {
            failed(entry, Failure::TooLarge)
        } else if request.phone_number_id.is_empty() {
            failed(entry, Failure::DownloadFailed)
        } else {
            let (file, attachment) =
                ingest(media, loc, request, entry, &mut budget, deadline).await;
            attachments.extend(attachment);
            file
        };
        outcomes.insert(entry.id.clone(), file);
    }
    let files: Map<String, Value> = groups
        .iter()
        .map(|(key, group)| {
            let list = group
                .iter()
                .filter_map(|e| outcomes.get(&e.id).cloned())
                .collect();
            (key.clone(), Value::Array(list))
        })
        .collect();
    entity["files"] = Value::Object(files);
    attachments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_need_every_item_to_be_a_file() {
        let response = json!({
            "photo": [{ "id": 123, "file_name": "a.png", "mime_type": "image/png" }],
            "colors": ["red", "blue"],
            "empty": [],
            "mixed": [{ "id": "9", "file_name": "b", "mime_type": "text/plain" }, "x"],
            "flow_token": "t",
        });
        let groups = media_groups(&response);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "photo");
        assert_eq!(groups[0].1[0].id, "123");
        assert!(media_id(&json!(9_007_199_254_740_993_u64)).is_none());
    }

    #[test]
    fn display_names_and_mime_rules() {
        assert_eq!(display_name("../x/\u{202e}evil.pdf "), "..xevil.pdf");
        assert_eq!(display_name("\u{0007}"), "file");
        assert!(is_allowed_mime_type("Image/JPEG; charset=x"));
        assert!(!is_allowed_mime_type("application/x-msdownload"));
        assert_eq!(file_type("video/mp4"), "video");
        assert!(is_transient(404, true) && !is_transient(404, false));
    }

    #[tokio::test]
    async fn without_a_phone_number_every_file_fails() {
        let client = reqwest::Client::new();
        let request = Request {
            client: &client,
            graph_url: "http://127.0.0.1:9",
            token: "t",
            phone_number_id: "",
        };
        let loc = ChannelMediaLocation {
            channel: "whatsapp".into(),
            workspace_id: "1".into(),
            integration_id: "2".into(),
        };
        let mut entity = json!({
            "type": "whatsapp_flow_response",
            "flowResponse": {
                "docs": [
                    { "id": "1", "file_name": "a.pdf", "mime_type": "application/pdf" },
                    { "id": "2", "file_name": "b.exe", "mime_type": "application/x-msdownload" }
                ]
            }
        });
        let attachments = read_flow_response_media(None, &loc, &request, &mut entity).await;
        assert!(attachments.is_empty());
        assert_eq!(entity["files"]["docs"][0]["reason"], "download_failed");
        assert_eq!(entity["files"]["docs"][1]["status"], "failed");
    }
}
