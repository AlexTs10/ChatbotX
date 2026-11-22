//! The AI conversation context (`packages/ai/src/server/cache/*`,
//! `services/ai-context-service.ts`) and the `summarizeConversation` job
//! (`apps/worker/src/ai-agent/handlers/summarize-conversation.ts`).
//!
//! The context (`ai:ctx:v1:<conversationId>`, 24 h) is a JSON value in the
//! durable kv; `runExclusive` is an in-process lock per conversation.

use std::sync::{Arc, LazyLock};

use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::Db;
use chatbotx_db::repo::conversation_ai_context::find_conversation_ai_context_state;
use chatbotx_db::repo::message::{
    FindAiContextMessagesOptions, SinceTimeInput, find_ai_context_messages, get_safe_since_time,
};
use chatbotx_db::repo::models::Message;
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::{JobError, JobOptions, QueueName, Retention};
use chatbotx_kv::lock::{LockGuard, LockMap, LockOpts};
use chatbotx_kv::{DurableKv, SystemClock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::llm::{Part, Turn};
use super::{AI_MESSAGE_HISTORY_LOOKBACK_MS, MAX_CONVERSATION_HISTORY, MAX_SUMMARY_LENGTH};
use crate::{Deps, fail, id_of, load_conversation};

/// `MAX_CONVERSATION_HISTORY_HARD_CAP`.
pub const MAX_CONVERSATION_HISTORY_HARD_CAP: usize = 150;
/// `AIJobAction.summarizeConversation`.
pub const SUMMARIZE_CONVERSATION: &str = "summarizeConversation";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
const LOCK_TIMEOUT_SECS: u64 = 10;
const HISTORY_LIMIT: usize = 20;
const SUMMARY_BATCH_SIZE: usize = 10;
const STALE_SUMMARIZING_THRESHOLD_MS: i64 = 5 * 60 * 1000;
const ROLES: [&str; 4] = ["system", "user", "assistant", "tool"];

/// `AIMessage`: `content` is a string or `text` / `file` / legacy `image`
/// parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    pub content: Value,
}

impl AiMessage {
    /// A text message before normalization.
    pub fn text(
        role: &str,
        content: &str,
        message_id: Option<String>,
        created_at: Option<i64>,
    ) -> AiMessage {
        AiMessage {
            role: role.to_owned(),
            message_id,
            created_at,
            seq: None,
            content: Value::String(content.to_owned()),
        }
    }
}

/// `AIContext`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiContext {
    pub marker_message_id: Option<String>,
    pub summary: String,
    pub history: Vec<AiMessage>,
    pub next_seq: i64,
    pub summarizing: bool,
    pub summarizing_started_at: Option<i64>,
    pub needs_resummarize: bool,
    pub updated_at: i64,
}

pub fn context_key(conversation_id: i64) -> String {
    format!("ai:ctx:v1:{conversation_id}")
}

fn kv(db: &Arc<Db>) -> DurableKv {
    DurableKv::new(Arc::clone(db), Arc::new(SystemClock))
}

static LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);

/// `aiContextStore.runExclusive` (held until the guard drops).
async fn exclusive(conversation_id: i64) -> Result<LockGuard, String> {
    LOCKS
        .acquire(
            &format!("lock:{}", context_key(conversation_id)),
            LockOpts {
                ttl_secs: LOCK_TIMEOUT_SECS,
                retry_secs: None,
            },
        )
        .await
        .map_err(|e| e.to_string())
}

/// `aiContextStore.get`: an unreadable value reads as none.
pub async fn get(db: &Arc<Db>, conversation_id: i64) -> Option<AiContext> {
    let value = kv(db)
        .get::<Value>(&context_key(conversation_id))
        .await
        .ok()??;
    let context: AiContext = serde_json::from_value(value).ok()?;
    context
        .history
        .iter()
        .all(|m| ROLES.contains(&m.role.as_str()))
        .then_some(context)
}

/// `aiContextStore.update` of the whole value (callers hold the lock).
async fn put(db: &Arc<Db>, conversation_id: i64, context: &mut AiContext) -> Result<(), String> {
    context.updated_at = now_ms();
    kv(db)
        .put(
            &context_key(conversation_id),
            &*context,
            Some(CACHE_TTL_SECS),
        )
        .await
        .map_err(|e| e.to_string())
}

