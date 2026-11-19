//! `coexistAttachmentDownload` (low queue; tests port
//! `coexist-attachment-download.test.ts` and the Graph cases of
//! `media-hydration.test.ts`): copy the bytes of a history-sync
//! attachment (`wa-media:{id}` stub) to our storage (`coexist/
//! attachment-download.ts`, `enqueue-attachment-downloads.ts`, the
//! `ensureAttachmentMirrored` / `markAttachmentUnresolvable` half of
//! `channel-registry/media-hydration.ts`).
//!
//! Strategies: WhatsApp (`wa-media:{id}` through `retrieveMedia`) and the
//! Graph channels Messenger / Instagram (`getMessageMediaUrls` on the
//! message's Graph id; a live webhook row first tries its stored Meta URL).
//! `restore` (`restoreMirroredAttachment`) re-derives an evicted object of a
//! Graph channel under its existing key; WhatsApp cannot (no fresh URL).
//!
//! ponytail: the per-message lock is in-process (one worker process).
use std::sync::LazyLock;

use bytes::Bytes;
use chatbotx_core::env::ProcessEnv;
use chatbotx_core::utils::image_optimize::{
    ImageOptimizeProfile, optimize_image_for_storage, read_image_dimensions,
};
use chatbotx_core::utils::image_optimize_settings::{
    EncodeSlots, get_image_optimization_settings, is_stored_image_optimization_enabled,
};
use chatbotx_core::utils::live_media::{META_MEDIA_HOSTS, is_url_on_hosts};
use chatbotx_db::repo::contact_inbox;
use chatbotx_db::repo::message::{
    self, FindAttachmentByIdParams, FindMessageByIdParams, UpdateAttachmentFields,
    UpdateAttachmentParams,
};
use chatbotx_db::repo::models::{Attachment, ContactInbox};
use chatbotx_files::{
    Acl, ChannelMediaLocation, PutOptions, get_channel_media_prefix,
    get_mirrored_channel_media_prefix, guess_file_type_from_mime_type,
};
use chatbotx_jobs::options::{Backoff, Retention};
use chatbotx_jobs::{JobCtx, JobError, JobOptions, QueueName};
use chatbotx_kv::lock::{LockMap, LockOpts};
use serde_json::{Value, json};

use crate::channels::whatsapp::{API_URL, DEFAULT_API_VERSION};
use crate::channels::whatsapp_media::{DownloadError, download_checked, media_client};
use crate::{Deps, db_fail, fail, id_of, str_of};

pub const JOB: &str = "coexistAttachmentDownload";
/// `LOW_BULK_PRIORITY`.
const LOW_BULK_PRIORITY: u32 = 10;
/// `MAX_ATTACHMENT_BYTES`.
pub(crate) const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
const LOCK_TTL_SECS: u64 = 5 * 60;
const WA_MEDIA_PREFIX: &str = "wa-media:";
const FAILED_PREFIX: &str = "failed:";
/// `MESSAGE_MEDIA_FIELDS` of the Messenger / Instagram `apis/sync.ts`.
const MESSENGER_MEDIA_FIELDS: &str =
    "attachments{id,name,mime_type,size,image_data,video_data,file_url}";
const INSTAGRAM_MEDIA_FIELDS: &str =
    "attachments{id,name,mime_type,size,payload,image_data,video_data,file_url}";
/// Native Instagram Login reads `graph.instagram.com`.
const INSTAGRAM_API_URL: &str = "https://graph.instagram.com";

static MEDIA_LOCK: LazyLock<LockMap> = LazyLock::new(LockMap::new);

/// `TerminalMediaError` (logged, the job succeeds) or a retryable failure.
#[derive(Debug)]
enum MediaError {
    Terminal(String),
    Retry(String),
}

type MediaResult<T> = Result<T, MediaError>;

fn retry(message: impl ToString) -> MediaError {
    MediaError::Retry(message.to_string())
}

/// `isPendingOriginPath`.
fn is_pending(origin_path: &str) -> bool {
    origin_path.starts_with("http://")
        || origin_path.starts_with("https://")
        || origin_path.starts_with(WA_MEDIA_PREFIX)
}

/// `MIME_EXTENSION_MAP`.
fn mime_extension(mime_type: &str) -> Option<&'static str> {
    Some(match mime_type {
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "image/bmp" => "bmp",
        "image/gif" => "gif",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        "text/plain" => "txt",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        _ => return None,
    })
}

/// `enqueueAttachmentDownloadJobs`: one low-queue job per attachment id
/// (`att-<id>` dedups re-enqueues). Does not catch; callers pick the policy.
pub async fn enqueue(
    deps: &Deps,
    workspace_id: i64,
    integration_id: i64,
    channel: &str,
    attachment_ids: &[i64],
) -> Result<(), JobError> {
    for id in attachment_ids {
        crate::inbound::add_job(
            deps,
            QueueName::Low,
            JOB,
            json!({ "type": JOB, "data": {
                "attachmentId": id.to_string(),
                "workspaceId": workspace_id.to_string(),
                "channel": channel,
                "integrationId": integration_id.to_string(),
            }}),
            job_options(*id),
        )
        .await?;
    }
    Ok(())
}

fn job_options(attachment_id: i64) -> JobOptions {
    JobOptions {
        job_id: Some(format!("att-{attachment_id}")),
        priority: Some(LOW_BULK_PRIORITY),
        attempts: Some(5),
        backoff: Some(Backoff::Exponential(30_000)),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::KeepCount(100)),
        ..JobOptions::default()
    }
}

/// The `coexistAttachmentDownload` processor.
pub async fn handle(deps: &Deps, job: &JobCtx) -> Result<(), JobError> {
    handle_with(deps, job, &Bases::live()).await
}

/// The Graph hosts (one local server in tests).
struct Bases {
    facebook: String,
    instagram: String,
}

