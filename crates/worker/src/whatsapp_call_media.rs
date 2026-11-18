//! Call recordings and transcripts (`handlers/whatsapp-call-native-
//! recording.ts`, `whatsapp-call-native-transcript.ts`, `whatsapp-call-
//! recording.ts`, `shared/whatsapp-call-native-media.ts`, the
//! `enrichCallActivityMessage` half of `shared/whatsapp-call-finalize.ts`,
//! `callRecordingService.uploadRecording`).
//!
//! The media come from Meta: the Graph media id first (SSRF-checked, capped
//! at 100 MB), then the webhook's lookaside URL, which only gets the token on
//! a Meta media host (https). The recording is stored privately; the
//! `callRecorded` event carries a 15-minute presigned link, as the original.
use std::time::Duration;

use bytes::Bytes;
use chatbotx_db::repo::message::{
    self as repo_message, CreateAttachmentInput, HasAttachmentOfTypeParams,
};
use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::whatsapp_call::{self as calls, AttachTranscriptInput};
use chatbotx_db::repo::{integration_whatsapp, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use chatbotx_rt::protocol::EventType;
use serde_json::{Map, Value, json};

use crate::channels::whatsapp::{API_URL, DEFAULT_API_VERSION};
use crate::channels::whatsapp_media::{DownloadError, download_checked, media_client};
use crate::coexist_attachment::{GraphMediaError, MAX_ATTACHMENT_BYTES, download_whatsapp_media};
use crate::whatsapp_call::{activity_source_id, emit};
use crate::{Deps, db_fail, fail, id_of, is_blocked_workspace, str_of};

pub const RECORDING_READY_JOB: &str = "whatsappCallRecordingReady";
const RECORDING_SIGNED_URL_TTL_SECS: u64 = 15 * 60;
/// `CALL_FINALIZE_WAIT_DELAYS_MS`.
const FINALIZE_WAIT_MS: [u64; 3] = [500, 1000, 2000];
/// `CALL_MESSAGE_SHARD_LOOKBACK_MS`.
const MESSAGE_LOOKBACK_MS: i64 = 5 * 60 * 1000;
const ALLOWED_MEDIA_HOSTS: [&str; 2] = ["lookaside.fbsbx.com", "graph.facebook.com"];

async fn read<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::read(&deps.db, f).await.map_err(db_fail)
}

async fn write<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::write(&deps.db, move |tx| f(tx))
        .await
        .map_err(db_fail)
}

/// `ALLOWED_RECORDING_CONTENT_TYPES` (content type → extension).
fn recording_extension(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        _ => return None,
    })
}

/// `normalizeRecordingContentType`: the base type, else `audio/ogg`.
fn normalize_recording_type(mime_type: &str) -> String {
    let base = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if recording_extension(&base).is_some() {
        base
    } else {
        "audio/ogg".into()
    }
}

/// `isTrustedMediaUrl`.
fn is_trusted_media_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    parsed.scheme() == "https"
        && ALLOWED_MEDIA_HOSTS
            .iter()
            .any(|a| host == *a || host.ends_with(&format!(".{a}")))
}

/// What `downloadCallMedia` answered.
enum CallMedia {
    Got(Bytes, String),
    /// `WhatsappCallMediaGoneError` / `AttachmentTooLargeError`: skip for good.
    Skip(String),
}

