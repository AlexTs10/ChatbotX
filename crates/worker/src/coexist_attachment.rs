//! `coexistAttachmentDownload` (low queue): copy the bytes of a history-sync
//! attachment (`wa-media:{id}` stub) to our storage (`coexist/
//! attachment-download.ts`, `enqueue-attachment-downloads.ts`, the
//! `ensureAttachmentMirrored` / `markAttachmentUnresolvable` half of
//! `channel-registry/media-hydration.ts`).
//!
//! ponytail: only the WhatsApp strategy is ported. Messenger / Instagram rows
//! (Graph `getMessageMediaUrls`) keep their provider URL as the media, like a
//! channel without a strategy; `restore` (re-derive an evicted object) is a
//! no-op, which is what the original answers for WhatsApp (no fresh URL). The
//! per-message lock is in-process (one worker process).
use std::sync::LazyLock;

use bytes::Bytes;
use chatbotx_core::env::ProcessEnv;
use chatbotx_core::utils::image_optimize::{
    ImageOptimizeProfile, optimize_image_for_storage, read_image_dimensions,
};
use chatbotx_core::utils::image_optimize_settings::{
    EncodeSlots, get_image_optimization_settings, is_stored_image_optimization_enabled,
};
use chatbotx_db::repo::contact_inbox;
use chatbotx_db::repo::message::{
    self, FindAttachmentByIdParams, FindMessageByIdParams, UpdateAttachmentFields,
    UpdateAttachmentParams,
};
use chatbotx_db::repo::models::{Attachment, ContactInbox};
use chatbotx_files::{ChannelMediaLocation, PutOptions, get_mirrored_channel_media_prefix};
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
    handle_with(deps, job, API_URL).await
}

async fn handle_with(deps: &Deps, job: &JobCtx, graph_url: &str) -> Result<(), JobError> {
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
        // ponytail: `restoreMirroredAttachment` answers `{ restored: false }`.
        tracing::debug!(
            attachment_id,
            channel,
            "[coexist-attachment] restore is not ported"
        );
        return Ok(());
    }
    match ensure_mirrored(deps, workspace_id, attachment_id, graph_url).await {
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

/// The attachment's message (with its attachments) and contact inbox.
async fn load_graph(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
) -> Result<Option<(Vec<Attachment>, ContactInbox)>, JobError> {
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
        Ok(inbox.map(|inbox| (found.attachments, inbox)))
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
        .and_then(|(attachments, _)| attachments.into_iter().find(|a| a.id == id))
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
    graph_url: &str,
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

    let (mut attachments, contact_inbox) = load_graph(deps, workspace_id, id)
        .await
        .map_err(retry)?
        .ok_or_else(not_found)?;
    let requested = attachments
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(not_found)?;
    if requested.origin_path.starts_with(FAILED_PREFIX) {
        return Err(MediaError::Terminal(format!(
            "Attachment {id} is permanently unavailable"
        )));
    }
    if !is_pending(&requested.origin_path) {
        return Ok(());
    }
    if contact_inbox.channel != "whatsapp" {
        // A channel without a (ported) strategy keeps the provider's URL.
        if requested.origin_path.starts_with("http://")
            || requested.origin_path.starts_with("https://")
        {
            return Ok(());
        }
        return Err(MediaError::Terminal(format!(
            "No media hydration strategy for attachment {id}"
        )));
    }
    let (_, ctx) = deps
        .channels
        .resolve(&deps.db, None, workspace_id, &contact_inbox)
        .await
        .map_err(retry)?;
    let token = ctx
        .auth
        .pointer("/tokens/accessToken")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mirror = Mirror {
        deps,
        workspace_id,
        graph_url,
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
    };
    attachments.sort_by_key(|a| (a.id != id, a.id));
    for attachment in &attachments {
        if !is_pending(&attachment.origin_path) {
            continue;
        }
        match mirror.whatsapp(attachment).await {
            Ok(()) => {}
            Err(err) if attachment.id == id => return Err(err),
            Err(MediaError::Terminal(_)) => {}
            Err(MediaError::Retry(error)) => {
                tracing::warn!(%error, attachment_id = attachment.id, "media-hydration: sibling attachment not mirrored; left pending");
            }
        }
    }
    Ok(())
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
    graph_url: &'a str,
    token: String,
    loc: ChannelMediaLocation,
    graph: reqwest::Client,
    download: reqwest::Client,
}

impl Mirror<'_> {
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
            self.graph_url,
            &self.token,
            media_id,
        )
        .await
        {
            Ok(got) => Ok(got),
            Err(GraphMediaError::TooLarge) => {
                mark_failed(self.deps, self.workspace_id, attachment, "too-large").await?;
                Err(MediaError::Terminal(format!(
                    "Attachment {} exceeds the media size limit",
                    attachment.id
                )))
            }
            Err(GraphMediaError::Failed(error)) => Err(retry(error)),
        }
    }

    /// `mirrorAttachment`: download, re-encode a photo, store it privately
    /// under the mirrored prefix, and point the row at it.
    async fn whatsapp(&self, attachment: &Attachment) -> MediaResult<()> {
        let (bytes, mime_type) = self.fetch(attachment).await?;
        let files = self
            .deps
            .files
            .as_ref()
            .ok_or_else(|| retry("[media-hydration] Object storage is unavailable"))?;
        let before = bytes.len();
        let (bytes, mime_type, optimized, dims) =
            encode(bytes, mime_type, attachment.keep_original).await;
        let extension = mime_extension(&mime_type)
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let key = format!(
            "{}/{}{extension}",
            get_mirrored_channel_media_prefix(&self.loc),
            chatbotx_core::utils::id::create_id()
        );
        let size = bytes.len();
        files
            .put_object(
                &key,
                bytes,
                PutOptions {
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
            channel = "whatsapp",
            source = "channel",
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
        let graph = "http://127.0.0.1:9";
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
        handle_with(&deps, &job(data(), 4), "http://127.0.0.1:9")
            .await
            .unwrap();
        assert_eq!(origin(&deps).await, "https://lookaside.example/x");
        let mut restore = data();
        restore["restore"] = json!(true);
        handle_with(&deps, &job(restore, 4), "http://127.0.0.1:9")
            .await
            .unwrap();
        let missing = json!({ "attachmentId": "99", "workspaceId": "2", "channel": "whatsapp" });
        handle_with(&deps, &job(missing, 0), "http://127.0.0.1:9")
            .await
            .unwrap();
    }
}