impl Bases {
    fn live() -> Self {
        Self {
            facebook: API_URL.into(),
            instagram: INSTAGRAM_API_URL.into(),
        }
    }
}

async fn handle_with(deps: &Deps, job: &JobCtx, bases: &Bases) -> Result<(), JobError> {
    let data = &job.data["data"];
    let channel = str_of(data, "channel").unwrap_or_default();
    let (Some(attachment_id), Some(workspace_id)) =
        (id_of(&data["attachmentId"]), id_of(&data["workspaceId"]))
    else {
        return Err(JobError::unrecoverable(
            "Invalid coexistAttachmentDownload data",
        ));
    };
    if data["restore"].as_bool() == Some(true) {
        // A failed restore never marks the row: the next view retries.
        return match restore(deps, workspace_id, attachment_id, bases).await {
            Ok(restored) => {
                tracing::debug!(
                    attachment_id,
                    channel,
                    restored,
                    "[coexist-attachment] restore done"
                );
                Ok(())
            }
            Err(MediaError::Terminal(error)) => {
                tracing::warn!(%error, attachment_id, channel, "[coexist-attachment] terminal media failure — skipping restore");
                Ok(())
            }
            Err(MediaError::Retry(error)) => {
                tracing::error!(%error, attachment_id, channel, "[coexist-attachment] restore failed");
                Err(fail(error))
            }
        };
    }
    match ensure_mirrored(deps, workspace_id, attachment_id, bases).await {
        Ok(()) => Ok(()),
        Err(MediaError::Terminal(error)) => {
            tracing::warn!(%error, attachment_id, channel, "[coexist-attachment] terminal media failure — skipping");
            Ok(())
        }
        Err(MediaError::Retry(error)) => {
            if job.is_final_attempt()
                && let Err(mark) = mark_unresolvable(deps, workspace_id, attachment_id).await
            {
                tracing::error!(error = %mark, attachment_id, channel, "[coexist-attachment] failed to mark unresolvable");
            }
            tracing::error!(%error, attachment_id, channel, "[coexist-attachment] hydration failed");
            Err(fail(error))
        }
    }
}

async fn find_attachment(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
) -> Result<Option<message::AttachmentLookupRow>, JobError> {
    chatbotx_db::repo::read(&deps.db, move |c| {
        message::find_attachment_by_id(
            c,
            &FindAttachmentByIdParams {
                id,
                message_created_at: None,
                workspace_id,
            },
        )
    })
    .await
    .map_err(db_fail)
}

/// The attachment's message attachments, contact inbox and Graph message id.
type AttachmentGraph = (Vec<Attachment>, ContactInbox, Option<String>);

async fn load_graph(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
) -> Result<Option<AttachmentGraph>, JobError> {
    chatbotx_db::repo::read(&deps.db, move |c| {
        let Some(row) = message::find_attachment_by_id(
            c,
            &FindAttachmentByIdParams {
                id,
                message_created_at: None,
                workspace_id,
            },
        )?
        else {
            return Ok(None);
        };
        let Some(found) = message::find_by_id(
            c,
            &FindMessageByIdParams {
                conversation_id: None,
                created_at: row.message_created_at,
                id: row.message_id,
                workspace_id,
            },
        )?
        else {
            return Ok(None);
        };
        let inbox = contact_inbox::find_model_by_id_for_workspace(
            c,
            found.message.contact_inbox_id,
            workspace_id,
        )?;
        Ok(inbox.map(|inbox| (found.attachments, inbox, found.message.source_id)))
    })
    .await
    .map_err(db_fail)
}

async fn update(
    deps: &Deps,
    workspace_id: i64,
    attachment: &Attachment,
    fields: UpdateAttachmentFields,
) -> MediaResult<()> {
    let params = UpdateAttachmentParams {
        created_at: attachment.created_at,
        fields,
        id: attachment.id,
        workspace_id,
    };
    let updated =
        chatbotx_db::repo::write(&deps.db, move |tx| message::update_attachment(tx, &params))
            .await
            .map_err(retry)?;
    // `updateAttachmentOrThrow`.
    if updated == 0 {
        return Err(retry(format!(
            "Attachment {} was not updated",
            attachment.id
        )));
    }
    Ok(())
}

/// `markAttachmentFailed`: `failed:<reason>`, the rest unchanged.
async fn mark_failed(
    deps: &Deps,
    workspace_id: i64,
    attachment: &Attachment,
    reason: &str,
) -> MediaResult<()> {
    update(
        deps,
        workspace_id,
        attachment,
        UpdateAttachmentFields {
            height: attachment.height,
            mime_type: attachment.mime_type.clone(),
            origin_path: format!("{FAILED_PREFIX}{reason}"),
            size: attachment.size,
            width: attachment.width,
        },
    )
    .await
}

/// `markAttachmentUnresolvable`: only a still-pending row.
async fn mark_unresolvable(deps: &Deps, workspace_id: i64, id: i64) -> Result<(), String> {
    let graph = load_graph(deps, workspace_id, id)
        .await
        .map_err(|e| e.to_string())?;
    let Some(attachment) = graph
        .and_then(|(attachments, _, _)| attachments.into_iter().find(|a| a.id == id))
        .filter(|a| is_pending(&a.origin_path))
    else {
        return Ok(());
    };
    mark_failed(deps, workspace_id, &attachment, "unresolvable")
        .await
        .map_err(|e| format!("{e:?}"))
}