/// `downloadCallMedia`: the media id first; a failure that Graph confirms as
/// gone (404 / 410) is permanent, any other falls back to the lookaside URL.
async fn download_call_media(
    graph_url: &str,
    token: &str,
    media_id: &str,
    lookaside: &str,
    fallback_mime: &str,
) -> Result<CallMedia, JobError> {
    let graph = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    let download = media_client();
    match download_whatsapp_media(&graph, &download, graph_url, token, media_id).await {
        Ok((bytes, mime)) => return Ok(CallMedia::Got(bytes, mime)),
        Err(GraphMediaError::TooLarge) => return Ok(CallMedia::Skip("exceeds size cap".into())),
        Err(GraphMediaError::Failed(error)) => {
            let gone = graph
                .get(format!("{graph_url}/{DEFAULT_API_VERSION}/{media_id}"))
                .bearer_auth(token)
                .send()
                .await
                .is_ok_and(|r| matches!(r.status().as_u16(), 404 | 410));
            if gone {
                return Ok(CallMedia::Skip(format!(
                    "media id {media_id} not found at Graph"
                )));
            }
            tracing::warn!(%error, "Whatsapp call native media: media-id download failed (transient); falling back to lookaside URL for this attempt");
        }
    }
    if !is_trusted_media_url(lookaside) {
        return Ok(CallMedia::Skip(
            "download URL is not a Meta media host; refusing to send the access token".into(),
        ));
    }
    match download_checked(
        &download,
        lookaside,
        Some(token),
        MAX_ATTACHMENT_BYTES,
        "WhatsApp call media URL",
    )
    .await
    {
        Ok((bytes, declared)) => Ok(CallMedia::Got(
            bytes,
            declared.unwrap_or_else(|| fallback_mime.to_owned()),
        )),
        Err(DownloadError::TooLarge) => Ok(CallMedia::Skip("exceeds size cap".into())),
        Err(DownloadError::Status(status)) => {
            Err(fail(format!("lookaside URL fetch failed: {status}")))
        }
        Err(DownloadError::Unsafe(e) | DownloadError::Other(e)) => Err(fail(e)),
    }
}

/// The row by id, else by wacid; missing = retry (the row-creating job may lag).
async fn resolve_row(deps: &Deps, data: &Value) -> Result<Option<WhatsappCall>, JobError> {
    let wacid = str_of(data, "wacid").unwrap_or_default().to_owned();
    let by_id = match id_of(&data["whatsappCallId"]) {
        Some(id) => read(deps, move |c| calls::find_by_id(c, id)).await?,
        None => None,
    };
    let call = match by_id {
        Some(call) => call,
        None => {
            let key = wacid.clone();
            match read(deps, move |c| calls::find_by_wacid(c, &key)).await? {
                Some(call) => call,
                None => return Err(fail(format!("whatsapp-call-row-not-ready: {wacid}"))),
            }
        }
    };
    // A job queued before its row existed carries no workspaceId: gate it here.
    if data.get("workspaceId").is_none_or(Value::is_null)
        && is_blocked_workspace(&deps.db, &call.workspace_id.to_string()).await?
    {
        tracing::info!(
            call_id = call.id,
            "[wa-call-media] skipped: blocked workspace"
        );
        return Ok(None);
    }
    Ok(Some(call))
}

