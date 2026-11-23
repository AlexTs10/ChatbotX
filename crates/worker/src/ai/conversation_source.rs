//! The conversation sources of the AI agent: the `processConversationSource`
//! and `processConversationSourceEmbedding` jobs
//! (`ai-agent/handlers/process-conversation-source{,-embedding}.ts`) and the
//! producers of the first job, the document and URL context-source adapters
//! (`system-tools/context-sources/{document,url}-source.ts`: resolve, then
//! retrieve the chunks).
//!
//! ponytail: no Readability pass on a fetched page; the page goes through
//! `html_to_text` (the original's fallback path). Add a readability crate
//! when article extraction matters.

use std::time::Duration;

use chatbotx_business::ai::knowledge;
use chatbotx_business::net::ssrf_guard::assert_public_url;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::repo::ai_conversation_embedding::{self as embeddings, EmbeddingChunkInput};
use chatbotx_db::repo::models::AiConversationSource;
use chatbotx_db::repo::{find_one, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::{JobError, JobOptions, NewJob, QueueName};
use serde_json::{Value, json};

use super::chunk::{js_trim, prose};
use super::extract::{html_to_text, text_from_bytes};
use crate::{Deps, db_fail, fail, id_of};

pub const PROCESS_CONVERSATION_SOURCE: &str = "processConversationSource";
pub const PROCESS_CONVERSATION_SOURCE_EMBEDDING: &str = "processConversationSourceEmbedding";

const MAX_DOCUMENT_TEXT_CHARS: usize = 200_000;
const MAX_DOCUMENT_CHUNKS: usize = 120;
/// `AI_FILES_DEFAULT_CHUNK_SIZE` / `AI_FILES_DEFAULT_OVERLAP_SIZE`.
const CHUNK_SIZE: i64 = 1000;
const OVERLAP_SIZE: i64 = 200;
const PARSE_TIMEOUT: Duration = Duration::from_secs(30);
const URL_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_URL_RESPONSE_BYTES: usize = 500_000;
const MAX_REDIRECTS: usize = 3;
const STALE_PROCESSING_MS: i64 = 5 * 60 * 1000;
const DEFAULT_TOP_K: i64 = 5;
const DOCUMENT_ATTACHMENT_LOOKUP_LIMIT: i64 = 50;
const MAX_SOURCE_KEY_LENGTH: usize = 500;
/// `PDF_MIME_TYPES` + `DOCX_MIME_TYPES`.
const DOCUMENT_MIME_TYPES: [&str; 3] = [
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/msword",
];

/// `isSupportedDocumentMimeType`.
fn is_supported_document(mime: &str) -> bool {
    let mime = mime.split(';').next().unwrap_or("").trim().to_lowercase();
    DOCUMENT_MIME_TYPES.contains(&mime.as_str())
}

/// JS `text.slice(0, max)`: the first `max` UTF-16 units (a split pair is dropped).
fn js_slice(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &text[..at];
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Source rows
// ---------------------------------------------------------------------------

fn find_source(
    conn: &Connection,
    id: i64,
) -> chatbotx_db::repo::Result<Option<AiConversationSource>> {
    find_one(conn, r#""id" = ?1"#, &[&id])
}

fn find_by_key(
    conn: &Connection,
    ws: i64,
    source_type: &str,
    key: &str,
) -> chatbotx_db::repo::Result<Option<AiConversationSource>> {
    find_one(
        conn,
        r#""workspaceId" = ?1 AND "sourceType" = ?2 AND "sourceKey" = ?3"#,
        &[&ws, &source_type, &key],
    )
}

fn find_latest(
    conn: &Connection,
    ws: i64,
    conv: i64,
    source_type: &str,
) -> chatbotx_db::repo::Result<Option<AiConversationSource>> {
    find_one(
        conn,
        r#""workspaceId" = ?1 AND "conversationId" = ?2 AND "sourceType" = ?3 ORDER BY "createdAt" DESC"#,
        &[&ws, &conv, &source_type],
    )
}

/// `sourceRepo.update(id, { status, errorMessage })`.
fn set_status(
    conn: &Connection,
    id: i64,
    status: &str,
    error: Option<&str>,
) -> chatbotx_db::repo::Result<()> {
    conn.execute(
        r#"UPDATE "AIConversationSource" SET "status" = ?1, "errorMessage" = ?2, "updatedAt" = ?3 WHERE "id" = ?4"#,
        params![status, error, now_ms(), id],
    )?;
    Ok(())
}

/// `createOrIgnore` + `deleteOlderByConversation` (a failed delete is only logged).
#[allow(clippy::too_many_arguments)]
fn create_or_ignore(
    conn: &Connection,
    ws: i64,
    conv: i64,
    message_id: i64,
    attachment_id: Option<i64>,
    source_type: &str,
    key: &str,
    mime: Option<&str>,
    title: Option<&str>,
    metadata: Option<String>,
) -> chatbotx_db::repo::Result<Option<AiConversationSource>> {
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "AIConversationSource" ("id", "createdAt", "updatedAt", "workspaceId", "conversationId", "messageId",
             "attachmentId", "sourceType", "status", "sourceKey", "mimeType", "title", "metadata")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9, ?10, ?11)
           ON CONFLICT ("workspaceId", "sourceType", "sourceKey") DO NOTHING"#,
        params![new_id(), now, ws, conv, message_id, attachment_id, source_type, key, mime, title, metadata],
    )?;
    let source = find_by_key(conn, ws, source_type, key)?;
    if source.is_some()
        && let Err(err) = conn.execute(
            r#"DELETE FROM "AIConversationSource"
               WHERE "workspaceId" = ?1 AND "conversationId" = ?2 AND "sourceType" = ?3 AND "sourceKey" != ?4"#,
            params![ws, conv, source_type, key],
        )
    {
        tracing::warn!(err = %err, workspace_id = ws, conversation_id = conv, "[{source_type}-source] failed to delete older {source_type} sources");
    }
    Ok(source)
}

/// Enqueues `processConversationSource` for a pending, errored or stale
/// (`processing` for over 5 min) source; a queue failure is logged.
async fn enqueue_if_due(deps: &Deps, source: &AiConversationSource) {
    let stale = source.status == "processing" && now_ms() - source.updated_at > STALE_PROCESSING_MS;
    if !(source.status == "pending" || source.status == "error" || stale) {
        return;
    }
    let data = json!({ "type": PROCESS_CONVERSATION_SOURCE, "data": { "sourceId": source.id.to_string() } });
    let opts = JobOptions {
        job_id: Some(format!(
            "{PROCESS_CONVERSATION_SOURCE}-{}-{}",
            source.id, source.updated_at
        )),
        ..JobOptions::default()
    };
    if let Err(err) = deps
        .jobs
        .add(QueueName::AiAgent, PROCESS_CONVERSATION_SOURCE, data, opts)
        .await
    {
        tracing::error!(err = %err, source_id = source.id, "[{}-source] failed to enqueue source processing", source.source_type);
    }
}

// ---------------------------------------------------------------------------
// Producers: the context-source adapters
// ---------------------------------------------------------------------------

/// `resolveDocumentSource`: the conversation's PDF/DOCX attachment that
/// matches `hint` (name, path or provider id), else the trigger message's,
/// else the newest; its source row is created (older document sources of
/// the conversation are dropped) or an errored one is set pending again.
pub async fn resolve_document_source(
    deps: &Deps,
    ws: i64,
    conv: i64,
    message_id: Option<i64>,
    hint: Option<&str>,
) -> Result<Option<AiConversationSource>, JobError> {
    let hint = hint
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty());
    let source = chatbotx_db::repo::write(&deps.db, move |tx| {
            let mut stmt = tx.prepare_cached(
                r#"SELECT "id", "messageId", "mimeType", COALESCE("name", ''), "originPath", COALESCE("sourceId", '')
                   FROM "Attachment" WHERE "workspaceId" = ?1 AND "conversationId" = ?2 AND "mimeType" IN (?3, ?4, ?5)
                   ORDER BY "createdAt" DESC LIMIT ?6"#,
            )?;
            let [pdf, docx, doc] = DOCUMENT_MIME_TYPES;
            let attachments = stmt
                .query_map(params![ws, conv, pdf, docx, doc, DOCUMENT_ATTACHMENT_LOOKUP_LIMIT], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        format!("{} {} {}", r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?)
                            .to_lowercase(),
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            let picked = hint
                .as_ref()
                .and_then(|h| attachments.iter().find(|a| a.4.contains(h.as_str())))
                .or_else(|| message_id.and_then(|m| attachments.iter().find(|a| a.1 == m)))
                .or(attachments.first());
            let Some((attachment_id, attachment_message, mime, name, _)) = picked else {
                return Ok(None);
            };
            let key = format!("attachment:{attachment_id}");
            match find_by_key(tx, ws, "document", &key)? {
                None => create_or_ignore(
                    tx,
                    ws,
                    conv,
                    *attachment_message,
                    Some(*attachment_id),
                    "document",
                    &key,
                    Some(mime),
                    Some(name).filter(|n| !n.is_empty()).map(String::as_str),
                    None,
                ),
                Some(source) if source.status == "error" && source.attachment_id.is_some() => {
                    set_status(tx, source.id, "pending", None)?;
                    find_source(tx, source.id)
                }
                found => Ok(found),
            }
        })
        .await
        .map_err(db_fail)?;
    if let Some(source) = &source {
        enqueue_if_due(deps, source).await;
    }
    Ok(source)
}

/// `resolveUrlSource`: `hint` is the URL; a new one gets a source row (older
/// URL sources of the conversation are dropped). The conversation's latest
/// URL source is returned, set pending again when it errored.
pub async fn resolve_url_source(
    deps: &Deps,
    ws: i64,
    conv: i64,
    message_id: Option<i64>,
    hint: Option<&str>,
) -> Result<Option<AiConversationSource>, JobError> {
    let Some(message_id) = message_id else {
        tracing::warn!(
            workspace_id = ws,
            conversation_id = conv,
            "[url-source] messageId is required to create url source"
        );
        return Ok(None);
    };
    let url = hint
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_owned);
    let source = chatbotx_db::repo::write(&deps.db, move |tx| {
        if let Some(url) = url {
            let key = js_slice(&format!("url:{url}"), MAX_SOURCE_KEY_LENGTH).to_owned();
            if find_by_key(tx, ws, "url", &key)?.is_none() {
                let metadata = json!({ "url": url }).to_string();
                create_or_ignore(
                    tx,
                    ws,
                    conv,
                    message_id,
                    None,
                    "url",
                    &key,
                    Some("text/html"),
                    Some(&url),
                    Some(metadata),
                )?;
            }
        }
        let Some(source) = find_latest(tx, ws, conv, "url")? else {
            return Ok(None);
        };
        if source.status == "error" {
            set_status(tx, source.id, "pending", None)?;
        }
        find_latest(tx, ws, conv, "url")
    })
    .await
    .map_err(db_fail)?;
    if let Some(source) = &source {
        enqueue_if_due(deps, source).await;
    }
    Ok(source)
}