/// `ensureAttachmentMirrored` with siblings: the requested row decides the
/// outcome; the message's other pending rows ride along, best effort.
async fn ensure_mirrored(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
    bases: &Bases,
) -> MediaResult<()> {
    let not_found = || MediaError::Terminal(format!("Attachment {id} was not found"));
    let initial = find_attachment(deps, workspace_id, id)
        .await
        .map_err(retry)?
        .ok_or_else(not_found)?;
    if initial.origin_path.starts_with(FAILED_PREFIX) {
        return Err(MediaError::Terminal(format!(
            "Attachment {id} is permanently unavailable"
        )));
    }
    if !is_pending(&initial.origin_path) {
        return Ok(());
    }
    let key = format!("media-hydration:{workspace_id}:{}", initial.message_id);
    let _guard = MEDIA_LOCK
        .acquire(
            &key,
            LockOpts {
                ttl_secs: LOCK_TTL_SECS,
                retry_secs: None,
            },
        )
        .await
        .map_err(retry)?;

    let (mut attachments, contact_inbox, message_source_id) = load_graph(deps, workspace_id, id)
        .await
        .map_err(retry)?
        .ok_or_else(not_found)?;
    let requested = attachments
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(not_found)?
        .clone();
    if requested.origin_path.starts_with(FAILED_PREFIX) {
        return Err(MediaError::Terminal(format!(
            "Attachment {id} is permanently unavailable"
        )));
    }
    if !is_pending(&requested.origin_path) {
        return Ok(());
    }
    let graph_channel = is_graph_channel(&contact_inbox.channel);
    if contact_inbox.channel != "whatsapp" && !graph_channel {
        // A channel without media hydration stores the provider's own URL.
        if requested.origin_path.starts_with("http://")
            || requested.origin_path.starts_with("https://")
        {
            return Ok(());
        }
        return Err(MediaError::Terminal(format!(
            "No media hydration strategy for attachment {id}"
        )));
    }
    let mirror = Mirror::resolve(deps, workspace_id, &contact_inbox, bases).await?;
    if graph_channel && message_source_id.is_none() {
        mark_failed(deps, workspace_id, &requested, "unresolvable").await?;
        return Err(MediaError::Terminal(format!(
            "Message source id is missing for attachment {id}"
        )));
    }
    // `compareAttachmentIds`; the requested row goes first.
    attachments.sort_by_key(|a| a.id);
    let count = attachments.len();
    let mut ordered: Vec<(usize, &Attachment)> = attachments.iter().enumerate().collect();
    ordered.sort_by_key(|(_, a)| a.id != id);
    let mut fresh: Option<Vec<Fresh>> = None;
    for (index, attachment) in ordered {
        if !is_pending(&attachment.origin_path) {
            continue;
        }
        let is_requested = attachment.id == id;
        let outcome = match message_source_id.as_deref() {
            Some(message_id) if graph_channel => {
                mirror
                    .graph_pending(
                        attachment,
                        index,
                        count,
                        is_requested,
                        message_id,
                        &mut fresh,
                    )
                    .await
            }
            _ => mirror.whatsapp(attachment).await,
        };
        match outcome {
            Ok(()) => {}
            Err(err) if is_requested => return Err(err),
            Err(MediaError::Terminal(_)) => {}
            Err(MediaError::Retry(error)) => {
                tracing::warn!(%error, attachment_id = attachment.id, "media-hydration: sibling attachment not mirrored; left pending");
            }
        }
    }
    Ok(())
}

/// `restoreMirroredAttachment`: the bytes of a mirrored row whose object was
/// evicted, re-derived from the channel and written under the same key.
/// `Ok(false)` = nothing to restore (WhatsApp, not mirrored, no match).
async fn restore(deps: &Deps, workspace_id: i64, id: i64, bases: &Bases) -> MediaResult<bool> {
    let Some(initial) = find_attachment(deps, workspace_id, id)
        .await
        .map_err(retry)?
    else {
        return Ok(false);
    };
    if !is_mirrored(&initial.origin_path) {
        return Ok(false);
    }
    let key = format!("media-hydration:{workspace_id}:{}", initial.message_id);
    let _guard = MEDIA_LOCK
        .acquire(
            &key,
            LockOpts {
                ttl_secs: LOCK_TTL_SECS,
                retry_secs: None,
            },
        )
        .await
        .map_err(retry)?;
    let Some((mut attachments, contact_inbox, message_source_id)) =
        load_graph(deps, workspace_id, id).await.map_err(retry)?
    else {
        return Ok(false);
    };
    if !is_graph_channel(&contact_inbox.channel) {
        return Ok(false);
    }
    let mirror = Mirror::resolve(deps, workspace_id, &contact_inbox, bases).await?;
    let Some(message_id) = message_source_id else {
        return Err(MediaError::Terminal(format!(
            "Message source id is missing for attachment {id}"
        )));
    };
    attachments.sort_by_key(|a| a.id);
    let Some(index) = attachments.iter().position(|a| a.id == id) else {
        return Ok(false);
    };
    let requested = &attachments[index];
    if !is_mirrored(&requested.origin_path) {
        return Ok(false);
    }
    let media = mirror.graph_media(&message_id).await?;
    let resolved = match &requested.source_id {
        Some(source_id) => media.iter().find(|m| m.source_id == *source_id),
        None => pair_by_position(&media, &requested.file_type, index, attachments.len()),
    }
    // A single-attachment message pairs by position even without an id.
    .or_else(|| (attachments.len() == 1 && media.len() == 1).then(|| &media[0]));
    let Some(resolved) = resolved else {
        return Ok(false);
    };
    let (bytes, mime_type) = match mirror.download_graph(resolved, requested).await {
        Ok(got) => got,
        Err(GraphFetch::TooLarge) => {
            return Err(MediaError::Terminal(format!(
                "Attachment {id} exceeds the media size limit"
            )));
        }
        Err(GraphFetch::Gone(error) | GraphFetch::Failed(error)) => return Err(retry(error)),
    };
    let files = deps
        .files
        .as_ref()
        .ok_or_else(|| retry("[media-hydration] Object storage is unavailable"))?;
    // `encodeForExistingKey`: a re-encode must keep the key's extension true.
    let (encoded, encoded_mime, optimized, encoded_dims) =
        encode(bytes.clone(), mime_type.clone(), requested.keep_original).await;
    let extension = requested
        .origin_path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext);
    let keeps_key =
        !optimized || extension.is_none_or(|ext| mime_extension(&encoded_mime) == Some(ext));
    let (bytes, mime_type, optimized, dims) = if keeps_key {
        (encoded, encoded_mime, optimized, encoded_dims)
    } else {
        let dims = if mime_type.starts_with("image/") {
            read_image_dimensions(&bytes)
        } else {
            None
        };
        (bytes, mime_type, false, dims)
    };
    let size = bytes.len();
    files
        .put_object(
            &requested.origin_path,
            bytes,
            PutOptions {
                acl: requested
                    .origin_path
                    .starts_with("public/")
                    .then_some(Acl::PublicRead),
                content_type: Some(mime_type.clone()),
                content_length: Some(size as u64),
                ..PutOptions::default()
            },
            None,
        )
        .await
        .map_err(retry)?;
    update(
        deps,
        workspace_id,
        requested,
        UpdateAttachmentFields {
            height: dims.map(|d| i64::from(d.1)).or(requested.height),
            mime_type,
            origin_path: requested.origin_path.clone(),
            size: size as i64,
            width: dims.map(|d| i64::from(d.0)).or(requested.width),
        },
    )
    .await?;
    tracing::info!(
        attachment_id = id,
        optimized,
        "media-hydration: attachment restored"
    );
    Ok(true)
}