/// `resolveVoipAuthByInboxId`: the number's access token.
async fn access_token(deps: &Deps, call: &WhatsappCall) -> Result<String, JobError> {
    let (ws, inbox) = (call.workspace_id, call.inbox_id);
    let row = read(deps, move |c| {
        integration_whatsapp::find_by_inbox_id_for_workspace(c, ws, inbox)
    })
    .await?;
    row.and_then(|r| {
        r.auth
            .0
            .pointer("/tokens/accessToken")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
    .ok_or_else(|| {
        fail(format!(
            "WhatsApp integration auth missing for inbox {inbox}"
        ))
    })
}

/// `whatsappCallNativeRecordingFetch`.
pub async fn native_recording_fetch(deps: &Deps, data: &Value) -> Result<(), JobError> {
    native_recording_fetch_with(deps, data, API_URL).await
}

async fn native_recording_fetch_with(
    deps: &Deps,
    data: &Value,
    graph_url: &str,
) -> Result<(), JobError> {
    let Some(call) = resolve_row(deps, data).await? else {
        return Ok(());
    };
    if call.recorded_at.is_some() {
        tracing::info!(
            call_id = call.id,
            "[wa-call-recording] already processed; skipping"
        );
        return Ok(());
    }
    let token = access_token(deps, &call).await?;
    let mime = str_of(data, "mimeType").unwrap_or("audio/ogg");
    let media = download_call_media(
        graph_url,
        &token,
        str_of(data, "audioMediaId").unwrap_or_default(),
        str_of(data, "audioUrl").unwrap_or_default(),
        mime,
    )
    .await?;
    let (bytes, served) = match media {
        CallMedia::Got(bytes, served) => (bytes, served),
        CallMedia::Skip(reason) => {
            tracing::warn!(
                call_id = call.id,
                reason,
                "[wa-call-recording] media unavailable; skipping"
            );
            return Ok(());
        }
    };
    let resolved = if served.is_empty() {
        mime.to_owned()
    } else {
        served
    };
    let content_type = normalize_recording_type(&resolved);
    let extension = recording_extension(&content_type).unwrap_or("ogg");
    let path = format!("space/{}/calls/{}.{extension}", call.workspace_id, call.id);
    let files = deps
        .files
        .as_ref()
        .ok_or_else(|| fail("File storage is not configured"))?;
    let size = bytes.len();
    files
        .put_object(
            &path,
            bytes,
            chatbotx_files::PutOptions {
                content_type: Some(content_type),
                content_length: Some(size as u64),
                ..chatbotx_files::PutOptions::default()
            },
            None,
        )
        .await
        .map_err(|e| fail(e.to_string()))?;
    attach_recording_and_notify(deps, &call, &path, Some(&resolved), Some(size as i64)).await
}

/// `attachRecordingAndNotify`: stamp once, attach the audio to the call
/// card, flag it, emit `callRecorded`; a failure releases the stamp.
async fn attach_recording_and_notify(
    deps: &Deps,
    call: &WhatsappCall,
    path: &str,
    mime_type: Option<&str>,
    size: Option<i64>,
) -> Result<(), JobError> {
    if call.recorded_at.is_some() {
        return Ok(());
    }
    let stamped_at = now_ms();
    let (id, key) = (call.id, path.to_owned());
    let Some(stamped) = write(deps, move |c| {
        calls::attach_recording(c, id, &key, stamped_at)
    })
    .await?
    else {
        return Ok(());
    };
    let result = async {
        let finalized = wait_finalized(deps, call.id).await?;
        let (Some(message_id), Some(ended_at)) = (finalized.message_id, finalized.ended_at) else {
            return Err(fail(format!(
                "whatsapp-call-recording-finalize-not-ready: {}",
                call.id
            )));
        };
        let attachment = CreateAttachmentInput {
            workspace_id: call.workspace_id,
            conversation_id: call.conversation_id,
            message_id,
            message_created_at: ended_at,
            file_type: "audio".into(),
            mime_type: mime_type.unwrap_or("audio/ogg").to_owned(),
            origin_path: path.to_owned(),
            size,
            ..CreateAttachmentInput::default()
        };
        let ws = call.workspace_id;
        write(deps, move |c| {
            let params = HasAttachmentOfTypeParams {
                workspace_id: ws,
                message_id,
                message_created_at: ended_at,
                file_type: "audio".into(),
            };
            if !repo_message::has_attachment_of_type(c, &params)? {
                repo_message::bulk_create_attachments(c, std::slice::from_ref(&attachment))?;
            }
            Ok(())
        })
        .await?;
        enrich(deps, &stamped, json!({ "hasRecording": true })).await?;
        if let Some(contact_id) = contact_id(deps, call.contact_inbox_id).await? {
            let url = match &deps.files {
                Some(files) => files
                    .get_presigned_download(path, Some(RECORDING_SIGNED_URL_TTL_SECS))
                    .await
                    .map_err(|e| fail(e.to_string()))?,
                None => return Err(fail("File storage is not configured")),
            };
            emit(
                deps,
                "callRecorded",
                ws,
                contact_id,
                json!({ "callId": correlation(call), "recordingUrl": url }),
            )
            .await;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        let id = call.id;
        if let Err(error) = write(deps, move |c| {
            calls::release_recording_stamp(c, id, stamped_at)
        })
        .await
        {
            tracing::error!(%error, call_id = id, "[wa-call-recording] could not release the recording stamp after a failed attach");
        }
    }
    result
}

/// `externalCorrelationId`.
fn correlation(call: &WhatsappCall) -> String {
    call.wacid
        .clone()
        .or_else(|| call.attempt_id.clone())
        .unwrap_or_else(|| call.id.to_string())
}

async fn contact_id(deps: &Deps, contact_inbox_id: i64) -> Result<Option<i64>, JobError> {
    read(deps, move |c| {
        Ok(c.query_row(
            r#"SELECT "contactId" FROM "ContactInbox" WHERE "id" = ?1"#,
            params![contact_inbox_id],
            |r| r.get(0),
        )
        .optional()?)
    })
    .await
}

/// `waitUntilReady` on the finalized row (`messageId` and `endedAt`).
async fn wait_finalized(deps: &Deps, id: i64) -> Result<WhatsappCall, JobError> {
    let ready = |c: &Option<WhatsappCall>| {
        c.as_ref()
            .is_some_and(|c| c.message_id.is_some() && c.ended_at.is_some())
    };
    let mut row = read(deps, move |c| calls::find_by_id(c, id)).await?;
    for delay in FINALIZE_WAIT_MS {
        if ready(&row) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(delay)).await;
        row = read(deps, move |c| calls::find_by_id(c, id)).await?;
    }
    row.ok_or_else(|| fail(format!("whatsapp-call-recording-finalize-not-ready: {id}")))
}

/// `enrichCallActivityMessage`: wait for the call card, merge `overrides`
/// into it atomically, then `messageContentUpdated`.
async fn enrich(deps: &Deps, call: &WhatsappCall, overrides: Value) -> Result<(), JobError> {
    let source = activity_source_id(call.id);
    let since = call.created_at - MESSAGE_LOOKBACK_MS;
    let (conversation_id, ws) = (call.conversation_id, call.workspace_id);
    let find = || {
        let source = source.clone();
        read(deps, move |c| {
            repo_message::find_by_source_id(c, &source, conversation_id, ws, Some(since))
        })
    };
    let mut existing = find().await?;
    for delay in FINALIZE_WAIT_MS {
        if existing.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(delay)).await;
        existing = find().await?;
    }
    if existing.is_none() {
        tracing::warn!(
            call_id = call.id,
            "Whatsapp call: no finalize message to enrich after bounded wait"
        );
        return Err(fail(format!(
            "whatsapp-call-enrichment-pending: finalize message not found for callId {} after bounded wait",
            call.id
        )));
    }
    let overlay: Map<String, Value> = overrides.as_object().cloned().unwrap_or_default();
    let key = source.clone();
    let Some(merged) = write(deps, move |c| {
        repo_message::merge_content_attributes_by_source_id(c, &key, ws, &overlay)
    })
    .await?
    else {
        tracing::warn!(
            call_id = call.id,
            "Whatsapp call: enrichment merge matched no row"
        );
        return Ok(());
    };
    tracing::info!(call_id = call.id, %overrides, "[wa-call-media] call card enriched");
    if let Some(conversation) = crate::load_conversation(&deps.db, call.conversation_id).await? {
        let data = json!({ "messageId": merged.id.to_string(), "contentAttributes": merged.content_attributes });
        crate::chat::queue_event(
            deps,
            &conversation,
            call.inbox_id,
            EventType::MessageContentUpdated,
            data,
        )
        .await;
    }
    Ok(())
}

/// `whatsappCallNativeTranscriptFetch`: Meta's diarized JSON document.
pub async fn native_transcript_fetch(deps: &Deps, data: &Value) -> Result<(), JobError> {
    native_transcript_fetch_with(deps, data, API_URL).await
}

async fn native_transcript_fetch_with(
    deps: &Deps,
    data: &Value,
    graph_url: &str,
) -> Result<(), JobError> {
    let Some(call) = resolve_row(deps, data).await? else {
        return Ok(());
    };
    if call.transcript.is_some() {
        tracing::info!(
            call_id = call.id,
            "[wa-call-transcript] already processed; skipping"
        );
        return Ok(());
    }
    let token = access_token(deps, &call).await?;
    let media = download_call_media(
        graph_url,
        &token,
        str_of(data, "documentMediaId").unwrap_or_default(),
        str_of(data, "documentUrl").unwrap_or_default(),
        "application/json",
    )
    .await?;
    let bytes = match media {
        CallMedia::Got(bytes, _) => bytes,
        CallMedia::Skip(reason) => {
            tracing::warn!(
                call_id = call.id,
                reason,
                "[wa-call-transcript] media unavailable; skipping"
            );
            return Ok(());
        }
    };
    let Some((transcript, segments)) = parse_transcript(&bytes) else {
        tracing::error!(
            call_id = call.id,
            "[wa-call-transcript] malformed document; skipping"
        );
        return Ok(());
    };
    let input = AttachTranscriptInput {
        id: call.id,
        transcript: transcript.clone(),
        transcribed_at: now_ms(),
        segments: Some(Some(segments)),
    };
    if write(deps, move |c| calls::attach_transcript(c, &input))
        .await?
        .is_none()
    {
        return Ok(());
    }
    enrich(deps, &call, json!({ "hasTranscript": true })).await?;
    if let Some(contact_id) = contact_id(deps, call.contact_inbox_id).await? {
        let metadata = json!({ "callId": correlation(&call), "transcript": transcript });
        emit(
            deps,
            "callTranscribed",
            call.workspace_id,
            contact_id,
            metadata,
        )
        .await;
    }
    Ok(())
}

/// `metaTranscriptDocumentSchema` + `mapSegments` + `resolveFlatTranscript`.
fn parse_transcript(bytes: &[u8]) -> Option<(String, Value)> {
    let doc: Value = serde_json::from_slice(bytes).ok()?;
    let transcript = doc.get("transcript").filter(|t| t.is_object())?;
    let mut segments = Vec::new();
    for segment in transcript
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (true, true, Some(text)) = (
            segment["start"].is_number(),
            segment["end"].is_number(),
            segment["text"].as_str(),
        ) else {
            return None;
        };
        let mut out = Map::new();
        if let Some(speaker) = segment["speaker"].as_str() {
            out.insert("speaker".into(), json!(speaker));
        }
        if let Some(channel) = segment["channel"].as_i64() {
            out.insert("channel".into(), json!(channel));
        }
        out.insert("start".into(), segment["start"].clone());
        out.insert("end".into(), segment["end"].clone());
        out.insert("text".into(), json!(text));
        segments.push(Value::Object(out));
    }
    let text = transcript["text"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            segments
                .iter()
                .filter_map(|s| s["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_owned()
        });
    Some((text, Value::Array(segments)))
}

/// `whatsappCallRecordingReady`: the browser-recorded upload, then the
/// transcription job (`callTranscription` queue).
pub async fn recording_ready(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(id) = id_of(&data["callId"]) else {
        return Err(JobError::unrecoverable(
            "whatsappCallRecordingReady without callId",
        ));
    };
    let Some(call) = read(deps, move |c| calls::find_by_id(c, id)).await? else {
        tracing::warn!(
            call_id = id,
            "[wa-call-recording] skipped: call row not found"
        );
        return Ok(());
    };
    if call.recorded_at.is_none() {
        let path = str_of(data, "recordingPath").unwrap_or_default();
        // The path must be this call's own key (the upload route writes it).
        if !path.starts_with(&format!("space/{}/calls/{}.", call.workspace_id, call.id)) {
            return Err(JobError::unrecoverable(
                "Recording path does not belong to the call",
            ));
        }
        let size = data["sizeBytes"].as_i64();
        attach_recording_and_notify(deps, &call, path, str_of(data, "mimeType"), size).await?;
    }
    crate::inbound::add_job(
        deps,
        QueueName::CallTranscription,
        "transcribeCall",
        json!({ "type": "transcribeCall", "data": {
            "channel": "whatsapp",
            "callId": call.id.to_string(),
            "workspaceId": call.workspace_id.to_string(),
        }}),
        JobOptions {
            job_id: Some(format!("transcribe-{}", call.id)),
            ..JobOptions::default()
        },
    )
    .await
}

/// `whatsapp-call-transcribe.ts`: Whisper over the stored browser recording
/// (the `callTranscription` queue; its limiter is in the worker spec).
/// Skips without a retry when the number does not transcribe or no OpenAI
/// integration exists; the recording stays usable.
pub async fn transcribe_call(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(id) = id_of(&data["callId"]) else {
        return Err(JobError::unrecoverable("transcribeCall without callId"));
    };
    let Some(call) = read(deps, move |c| calls::find_by_id(c, id)).await? else {
        return Ok(());
    };
    let Some(path) = call.recording_path.clone() else {
        tracing::warn!(call_id = id, "[wa-call-transcript] skipped: no recording");
        return Ok(());
    };
    if call.transcript.is_some() {
        return Ok(());
    }
    let (ws, inbox_id) = (call.workspace_id, call.inbox_id);
    let integration = read(deps, move |c| {
        integration_whatsapp::find_by_inbox_id_for_workspace(c, ws, inbox_id)
    })
    .await?;
    let enabled = integration.is_some_and(|i| {
        chatbotx_core::utils::whatsapp_call::transcribes_calls(
            chatbotx_core::utils::whatsapp_call::WhatsappCallMediaToggles {
                call_recording_enabled: i.call_recording_enabled,
                call_transcription_enabled: i.call_transcription_enabled,
            },
        )
    });
    if !enabled {
        tracing::info!(
            call_id = id,
            "[wa-call-transcript] skipped: not enabled for this number"
        );
        return Ok(());
    }
    let auth = read(deps, move |c| {
        Ok(crate::ai::model::native_auth(c, ws, "openai", None)?)
    })
    .await?;
    let Some(auth) = auth else {
        tracing::info!(
            call_id = id,
            "[wa-call-transcript] skipped: no OpenAI integration"
        );
        return Ok(());
    };
    let model = crate::ai::model::native_model("openai", &auth, TRANSCRIPTION_MODEL)
        .map_err(JobError::unrecoverable)?;
    let files = deps
        .files
        .as_ref()
        .ok_or_else(|| fail("File storage is not configured"))?;
    // ponytail: reads the object directly instead of the original's signed-URL fetch.
    let audio = files
        .get_object(&path)
        .await
        .map_err(|e| fail(e.to_string()))?;
    let media_type = recording_media_type(&path);
    let text = crate::heavy::provider::transcribe_openai(&model, &audio, media_type)
        .await
        .map_err(|e| fail(e.message))?;
    if text.trim().is_empty() {
        tracing::info!(
            call_id = id,
            "[wa-call-transcript] produced empty text; not stamping"
        );
        return Ok(());
    }
    let input = AttachTranscriptInput {
        id,
        transcript: text.clone(),
        transcribed_at: now_ms(),
        segments: None,
    };
    if write(deps, move |c| calls::attach_transcript(c, &input))
        .await?
        .is_none()
    {
        return Ok(());
    }
    enrich(deps, &call, json!({ "hasTranscript": true })).await?;
    if let Some(contact_id) = contact_id(deps, call.contact_inbox_id).await? {
        let metadata = json!({ "callId": correlation(&call), "transcript": text });
        emit(
            deps,
            "callTranscribed",
            call.workspace_id,
            contact_id,
            metadata,
        )
        .await;
    }
    Ok(())
}

const TRANSCRIPTION_MODEL: &str = "whisper-1";

/// The upload type from the stored key's extension (`recording_extension` reversed).
fn recording_media_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("webm") => "audio/webm",
        Some("m4a") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        _ => "audio/ogg",
    }
}