/// `aiContextStore.delete`.
pub async fn delete(db: &Arc<Db>, conversation_id: i64) {
    if let Err(err) = kv(db).delete(&[&context_key(conversation_id)]).await {
        tracing::warn!(err = %err, conversation_id, "AI context delete failed");
    }
}

fn content_string(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn fallback_message_id(role: &str, content: &Value, created_at: Option<i64>) -> String {
    sha256_hex(&format!(
        "{role}:{}:{}",
        content_string(content),
        created_at.unwrap_or(0)
    ))
}

/// `isSameContextMessage`.
pub fn is_same_message(existing: &AiMessage, incoming: &AiMessage) -> bool {
    if let (Some(a), Some(b)) = (existing.seq, incoming.seq) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (&existing.message_id, &incoming.message_id) {
        return a == b;
    }
    existing.role == incoming.role
        && existing.created_at == incoming.created_at
        && content_string(&existing.content) == content_string(&incoming.content)
}

fn media_type_of_data_url(data: &str) -> Option<&str> {
    let rest = data.strip_prefix("data:")?;
    let end = rest.find([';', ','])?;
    Some(&rest[..end]).filter(|m| !m.is_empty())
}

/// `normalizeMessageForContext`: system turns are dropped, non-user parts
/// keep only text, legacy `image` parts become `file` parts.
pub fn normalize_message(message: AiMessage) -> Option<AiMessage> {
    if message.role == "system" {
        return None;
    }
    let content = match &message.content {
        Value::String(_) => message.content.clone(),
        Value::Array(parts) => {
            let mut normalized = Vec::new();
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str);
                match kind {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            normalized.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    _ if message.role != "user" => {}
                    Some("file") => {
                        if let (Some(data), Some(media_type)) = (
                            part.get("data").and_then(Value::as_str),
                            part.get("mediaType").and_then(Value::as_str),
                        ) {
                            normalized.push(
                                json!({ "type": "file", "data": data, "mediaType": media_type }),
                            );
                        }
                    }
                    Some("image") => {
                        if let Some(image) = part.get("image").and_then(Value::as_str) {
                            let media_type = part
                                .get("mimeType")
                                .and_then(Value::as_str)
                                .or_else(|| media_type_of_data_url(image))
                                .unwrap_or("image/png");
                            normalized.push(
                                json!({ "type": "file", "data": image, "mediaType": media_type }),
                            );
                        }
                    }
                    _ => {}
                }
            }
            if normalized.is_empty() {
                return None;
            }
            Value::Array(normalized)
        }
        _ => return None,
    };
    let message_id = message
        .message_id
        .unwrap_or_else(|| fallback_message_id(&message.role, &content, message.created_at));
    Some(AiMessage {
        role: message.role,
        message_id: Some(message_id),
        created_at: message.created_at,
        seq: None,
        content,
    })
}

/// `toModelUserPart` of one stored part.
fn model_user_part(part: &Value) -> Value {
    if part.get("type").and_then(Value::as_str) != Some("image") {
        return part.clone();
    }
    let image = part.get("image").and_then(Value::as_str).unwrap_or("");
    let media_type = part
        .get("mimeType")
        .and_then(Value::as_str)
        .or_else(|| media_type_of_data_url(image))
        .unwrap_or("image/png");
    json!({ "type": "file", "data": image, "mediaType": media_type })
}

/// `mapContextToModelMessages` as `(role, content)`: system and tool turns
/// are not replayed.
pub fn model_messages(history: &[AiMessage]) -> Vec<(String, Value)> {
    history
        .iter()
        .filter_map(|m| match m.role.as_str() {
            "user" => Some((
                "user".to_owned(),
                match &m.content {
                    Value::Array(parts) => {
                        Value::Array(parts.iter().map(model_user_part).collect())
                    }
                    other => other.clone(),
                },
            )),
            "assistant" => Some((
                "assistant".to_owned(),
                Value::String(content_string(&m.content)),
            )),
            _ => None,
        })
        .collect()
}