fn is_graph_channel(channel: &str) -> bool {
    matches!(channel, "messenger" | "instagram")
}

/// `isMirroredOriginPath`: neither failed nor pending.
fn is_mirrored(origin_path: &str) -> bool {
    !(origin_path.starts_with(FAILED_PREFIX) || is_pending(origin_path))
}

/// `isLiveWebhookRow`: no provider id, and a Meta CDN URL.
fn is_live_webhook_row(attachment: &Attachment) -> bool {
    attachment.source_id.is_none()
        && is_url_on_hosts(&attachment.origin_path, Some(META_MEDIA_HOSTS))
}

/// One entry of `getMessageMediaUrls`.
#[derive(Debug, Clone, PartialEq)]
struct Fresh {
    source_id: String,
    url: String,
    mime_type: Option<String>,
}

/// `pairByPosition`: only when the counts match and the kind agrees.
fn pair_by_position<'a>(
    media: &'a [Fresh],
    file_type: &str,
    index: usize,
    count: usize,
) -> Option<&'a Fresh> {
    let candidate = media.get(index).filter(|_| media.len() == count)?;
    (guess_file_type_from_mime_type(candidate.mime_type.as_deref().unwrap_or_default()).as_str()
        == file_type)
        .then_some(candidate)
}

/// `getStorageExtension`: the URL path's extension, else the MIME one.
fn storage_extension(url: Option<&str>, mime_type: &str) -> Option<String> {
    let from_url = url
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .and_then(|u| url::Url::parse(u).ok())
        .and_then(|u| {
            let name = u.path().rsplit('/').next()?.to_owned();
            let (stem, ext) = name.rsplit_once('.')?;
            (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_owned())
        });
    from_url.or_else(|| mime_extension(mime_type).map(str::to_owned))
}

/// Why a Graph media download failed.
enum GraphFetch {
    TooLarge,
    /// The URL will never serve the media again (`isUrlGone`: a 4xx except
    /// 408 / 429, or no bytes).
    Gone(String),
    Failed(String),
}

/// Why [`download_whatsapp_media`] failed.
#[derive(Debug)]
pub(crate) enum GraphMediaError {
    /// Over `MAX_ATTACHMENT_BYTES` (`AttachmentTooLargeError`).
    TooLarge,
    Failed(String),
}

/// `downloadWhatsappMedia`: `retrieveMedia(id)`, then the bytes with the
/// token (only on the media URL's origin, every hop SSRF-checked), capped at
/// 100 MB. The mime type is the served one, else Graph's.
pub(crate) async fn download_whatsapp_media(
    graph: &reqwest::Client,
    download: &reqwest::Client,
    graph_url: &str,
    token: &str,
    media_id: &str,
) -> Result<(Bytes, String), GraphMediaError> {
    let failed = |e: reqwest::Error| GraphMediaError::Failed(e.to_string());
    let body = graph
        .get(format!("{graph_url}/{DEFAULT_API_VERSION}/{media_id}"))
        .bearer_auth(token)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(failed)?
        .bytes()
        .await
        .map_err(failed)?;
    let info: Value = serde_json::from_slice(&body)
        .map_err(|e| GraphMediaError::Failed(format!("[media-hydration] retrieveMedia: {e}")))?;
    let (Some(url), Some(info_mime)) = (info["url"].as_str(), info["mime_type"].as_str()) else {
        return Err(GraphMediaError::Failed(format!(
            "[media-hydration] WhatsApp retrieveMedia returned no url for {media_id}"
        )));
    };
    let (bytes, declared) = match download_checked(
        download,
        url,
        Some(token),
        MAX_ATTACHMENT_BYTES,
        "WhatsApp media URL",
    )
    .await
    {
        Ok(got) => got,
        Err(DownloadError::TooLarge) => return Err(GraphMediaError::TooLarge),
        Err(DownloadError::Status(status)) => {
            return Err(GraphMediaError::Failed(format!(
                "[media-hydration] WhatsApp fetch failed: {status}"
            )));
        }
        Err(DownloadError::Unsafe(e) | DownloadError::Other(e)) => {
            return Err(GraphMediaError::Failed(e));
        }
    };
    if bytes.is_empty() {
        return Err(GraphMediaError::Failed(
            "[media-hydration] WhatsApp fetch returned no bytes".into(),
        ));
    }
    Ok((bytes, declared.unwrap_or_else(|| info_mime.to_owned())))
}