/// One `ConversationContextSnippet` (its `source` is always `cached_embedding`).
#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    pub chunk_index: Option<i64>,
    pub content: String,
    pub similarity: Option<f64>,
}

/// `retrieveRelevantChunks` (document and URL): nothing until the source is
/// processed; a blank query lists the first chunks, else the `top_k` nearest.
pub async fn retrieve_chunks(
    deps: &Deps,
    source: &AiConversationSource,
    query: &str,
    top_k: Option<i64>,
) -> Result<Vec<Snippet>, String> {
    if source.status != "success" {
        return Ok(Vec::new());
    }
    let (id, ws, top_k) = (
        source.id,
        source.workspace_id,
        top_k.unwrap_or(DEFAULT_TOP_K),
    );
    if query.trim().is_empty() {
        let rows = chatbotx_db::repo::read(&deps.db, move |conn| {
            embeddings::find_many_by_source(conn, id, ws, top_k)
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(rows
            .into_iter()
            .map(|r| Snippet {
                chunk_index: Some(r.chunk_index),
                content: r.content,
                similarity: None,
            })
            .collect());
    }
    let model = chatbotx_db::repo::read(&deps.db, move |conn| {
        Ok(knowledge::resolve_embedding_model(conn, ws))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let vector = knowledge::embed(&*deps.ai.embedder, &model, query)
        .await
        .map_err(|e| e.message)?;
    let vector = vector_text(&vector);
    let rows = chatbotx_db::repo::read(&deps.db, move |conn| {
        embeddings::vector_search(conn, &vector, id, top_k, ws)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| Snippet {
            chunk_index: r.chunk_index,
            content: r.content,
            similarity: r.similarity,
        })
        .collect())
}

fn vector_text(vector: &[f32]) -> String {
    Value::from(vector.iter().map(|v| f64::from(*v)).collect::<Vec<_>>()).to_string()
}

// ---------------------------------------------------------------------------
// processConversationSource
// ---------------------------------------------------------------------------

/// `processConversationSource`.
pub async fn process_source(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let id = data.get("sourceId").and_then(id_of).unwrap_or_default();
    let source = chatbotx_db::repo::read(&deps.db, move |conn| find_source(conn, id))
        .await
        .map_err(db_fail)?
        .ok_or_else(|| fail("AI conversation source not found"))?;
    if source.status == "success" {
        return Ok(());
    }
    match source.source_type.as_str() {
        "document" => process_document(deps, &source).await,
        "url" => process_url(deps, &source).await,
        _ => Ok(()),
    }
}

async fn update_status(
    deps: &Deps,
    id: i64,
    status: &'static str,
    error: Option<String>,
) -> Result<(), JobError> {
    chatbotx_db::repo::write(&deps.db, move |tx| {
        set_status(tx, id, status, error.as_deref())
    })
    .await
    .map_err(db_fail)
}

async fn process_document(deps: &Deps, source: &AiConversationSource) -> Result<(), JobError> {
    let attachment = match source.attachment_id {
        Some(attachment_id) => chatbotx_db::repo::read(&deps.db, move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "mimeType", "originPath" FROM "Attachment" WHERE "id" = ?1"#,
                    [attachment_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?,
        None => None,
    };
    let Some((mime, origin_path)) = attachment else {
        let message = "Attachment not found for document source".to_owned();
        return update_status(deps, source.id, "error", Some(message)).await;
    };
    if !is_supported_document(&mime) {
        let message = "Unsupported document mime type".to_owned();
        return update_status(deps, source.id, "error", Some(message)).await;
    }
    update_status(deps, source.id, "processing", None).await?;
    // ponytail: no `ensureAttachmentStored`; Rust stores every attachment on
    // ingest, so the origin path is a storage key.
    let Some(files) = deps.files.clone() else {
        let message = "Attachment unavailable for document source".to_owned();
        return update_status(deps, source.id, "error", Some(message)).await;
    };
    let read = async {
        let bytes = files
            .get_object(&origin_path)
            .await
            .map_err(|e| e.to_string())?;
        let mime = mime.clone();
        let extract = tokio::task::spawn_blocking(move || {
            text_from_bytes(&bytes, &mime, MAX_DOCUMENT_TEXT_CHARS)
        });
        tokio::time::timeout(PARSE_TIMEOUT, extract)
            .await
            .map_err(|_| "Operation timed out".to_owned())?
            .map_err(|e| e.to_string())?
    };
    match read.await {
        Ok(text) => save_text(deps, source, &text, json!({}), "document").await,
        Err(err) => {
            tracing::error!(err = %err, source_id = source.id, conversation_id = source.conversation_id, workspace_id = source.workspace_id, "[ai-agent] processDocumentSource failed");
            let message = if err.is_empty() {
                "Failed to process conversation source".to_owned()
            } else {
                err
            };
            update_status(deps, source.id, "error", Some(message)).await
        }
    }
}

async fn process_url(deps: &Deps, source: &AiConversationSource) -> Result<(), JobError> {
    let url = source
        .metadata
        .as_ref()
        .and_then(|m| m.0.get("url"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(url) = url else {
        let message = "URL missing in source metadata".to_owned();
        return update_status(deps, source.id, "error", Some(message)).await;
    };
    update_status(deps, source.id, "processing", None).await?;
    let fetched = tokio::time::timeout(URL_FETCH_TIMEOUT, fetch_html_text(&url))
        .await
        .unwrap_or_else(|_| Err("Operation timed out".to_owned()));
    match fetched {
        Ok(text) => save_text(deps, source, &text, json!({ "url": url }), "URL").await,
        Err(err) => {
            tracing::error!(err = %err, source_id = source.id, conversation_id = source.conversation_id, workspace_id = source.workspace_id, url, "[ai-agent] processUrlSource failed");
            let message = if err.is_empty() {
                "Failed to process URL source".to_owned()
            } else {
                err
            };
            update_status(deps, source.id, "error", Some(message)).await
        }
    }
}

/// The shared success path: trim to 200k chars, chunk (at most 120), replace
/// the source's chunks, mark it `success` and queue one embedding job per
/// chunk. A failed enqueue errors the source (the original's catch).
async fn save_text(
    deps: &Deps,
    source: &AiConversationSource,
    text: &str,
    mut metadata: Value,
    what: &'static str,
) -> Result<(), JobError> {
    let trimmed = js_trim(js_slice(text, MAX_DOCUMENT_TEXT_CHARS)).to_owned();
    if trimmed.is_empty() {
        let message = format!(
            "Unable to extract readable text from {}",
            if what == "URL" { "URL" } else { "document" }
        );
        return update_status(deps, source.id, "error", Some(message)).await;
    }
    let chunks: Vec<EmbeddingChunkInput> = prose(&trimmed, CHUNK_SIZE, OVERLAP_SIZE)
        .into_iter()
        .take(MAX_DOCUMENT_CHUNKS)
        .enumerate()
        .map(|(i, content)| EmbeddingChunkInput {
            chunk_index: i as i64,
            content,
            conversation_id: source.conversation_id,
            id: new_id(),
            source_id: source.id,
            workspace_id: source.workspace_id,
        })
        .collect();
    metadata["chunkCount"] = chunks.len().into();
    metadata["maxChunks"] = MAX_DOCUMENT_CHUNKS.into();
    metadata["maxTextChars"] = MAX_DOCUMENT_TEXT_CHARS.into();
    let hash = sha256_hex(&trimmed);
    let summary = js_trim(js_slice(&trimmed, 500)).to_owned();
    let id = source.id;
    let ids = chatbotx_db::repo::write(&deps.db, move |tx| {
            let ids = embeddings::replace_for_source(tx, id, &chunks)?;
            tx.execute(
                r#"UPDATE "AIConversationSource" SET "status" = 'success', "contentHash" = ?1, "summary" = ?2,
                     "metadata" = ?3, "errorMessage" = NULL, "updatedAt" = ?4 WHERE "id" = ?5"#,
                params![hash, summary, metadata.to_string(), now_ms(), id],
            )?;
            Ok(ids)
        })
        .await
        .map_err(db_fail)?;
    if ids.is_empty() {
        return Ok(());
    }
    let jobs = ids
        .iter()
        .map(|id| NewJob {
            name: PROCESS_CONVERSATION_SOURCE_EMBEDDING.to_owned(),
            data: json!({ "type": PROCESS_CONVERSATION_SOURCE_EMBEDDING, "data": { "conversationEmbeddingId": id.to_string() } }),
            opts: JobOptions::default(),
        })
        .collect();
    if let Err(err) = deps.jobs.add_bulk(QueueName::AiAgent, jobs).await {
        tracing::error!(err = %err, source_id = id, "[ai-agent] process{what}Source failed");
        update_status(deps, id, "error", Some(err.to_string())).await?;
    }
    Ok(())
}

/// `fetchSafe` + `fetchHtmlText`: every hop is SSRF-checked (at most 3
/// redirects), only `text/html` / `text/plain` is read, and the body stops
/// after 500 KB (the rest is cut off, not an error).
async fn fetch_html_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(URL_FETCH_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut current = url.to_owned();
    let mut redirects_left = MAX_REDIRECTS;
    let mut response = loop {
        assert_public_url(&current, "URL source").await?;
        let response = client
            .get(&current)
            .header("User-Agent", "ChatbotX-URLContext/1.0")
            .header("Accept", "text/html,text/plain;q=0.9")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_redirection() {
            break response;
        }
        if redirects_left == 0 {
            return Err("Too many redirects".into());
        }
        redirects_left -= 1;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or("Redirect with no Location header")?;
        current = url::Url::parse(&current)
            .and_then(|base| base.join(location))
            .map_err(|e| e.to_string())?
            .to_string();
    };
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if !(content_type.contains("text/html") || content_type.contains("text/plain")) {
        return Err(format!("Unsupported content type: {content_type}"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_URL_RESPONSE_BYTES {
            break;
        }
    }
    Ok(html_to_text(&String::from_utf8_lossy(&body)))
}

// ---------------------------------------------------------------------------
// processConversationSourceEmbedding
// ---------------------------------------------------------------------------

/// `processConversationSourceEmbedding`: embeds one chunk; a provider error
/// marks the chunk `error` (the job does not fail).
pub async fn process_embedding(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let id = data
        .get("conversationEmbeddingId")
        .and_then(id_of)
        .unwrap_or_default();
    let row = chatbotx_db::repo::read(&deps.db, move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "status", "content", "workspaceId", "sourceId" FROM "AIConversationEmbedding" WHERE "id" = ?1"#,
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some((status, content, ws, source_id)) = row else {
        return Err(fail("AI conversation embedding not found"));
    };
    if status != "pending" && status != "processing" {
        return Ok(());
    }
    let set = move |status: &'static str, error: Option<String>, vector: Option<String>| {
        chatbotx_db::repo::write(&deps.db, move |tx| {
            tx.execute(
                r#"UPDATE "AIConversationEmbedding" SET "status" = ?1, "errorMessage" = ?2,
                     "embedding" = CASE WHEN ?3 IS NULL THEN "embedding" ELSE vector32(?3) END, "updatedAt" = ?4
                   WHERE "id" = ?5"#,
                params![status, error, vector, now_ms(), id],
            )?;
            Ok(())
        })
    };
    set("processing", None, None).await.map_err(db_fail)?;
    let embedded = async {
        let model = chatbotx_db::repo::read(&deps.db, move |conn| {
            Ok(knowledge::resolve_embedding_model(conn, ws))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        knowledge::embed(&*deps.ai.embedder, &model, &content)
            .await
            .map_err(|e| e.message)
    };
    match embedded.await {
        Ok(vector) => set("success", None, Some(vector_text(&vector)))
            .await
            .map_err(db_fail),
        Err(err) => {
            tracing::error!(err = %err, conversation_embedding_id = id, source_id, "[ai-agent] processConversationSourceEmbedding failed");
            let message = if err.is_empty() {
                "Failed to generate conversation source embedding".to_owned()
            } else {
                err
            };
            set("error", Some(message), None).await.map_err(db_fail)
        }
    }
}