/// The model messages as model turns.
pub fn turns(history: &[AiMessage]) -> Vec<Turn> {
    model_messages(history)
        .into_iter()
        .map(|(role, content)| {
            if role == "assistant" {
                return Turn::assistant(content_string(&content));
            }
            match content {
                Value::Array(parts) => Turn::User(
                    parts
                        .iter()
                        .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                            Some("text") => Some(Part::Text(p["text"].as_str()?.to_owned())),
                            Some("file") => Some(Part::File {
                                url: p["data"].as_str()?.to_owned(),
                                media_type: p["mediaType"].as_str()?.to_owned(),
                            }),
                            _ => None,
                        })
                        .collect(),
                ),
                other => Turn::user(content_string(&other)),
            }
        })
        .collect()
}

/// The summarizer's `role: content` lines of a history.
pub fn summary_lines(history: &[AiMessage]) -> Vec<(String, String)> {
    model_messages(history)
        .into_iter()
        .map(|(role, content)| (role, content_string(&content)))
        .collect()
}

/// `mapDbMessagesToContext`: text messages of a known sender, the last 100.
pub fn map_db_messages(rows: Vec<Message>) -> Vec<AiMessage> {
    let mapped: Vec<AiMessage> = rows
        .into_iter()
        .filter(|m| m.text.as_deref().is_some_and(|t| !t.is_empty()))
        .filter(|m| {
            matches!(
                m.sender_type.as_str(),
                "bot" | "contact" | "system" | "user" | "api"
            )
        })
        .filter_map(|m| {
            let role = if m.sender_type == "contact" {
                "user"
            } else {
                "assistant"
            };
            normalize_message(AiMessage::text(
                role,
                m.text.as_deref().unwrap_or(""),
                Some(m.id.to_string()),
                Some(m.created_at),
            ))
        })
        .collect();
    let skip = mapped
        .len()
        .saturating_sub(MAX_CONVERSATION_HISTORY as usize);
    mapped.into_iter().skip(skip).collect()
}

/// `aiContextService.getOrInitContext`. `Err` = the message read failed
/// (the caller's job retries); other failures are logged and read as none.
pub async fn get_or_init(
    deps: &Deps,
    workspace_id: i64,
    conversation_id: i64,
    preferred_models: &Value,
) -> Result<Option<AiContext>, String> {
    let log = |err: &str| {
        tracing::error!(
            err,
            workspace_id,
            conversation_id,
            action = "getOrInitContext",
            "[ai-context-service] Failed to get or init AI context"
        );
    };
    let _guard = match exclusive(conversation_id).await {
        Ok(guard) => guard,
        Err(err) => {
            log(&err);
            return Ok(None);
        }
    };
    let db = &deps.db;
    let mut context = get(db, conversation_id).await;
    let state = db
        .read(move |conn| {
            find_conversation_ai_context_state(conn, conversation_id, workspace_id)
                .map_err(crate::repo_err)
        })
        .await;
    let state = match state {
        Ok(Some(state)) => state,
        Ok(None) => return Ok(None),
        Err(err) => {
            log(&err.to_string());
            return Ok(None);
        }
    };
    let marker = state.ai_context_last_message_id.map(|m| m.to_string());
    if context.as_ref().is_some_and(|c| {
        c.marker_message_id != marker || c.history.iter().any(|m| m.role == "system")
    }) {
        delete(db, conversation_id).await;
        context = None;
    }
    if let Some(context) = context {
        return Ok(Some(context));
    }
    let since = get_safe_since_time(
        state.last_activity_at.map(SinceTimeInput::Date),
        Some(AI_MESSAGE_HISTORY_LOOKBACK_MS),
    )
    .unwrap_or(0);
    let options = FindAiContextMessagesOptions {
        conversation_id,
        limit: MAX_CONVERSATION_HISTORY,
        marker_message_id: state.ai_context_last_message_id,
        message_types: Vec::new(),
        since_time: Some(since),
        text_not_null: false,
        workspace_id,
    };
    let rows = db
        .read(move |conn| find_ai_context_messages(conn, &options).map_err(crate::repo_err))
        .await
        .map_err(|err| {
            log(&err.to_string());
            err.to_string()
        })?;
    let history: Vec<AiMessage> = map_db_messages(rows)
        .into_iter()
        .enumerate()
        .map(|(i, m)| AiMessage {
            seq: Some(i as i64),
            ..m
        })
        .collect();
    let summary = if history.is_empty() {
        String::new()
    } else {
        super::summarize(
            deps,
            workspace_id,
            &summary_lines(&history),
            None,
            preferred_models,
        )
        .await
    };
    let mut next = AiContext {
        marker_message_id: marker,
        summary: summary.chars().take(MAX_SUMMARY_LENGTH).collect(),
        next_seq: history.len() as i64,
        history,
        ..AiContext::default()
    };
    if let Err(err) = put(db, conversation_id, &mut next).await {
        log(&err);
        return Ok(None);
    }
    Ok(Some(next))
}