struct Mirror<'a> {
    deps: &'a Deps,
    workspace_id: i64,
    /// The WhatsApp / Messenger / Instagram-via-Facebook host, or the native
    /// Instagram one.
    graph_url: String,
    version: String,
    token: String,
    loc: ChannelMediaLocation,
    graph: reqwest::Client,
    download: reqwest::Client,
}

impl<'a> Mirror<'a> {
    /// `resolveHydrationContext`: the channel's integration and token.
    async fn resolve(
        deps: &'a Deps,
        workspace_id: i64,
        contact_inbox: &ContactInbox,
        bases: &Bases,
    ) -> MediaResult<Mirror<'a>> {
        let (_, ctx) = deps
            .channels
            .resolve(&deps.db, None, workspace_id, contact_inbox)
            .await
            .map_err(retry)?;
        let token = ctx
            .auth
            .pointer("/tokens/accessToken")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if is_graph_channel(&contact_inbox.channel) && token.is_empty() {
            return Err(retry(format!(
                "[media-hydration] Missing access token for {}",
                contact_inbox.channel
            )));
        }
        let integration_id = ctx.integration_id;
        let native_instagram = contact_inbox.channel == "instagram"
            && chatbotx_db::repo::read(&deps.db, move |c| {
                use chatbotx_db::rusqlite::OptionalExtension as _;
                Ok(c.query_row(
                    r#"SELECT "type" FROM "IntegrationInstagram" WHERE "id" = ?1"#,
                    [integration_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
            })
            .await
            .map_err(retry)?
            .as_deref()
                == Some("instagram");
        let version = ctx
            .auth
            .pointer("/metadata/version")
            .and_then(Value::as_str)
            .unwrap_or(if native_instagram {
                "v22.0"
            } else {
                DEFAULT_API_VERSION
            })
            .to_owned();
        Ok(Mirror {
            deps,
            workspace_id,
            graph_url: if native_instagram {
                bases.instagram.clone()
            } else {
                bases.facebook.clone()
            },
            version,
            token,
            loc: ChannelMediaLocation {
                channel: contact_inbox.channel.clone(),
                workspace_id: workspace_id.to_string(),
                integration_id: ctx.integration_id.to_string(),
            },
            graph: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            download: media_client(),
        })
    }

    /// `downloadWhatsappMedia`: `retrieveMedia(id)`, then the bytes with the
    /// token (only on the media URL's origin), capped at 100 MB.
    async fn fetch(&self, attachment: &Attachment) -> MediaResult<(Bytes, String)> {
        let Some(media_id) = attachment.origin_path.strip_prefix(WA_MEDIA_PREFIX) else {
            return Err(MediaError::Terminal(format!(
                "WhatsApp attachment {} has no media id",
                attachment.id
            )));
        };
        match download_whatsapp_media(
            &self.graph,
            &self.download,
            &self.graph_url,
            &self.token,
            media_id,
        )
        .await
        {
            Ok(got) => Ok(got),
            Err(GraphMediaError::TooLarge) => self.too_large(attachment).await,
            Err(GraphMediaError::Failed(error)) => Err(retry(error)),
        }
    }

    async fn too_large<T>(&self, attachment: &Attachment) -> MediaResult<T> {
        mark_failed(self.deps, self.workspace_id, attachment, "too-large").await?;
        Err(MediaError::Terminal(format!(
            "Attachment {} exceeds the media size limit",
            attachment.id
        )))
    }

    /// The WhatsApp strategy of `mirrorPendingAttachment`.
    async fn whatsapp(&self, attachment: &Attachment) -> MediaResult<()> {
        let (bytes, mime_type) = self.fetch(attachment).await?;
        self.store(attachment, bytes, mime_type, None, "channel")
            .await
    }

    /// `resolveGraphMedia` (`getMessageMediaUrls`): the message's attachments
    /// that carry both a URL and a provider id.
    async fn graph_media(&self, message_id: &str) -> MediaResult<Vec<Fresh>> {
        let instagram = self.loc.channel == "instagram";
        let mut url = url::Url::parse(&format!("{}/{}/{message_id}", self.graph_url, self.version))
            .map_err(retry)?;
        url.query_pairs_mut().append_pair(
            "fields",
            if instagram {
                INSTAGRAM_MEDIA_FIELDS
            } else {
                MESSENGER_MEDIA_FIELDS
            },
        );
        let body = self
            .graph
            .get(url.as_str())
            .bearer_auth(&self.token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(retry)?
            .bytes()
            .await
            .map_err(retry)?;
        let parsed: Value = serde_json::from_slice(&body).map_err(retry)?;
        Ok(parsed["attachments"]["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let url = instagram
                    .then(|| a["payload"]["url"].as_str())
                    .flatten()
                    .or_else(|| a["image_data"]["url"].as_str())
                    .or_else(|| a["video_data"]["url"].as_str())
                    .or_else(|| a["file_url"].as_str())?;
                Some(Fresh {
                    source_id: a["id"].as_str()?.to_owned(),
                    url: url.to_owned(),
                    mime_type: a["mime_type"].as_str().map(str::to_owned),
                })
            })
            .collect())
    }

    /// `downloadGraphMedia` / `downloadBearerUrlMedia`: the bytes with the
    /// channel token (every hop SSRF-checked), capped at 100 MB.
    async fn download_graph(
        &self,
        media: &Fresh,
        attachment: &Attachment,
    ) -> Result<(Bytes, String), GraphFetch> {
        let label = format!("{} media URL", self.loc.channel);
        match download_checked(
            &self.download,
            &media.url,
            Some(&self.token),
            MAX_ATTACHMENT_BYTES,
            &label,
        )
        .await
        {
            Ok((bytes, _)) if bytes.is_empty() => Err(GraphFetch::Gone(format!(
                "[media-hydration] {} fetch returned no bytes",
                self.loc.channel
            ))),
            Ok((bytes, declared)) => Ok((
                bytes,
                declared
                    .or_else(|| media.mime_type.clone())
                    .unwrap_or_else(|| attachment.mime_type.clone()),
            )),
            Err(DownloadError::TooLarge) => Err(GraphFetch::TooLarge),
            Err(DownloadError::Status(status)) => {
                let message = format!(
                    "[media-hydration] {} fetch failed: {status}",
                    self.loc.channel
                );
                let code = status.as_u16();
                if (400..500).contains(&code) && code != 408 && code != 429 {
                    Err(GraphFetch::Gone(message))
                } else {
                    Err(GraphFetch::Failed(message))
                }
            }
            Err(DownloadError::Unsafe(e) | DownloadError::Other(e)) => Err(GraphFetch::Failed(e)),
        }
    }

    /// The Graph strategy of `mirrorPendingAttachment`.
    async fn graph_pending(
        &self,
        attachment: &Attachment,
        index: usize,
        count: usize,
        is_requested: bool,
        message_id: &str,
        fresh: &mut Option<Vec<Fresh>>,
    ) -> MediaResult<()> {
        let live = is_live_webhook_row(attachment);
        if live {
            let stored = Fresh {
                source_id: String::new(),
                url: attachment.origin_path.clone(),
                mime_type: Some(attachment.mime_type.clone()),
            };
            match self.download_graph(&stored, attachment).await {
                Ok((bytes, mime)) => {
                    return self
                        .store(attachment, bytes, mime, Some(&stored.url), "stored-url")
                        .await;
                }
                Err(GraphFetch::TooLarge) => return self.too_large(attachment).await,
                Err(GraphFetch::Failed(error)) => return Err(retry(error)),
                // The webhook's URL expired: ask the provider.
                Err(GraphFetch::Gone(_)) => {}
            }
        }
        if fresh.is_none() {
            *fresh = Some(self.graph_media(message_id).await?);
        }
        let media = fresh.as_deref().unwrap_or_default();
        let reference = match &attachment.source_id {
            Some(source_id) => media.iter().find(|m| m.source_id == *source_id),
            None => pair_by_position(media, &attachment.file_type, index, count),
        }
        .cloned();
        if let Some(reference) = reference {
            return match self.download_graph(&reference, attachment).await {
                Ok((bytes, mime)) => {
                    self.store(attachment, bytes, mime, Some(&reference.url), "graph")
                        .await
                }
                Err(GraphFetch::TooLarge) => self.too_large(attachment).await,
                Err(GraphFetch::Gone(error) | GraphFetch::Failed(error)) => Err(retry(error)),
            };
        }
        if !is_requested {
            return Err(MediaError::Terminal("no media for a sibling".into()));
        }
        // The provider answered but no longer exposes this attachment.
        mark_failed(self.deps, self.workspace_id, attachment, "unresolvable").await?;
        Err(MediaError::Terminal(format!(
            "Media reference is unavailable for attachment {}",
            attachment.id
        )))
    }

    /// `mirrorAttachment`: re-encode a photo, store it (a live webhook row
    /// public-read under the channel prefix, else privately under the
    /// mirrored prefix), and point the row at it.
    async fn store(
        &self,
        attachment: &Attachment,
        bytes: Bytes,
        mime_type: String,
        source_url: Option<&str>,
        source: &'static str,
    ) -> MediaResult<()> {
        let files = self
            .deps
            .files
            .as_ref()
            .ok_or_else(|| retry("[media-hydration] Object storage is unavailable"))?;
        let before = bytes.len();
        let (bytes, mime_type, optimized, dims) =
            encode(bytes, mime_type, attachment.keep_original).await;
        let extension = if optimized {
            mime_extension(&mime_type).map(str::to_owned)
        } else {
            storage_extension(source_url, &mime_type)
        }
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
        let live = is_live_webhook_row(attachment);
        let prefix = if live {
            get_channel_media_prefix(&self.loc)
        } else {
            get_mirrored_channel_media_prefix(&self.loc)
        };
        let key = format!(
            "{prefix}/{}{extension}",
            chatbotx_core::utils::id::create_id()
        );
        let size = bytes.len();
        files
            .put_object(
                &key,
                bytes,
                PutOptions {
                    acl: live.then_some(Acl::PublicRead),
                    content_type: Some(mime_type.clone()),
                    content_length: Some(size as u64),
                    ..PutOptions::default()
                },
                None,
            )
            .await
            .map_err(retry)?;
        update(
            self.deps,
            self.workspace_id,
            attachment,
            UpdateAttachmentFields {
                height: dims.map(|d| i64::from(d.1)).or(attachment.height),
                mime_type,
                origin_path: key,
                size: size as i64,
                width: dims.map(|d| i64::from(d.0)).or(attachment.width),
            },
        )
        .await?;
        tracing::info!(
            attachment_id = attachment.id,
            channel = %self.loc.channel,
            source,
            optimized,
            bytes_before = before,
            bytes_after = size,
            "media-hydration: attachment mirrored"
        );
        Ok(())
    }
}

/// `encodeMirroredImage` + `storedImageDimensions`: only a photo (not sent as
/// a file) goes through the optimizer, waiting for an encode slot.
async fn encode(
    bytes: Bytes,
    mime_type: String,
    keep_original: Option<bool>,
) -> (Bytes, String, bool, Option<(u32, u32)>) {
    let is_image = mime_type.starts_with("image/");
    let read_dims = |b: &Bytes| {
        if is_image {
            read_image_dimensions(b)
        } else {
            None
        }
    };
    if keep_original == Some(true)
        || !is_image
        || !is_stored_image_optimization_enabled(&ProcessEnv)
    {
        let dims = read_dims(&bytes);
        return (bytes, mime_type, false, dims);
    }
    let Ok(settings) = get_image_optimization_settings(&ProcessEnv) else {
        let dims = read_dims(&bytes);
        return (bytes, mime_type, false, dims);
    };
    if bytes.len() as u64 > settings.job.max_source_bytes {
        let dims = read_dims(&bytes);
        return (bytes, mime_type, false, dims);
    }
    let source = bytes.clone();
    let mime = mime_type.clone();
    let slot = EncodeSlots::global()
        .run_in_encode_slot(
            settings.inline.max_concurrent_encodes.max(1),
            true,
            || async move {
                Ok::<_, ()>(
                    optimize_image_for_storage(
                        source,
                        &mime,
                        ImageOptimizeProfile::Media,
                        &ProcessEnv,
                    )
                    .await,
                )
            },
        )
        .await;
    match slot {
        Ok(Some(encoded)) if encoded.optimized => {
            let dims = encoded.width.zip(encoded.height);
            (encoded.bytes, encoded.mime_type, true, dims)
        }
        _ => {
            let dims = read_dims(&bytes);
            (bytes, mime_type, false, dims)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound::tests::setup;

    fn job(data: Value, attempts_made: u32) -> JobCtx {
        JobCtx {
            id: 1,
            job_id: "1".into(),
            name: JOB.into(),
            data: json!({ "type": JOB, "data": data }),
            attempts: Some(5),
            attempts_made,
            attempts_started: attempts_made + 1,
            priority: 0,
            lease_token: None,
        }
    }

    async fn seed(deps: &Deps, origin_path: &str) {
        let origin_path = origin_path.to_owned();
        deps.db
            .write(move |tx| {
                for sql in [
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (7, 2, 'Wa', 'whatsapp', 'phone-1')"#,
                    r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId") VALUES (8, '{"tokens":{"accessToken":"t"}}', 'phone-1', 'w', 'b', 'Wa', 2, 7)"#,
                    r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (10, 2, 'Ann')"#,
                    r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES (11, 10, 10, 7, 'whatsapp', 'whatsapp', '849')"#,
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (12, 2, 10)"#,
                    r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "contentType", "messageType", "senderType", "createdAt", "updatedAt") VALUES (13, 2, 12, 11, 'text', 'incoming', 'contact', 1000, 1000)"#,
                ] {
                    tx.execute(sql, [])?;
                }
                tx.execute(
                    r#"INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "size", "originPath", "createdAt", "updatedAt")
                       VALUES (14, 2, 12, 13, 1000, 'image', 'image/jpeg', 0, ?1, 1000, 1000)"#,
                    [origin_path],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn origin(deps: &Deps) -> String {
        deps.db
            .read(|c| {
                Ok(c.query_row(
                    r#"SELECT "originPath" FROM "Attachment" WHERE "id" = 14"#,
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap()
    }

    fn data() -> Value {
        json!({ "attachmentId": "14", "workspaceId": "2", "channel": "whatsapp", "integrationId": "8" })
    }

    #[test]
    fn job_shape_and_attempt_arithmetic() {
        let opts = job_options(42);
        assert_eq!(opts.job_id.as_deref(), Some("att-42"));
        assert!(
            !opts.job_id.unwrap().contains(':'),
            "BullMQ forbids ':' in a job id"
        );
        assert_eq!((opts.attempts, opts.priority), (Some(5), Some(10)));
        assert!(!job(data(), 3).is_final_attempt());
        assert!(job(data(), 4).is_final_attempt());
        assert_eq!(mime_extension("image/jpg"), Some("jpg"));
        assert!(is_pending("wa-media:1") && is_pending("https://x") && !is_pending("failed:x"));
    }

    #[tokio::test]
    async fn enqueues_one_low_job_per_attachment_and_none_for_an_empty_list() {
        let (deps, jobs) = setup().await;
        enqueue(&deps, 2, 8, "whatsapp", &[]).await.unwrap();
        enqueue(&deps, 2, 8, "whatsapp", &[14, 15]).await.unwrap();
        let added = jobs
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.1 == JOB)
            .map(|j| (j.3.job_id.clone(), j.2["data"]["attachmentId"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            added,
            vec![
                (Some("att-14".into()), json!("14")),
                (Some("att-15".into()), json!("15"))
            ]
        );
    }

    #[tokio::test]
    async fn a_transient_failure_retries_then_marks_the_row_unresolvable_on_the_final_attempt() {
        let (deps, _) = setup().await;
        seed(&deps, "wa-media:m1").await;
        // A closed port: the Graph lookup fails (transport error = retryable).
        let graph = &bases("http://127.0.0.1:9");
        assert!(handle_with(&deps, &job(data(), 0), graph).await.is_err());
        assert_eq!(
            origin(&deps).await,
            "wa-media:m1",
            "an early attempt leaves the row pending"
        );
        assert!(handle_with(&deps, &job(data(), 4), graph).await.is_err());
        assert_eq!(origin(&deps).await, "failed:unresolvable");
        // Terminal now: the next run succeeds without a retry.
        handle_with(&deps, &job(data(), 0), graph).await.unwrap();
    }

    #[tokio::test]
    async fn a_whatsapp_row_without_a_media_id_and_a_restore_are_terminal_no_ops() {
        let (deps, _) = setup().await;
        seed(&deps, "https://lookaside.example/x").await;
        handle_with(&deps, &job(data(), 4), &bases("http://127.0.0.1:9"))
            .await
            .unwrap();
        assert_eq!(origin(&deps).await, "https://lookaside.example/x");
        let mut restore = data();
        restore["restore"] = json!(true);
        handle_with(&deps, &job(restore, 4), &bases("http://127.0.0.1:9"))
            .await
            .unwrap();
        let missing = json!({ "attachmentId": "99", "workspaceId": "2", "channel": "whatsapp" });
        handle_with(&deps, &job(missing, 0), &bases("http://127.0.0.1:9"))
            .await
            .unwrap();
    }

    fn bases(url: &str) -> Bases {
        Bases {
            facebook: url.into(),
            instagram: url.into(),
        }
    }

    /// One HTTP answer on a local port (the Graph `getMessageMediaUrls`).
    fn graph_once(body: Value) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        url
    }

    /// A Messenger message (`sourceId` = the Graph id) with two attachments.
    async fn seed_messenger(deps: &Deps, message_source: Option<&str>, origin_path: &str) {
        let (message_source, origin_path) =
            (message_source.map(str::to_owned), origin_path.to_owned());
        deps.db
            .write(move |tx| {
                for sql in [
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (7, 2, 'Fb', 'messenger', 'page-1')"#,
                    r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId") VALUES (8, '{"tokens":{"accessToken":"t"}}', 'page-1', 'Fb', '[]', '[]', '[]', 2, 7)"#,
                    r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (10, 2, 'Ann')"#,
                    r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES (11, 10, 10, 7, 'messenger', 'messenger', 'psid-1')"#,
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (12, 2, 10)"#,
                ] {
                    tx.execute(sql, [])?;
                }
                tx.execute(
                    r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "contentType", "messageType", "senderType", "sourceId", "createdAt", "updatedAt") VALUES (13, 2, 12, 11, 'text', 'incoming', 'contact', ?1, 1000, 1000)"#,
                    [message_source],
                )?;
                tx.execute(
                    r#"INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "size", "originPath", "sourceId", "createdAt", "updatedAt")
                       VALUES (14, 2, 12, 13, 1000, 'image', 'image/jpeg', 0, ?1, 'att-a', 1000, 1000),
                              (15, 2, 12, 13, 1000, 'video', 'video/mp4', 0, 'https://cdn.example/v.mp4', 'att-b', 1000, 1000)"#,
                    [origin_path],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn messenger_data() -> Value {
        json!({ "attachmentId": "14", "workspaceId": "2", "channel": "messenger", "integrationId": "8" })
    }

    #[tokio::test]
    async fn a_graph_row_without_a_message_id_or_a_fresh_entry_is_unresolvable() {
        let (deps, _) = setup().await;
        seed_messenger(&deps, None, "https://cdn.example/a.jpg").await;
        handle_with(
            &deps,
            &job(messenger_data(), 0),
            &bases("http://127.0.0.1:9"),
        )
        .await
        .unwrap();
        assert_eq!(origin(&deps).await, "failed:unresolvable");

        let (deps, _) = setup().await;
        seed_messenger(&deps, Some("mid.1"), "https://cdn.example/a.jpg").await;
        // Graph lists only the sibling: the requested row cannot be re-derived.
        let graph = graph_once(json!({ "attachments": { "data": [
            { "id": "att-b", "mime_type": "video/mp4", "video_data": { "url": "https://cdn.example/v2.mp4" } } ] } }));
        handle_with(&deps, &job(messenger_data(), 0), &bases(&graph))
            .await
            .unwrap();
        assert_eq!(origin(&deps).await, "failed:unresolvable");
        // The sibling's download goes to a non-public host: it stays pending.
        let sibling: String = deps
            .db
            .read(|c| {
                Ok(c.query_row(
                    r#"SELECT "originPath" FROM "Attachment" WHERE "id" = 15"#,
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(sibling, "https://cdn.example/v.mp4");
    }

    #[tokio::test]
    async fn restore_skips_rows_that_are_not_mirrored_and_a_message_without_an_id() {
        let (deps, _) = setup().await;
        seed_messenger(&deps, None, "workspace/fb/x.jpg").await;
        let mut restore = messenger_data();
        restore["restore"] = json!(true);
        // Terminal (`Message source id is missing`): logged, the row is kept.
        handle_with(
            &deps,
            &job(restore.clone(), 0),
            &bases("http://127.0.0.1:9"),
        )
        .await
        .unwrap();
        assert_eq!(origin(&deps).await, "workspace/fb/x.jpg");
        let (deps, _) = setup().await;
        seed_messenger(&deps, Some("mid.1"), "failed:too-large").await;
        assert!(!restore_row(&deps).await);
    }

    #[tokio::test]
    async fn a_failed_restore_on_the_final_attempt_never_marks_the_row() {
        let (deps, _) = setup().await;
        seed_messenger(&deps, Some("mid.1"), "workspace/fb/x.jpg").await;
        let mut restore = messenger_data();
        restore["restore"] = json!(true);
        // The Graph lookup fails (closed port): retryable, the row is kept.
        assert!(
            handle_with(&deps, &job(restore, 4), &bases("http://127.0.0.1:9"))
                .await
                .is_err()
        );
        assert_eq!(origin(&deps).await, "workspace/fb/x.jpg");
    }

    async fn restore_row(deps: &Deps) -> bool {
        restore(deps, 2, 14, &bases("http://127.0.0.1:9"))
            .await
            .unwrap()
    }

    #[test]
    fn graph_pairing_and_storage_extensions_follow_the_original() {
        let fresh = |id: &str, mime: &str| Fresh {
            source_id: id.into(),
            url: format!("https://cdn.example/{id}"),
            mime_type: Some(mime.into()),
        };
        let media = [fresh("x", "image/png"), fresh("y", "video/mp4")];
        assert_eq!(pair_by_position(&media, "image", 0, 2), Some(&media[0]));
        assert_eq!(
            pair_by_position(&media, "image", 1, 2),
            None,
            "kind differs"
        );
        assert_eq!(
            pair_by_position(&media, "image", 0, 3),
            None,
            "counts differ"
        );
        assert_eq!(
            storage_extension(Some("https://cdn.example/a/b.webp?x=1"), "image/png").as_deref(),
            Some("webp")
        );
        assert_eq!(
            storage_extension(Some("https://cdn.example/a/b"), "image/png").as_deref(),
            Some("png")
        );
        assert_eq!(
            storage_extension(Some("wa-media:1"), "audio/ogg").as_deref(),
            Some("ogg")
        );
        assert!(
            is_mirrored("workspace/a.png") && !is_mirrored("failed:x") && !is_mirrored("https://a")
        );
    }
}