/// The `callTranscription` queue processor.
pub async fn handle_transcription(
    deps: crate::Deps,
    job: chatbotx_jobs::JobCtx,
) -> Result<Value, JobError> {
    match str_of(&job.data, "type") {
        Some("transcribeCall") => {
            use chatbotx_business::events::context::{
                WebhookExecutionContext, with_webhook_execution_context,
            };
            with_webhook_execution_context(
                WebhookExecutionContext::webhook(),
                transcribe_call(&deps, &job.data["data"]),
            )
            .await?;
        }
        other => tracing::warn!(?other, "[call-transcription] unknown job type"),
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(
            normalize_recording_type("audio/ogg; codecs=opus"),
            "audio/ogg"
        );
        assert_eq!(normalize_recording_type("audio/wav"), "audio/ogg");
        assert_eq!(recording_media_type("space/1/calls/2.webm"), "audio/webm");
        assert_eq!(recording_media_type("space/1/calls/2.ogg"), "audio/ogg");
        assert!(is_trusted_media_url("https://lookaside.fbsbx.com/x"));
        assert!(is_trusted_media_url("https://a.graph.facebook.com/x"));
        assert!(!is_trusted_media_url("http://lookaside.fbsbx.com/x"));
        assert!(!is_trusted_media_url("https://evilfbsbx.com/x"));
        let doc = br#"{"transcript":{"segments":[{"speaker":"A","start":0,"end":1.5,"text":"hi"},{"start":2,"end":3,"text":"there"}]}}"#;
        let (text, segments) = parse_transcript(doc).unwrap();
        assert_eq!(text, "hi there");
        assert_eq!(segments[0]["speaker"], "A");
        assert!(parse_transcript(br#"{"transcript":{"segments":[{"text":"x"}]}}"#).is_none());
    }
}