fn summarize_job(conversation_id: i64) -> (Value, JobOptions) {
    (
        json!({ "type": SUMMARIZE_CONVERSATION, "data": { "conversationId": conversation_id.to_string() } }),
        JobOptions {
            job_id: Some(format!("summarize-{conversation_id}")),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        },
    )
}

/// `aiContextService.appendHistory`: the context after the append, none
/// when no context is cached (or on a failure, logged).
pub async fn append_history(
    deps: &Deps,
    conversation_id: i64,
    new_messages: Vec<AiMessage>,
) -> Option<AiContext> {
    let result: Result<Option<AiContext>, String> = async {
        let _guard = exclusive(conversation_id).await?;
        let db = &deps.db;
        let Some(mut context) = get(db, conversation_id).await else {
            return Ok(None);
        };
        let mut next_seq = context.next_seq;
        let mut added = false;
        for message in new_messages.into_iter().filter_map(normalize_message) {
            if !context.history.iter().any(|h| is_same_message(h, &message)) {
                context.history.push(AiMessage {
                    seq: Some(next_seq),
                    ..message
                });
                next_seq += 1;
                added = true;
            }
        }
        if !added {
            return Ok(get(db, conversation_id).await);
        }
        let length = context.history.len();
        if length > MAX_CONVERSATION_HISTORY_HARD_CAP {
            let dropped = length - MAX_CONVERSATION_HISTORY_HARD_CAP;
            tracing::error!(
                conversation_id,
                history_length_before_truncate = length,
                hard_cap = MAX_CONVERSATION_HISTORY_HARD_CAP,
                dropped_count = dropped,
                "[ai-context-service] history exceeded hard cap — the summarizer safety net already failed; truncating to protect the AI prompt from unbounded growth"
            );
            context.history.drain(..dropped);
        }
        let should_summarize = context.history.len() > MAX_CONVERSATION_HISTORY as usize;
        let in_flight = context.summarizing;
        if should_summarize && in_flight {
            tracing::warn!(
                conversation_id,
                history_length = context.history.len(),
                max_conversation_history = MAX_CONVERSATION_HISTORY,
                "[ai-context-service] history exceeds cap while a summarize job is already marked in-flight — if this repeats across messages, the 'summarizing' flag is likely stuck and history is growing unbounded"
            );
        }
        context.next_seq = next_seq;
        context.needs_resummarize = should_summarize && in_flight;
        put(db, conversation_id, &mut context).await?;
        if should_summarize && !in_flight {
            let (data, opts) = summarize_job(conversation_id);
            deps.jobs
                .add(QueueName::AiAgent, SUMMARIZE_CONVERSATION, data, opts)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(get(db, conversation_id).await)
    }
    .await;
    result.unwrap_or_else(|err| {
        tracing::error!(err = %err, conversation_id, "[ai-context-service] Failed to append history");
        None
    })
}

/// `removeSummarizedMessages`: by `seq`, legacy entries by identity.
fn remove_summarized(history: Vec<AiMessage>, summarized: &[AiMessage]) -> Vec<AiMessage> {
    history
        .into_iter()
        .filter(|h| match h.seq {
            None => !summarized
                .iter()
                .filter(|s| s.seq.is_none())
                .any(|s| is_same_message(s, h)),
            Some(seq) => !summarized.iter().any(|s| s.seq == Some(seq)),
        })
        .collect()
}

/// `handleSummarizeConversation`.
pub async fn handle_summarize(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(conversation_id) = data.get("conversationId").and_then(id_of) else {
        tracing::warn!(data = %data, "[summarizer-worker] Invalid summarize payload");
        return Ok(());
    };
    let db = &deps.db;
    let result: Result<(), String> = async {
        // Claim a batch under the lock.
        let snapshot = {
            let _guard = exclusive(conversation_id).await?;
            let Some(mut context) = get(db, conversation_id).await else {
                return Ok(());
            };
            if context.summarizing {
                let stale_for = context.summarizing_started_at.map(|at| now_ms() - at);
                if !stale_for.is_some_and(|ms| ms > STALE_SUMMARIZING_THRESHOLD_MS) {
                    context.needs_resummarize = true;
                    put(db, conversation_id, &mut context).await?;
                    return Ok(());
                }
                tracing::warn!(
                    conversation_id,
                    stale_for_ms = stale_for,
                    "[summarizer-worker] 'summarizing' flag stale for longer than a healthy summarize call can take — recovering from a likely crashed job"
                );
            }
            if context.history.len() <= HISTORY_LIMIT {
                if context.needs_resummarize || context.summarizing {
                    context.summarizing = false;
                    context.summarizing_started_at = None;
                    context.needs_resummarize = false;
                    put(db, conversation_id, &mut context).await?;
                }
                return Ok(());
            }
            let batch: Vec<AiMessage> = context.history[..SUMMARY_BATCH_SIZE].to_vec();
            let existing = context.summary.clone();
            context.summarizing = true;
            context.summarizing_started_at = Some(now_ms());
            context.needs_resummarize = false;
            put(db, conversation_id, &mut context).await?;
            (existing, batch)
        };
        let Some(conversation) = load_conversation(db, conversation_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            let _guard = exclusive(conversation_id).await?;
            if let Some(mut context) = get(db, conversation_id).await {
                context.summarizing = false;
                context.summarizing_started_at = None;
                put(db, conversation_id, &mut context).await?;
            }
            return Ok(());
        };
        let (existing, batch) = snapshot;
        let summary = super::summarize(
            deps,
            conversation.workspace_id,
            &summary_lines(&batch),
            Some(&existing),
            &Value::Null,
        )
        .await;
        let requeue = {
            let _guard = exclusive(conversation_id).await?;
            let Some(mut latest) = get(db, conversation_id).await else {
                return Ok(());
            };
            let remaining = remove_summarized(std::mem::take(&mut latest.history), &batch);
            let requeue = latest.needs_resummarize || remaining.len() > HISTORY_LIMIT;
            latest.summary = summary;
            latest.history = remaining;
            latest.summarizing = false;
            latest.summarizing_started_at = None;
            latest.needs_resummarize = false;
            put(db, conversation_id, &mut latest).await?;
            requeue
        };
        if requeue {
            let (data, opts) = summarize_job(conversation_id);
            if let Err(err) = deps
                .jobs
                .add(QueueName::AiAgent, SUMMARIZE_CONVERSATION, data, opts)
                .await
            {
                tracing::error!(err = %err, conversation_id, "[summarizer-worker] Failed to requeue follow-up summarize job after a successful summarize — will retry via next appendHistory trigger instead");
            }
        }
        Ok(())
    }
    .await;
    let Err(err) = result else {
        return Ok(());
    };
    // Never leave `summarizing` stuck on a failed attempt.
    let snapshot = match exclusive(conversation_id).await {
        Ok(_guard) => {
            let context = get(db, conversation_id).await;
            if let Some(mut reset) = context.clone() {
                reset.summarizing = false;
                reset.summarizing_started_at = None;
                if let Err(reset_err) = put(db, conversation_id, &mut reset).await {
                    tracing::error!(err = %reset_err, conversation_id, "[summarizer-worker] Failed to reset stuck 'summarizing' flag after summarize failure");
                }
            }
            context
        }
        Err(lock_err) => {
            tracing::error!(err = %lock_err, conversation_id, "[summarizer-worker] Failed to reset stuck 'summarizing' flag after summarize failure");
            None
        }
    };
    tracing::error!(
        err = %err,
        conversation_id,
        history_length_at_failure = snapshot.as_ref().map(|c| c.history.len()),
        summarizing_at_failure = snapshot.as_ref().map(|c| c.summarizing),
        "[summarizer-worker] Failed to summarize conversation"
    );
    Err(fail(err))
}
