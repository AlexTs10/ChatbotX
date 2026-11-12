//! Port of `repositories/message` (`getSafeSinceTime`, the `IMessageRepository` types) and
//! `sharding/message/repository/sharded-message-repository.ts` + `primary-key-collision.ts`
//! (P3.3).
//!
//! One SQLite file holds every message (no shards): a shard fan-out is one query, the shard
//! retry / shard-failure paths do not exist, and the distributed lock of `createOrUpdate*` is
//! not needed (the one writer serializes the write jobs). The `sinceTime` checks and the
//! `createdAt >= sinceTime` filters of the queries stay. `matchesCreatedAt` (whole-millisecond
//! range) is `"createdAt" = ?` on integer milliseconds. Every UPDATE sets `"updatedAt"` (the
//! Postgres trigger `update_updated_at_column`).

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::{Map, Value as JsonValue};

use super::models::{Attachment, Message};
use super::sql::{Sql, opt_int, opt_text};
use super::{Error, Json, Model, Result, new_id, now_ms, parse_iso};

/// `ATTACHMENT_FALLBACK_LOOKBACK_MS` (7 days).
pub const ATTACHMENT_FALLBACK_LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// `RICH_RESPONSE_FALLBACK_LOOKBACK_MS` (7 days).
pub const RICH_RESPONSE_FALLBACK_LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// `ECHO_DEDUP_LOOKBACK_MS` (24 hours): the dedup guard read looks back this far.
pub const ECHO_DEDUP_LOOKBACK_MS: i64 = 24 * 60 * 60 * 1000;
/// `bulkCreate` insert chunk size.
pub const BULK_CREATE_CHUNK_SIZE: usize = 1000;
/// `LIST_INCOMING_TEXTS_BATCH_SIZE`.
pub const LIST_INCOMING_TEXTS_BATCH_SIZE: i64 = 1000;

const HOUR_MS: i64 = 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// getSafeSinceTime
// ---------------------------------------------------------------------------

/// The `time` argument of `getSafeSinceTime` (`Date | string | number`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SinceTimeInput<'a> {
    /// A `Date` (always truthy).
    Date(i64),
    /// An ISO string (an empty string is falsy).
    Text(&'a str),
    /// A numeric timestamp (`0` is falsy).
    Ms(i64),
}

/// `getSafeSinceTime`: the start of the previous hour, or (with `buffer_ms`) the hour start
/// of `time - buffer_ms`. `None` for a falsy or invalid `time`. The hour is a UTC hour (the
/// JS `setMinutes` works in the process zone; the servers run in UTC).
pub fn get_safe_since_time(
    time: Option<SinceTimeInput<'_>>,
    buffer_ms: Option<i64>,
) -> Option<i64> {
    let ts = match time? {
        SinceTimeInput::Date(ms) => ms,
        SinceTimeInput::Ms(0) | SinceTimeInput::Text("") => return None,
        SinceTimeInput::Ms(ms) => ms,
        SinceTimeInput::Text(text) => parse_iso(text)?,
    };
    match buffer_ms {
        Some(buffer) => Some((ts - buffer).div_euclid(HOUR_MS) * HOUR_MS),
        None => Some(ts.div_euclid(HOUR_MS) * HOUR_MS - HOUR_MS),
    }
}

// ---------------------------------------------------------------------------
// Types (message-repository.ts)
// ---------------------------------------------------------------------------

/// `CreateMessageInput` (`id` / `createdAt` `None` = stamped by the repository).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreateMessageInput {
    pub id: Option<i64>,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub workspace_id: i64,
    pub content_attributes: Option<JsonValue>,
    /// `text` | `location` | `refLink`.
    pub content_type: String,
    /// `incoming` | `outgoing` | `activity`.
    pub message_type: String,
    /// `bot` | `contact` | `system` | `user` | `api`.
    pub sender_type: String,
    pub sender_id: Option<i64>,
    pub source_id: Option<String>,
    pub parent_id: Option<String>,
    pub text: Option<String>,
    /// `message` (default) | `comment`.
    pub r#type: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// `CreateMessageResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMessageResult {
    pub is_new: bool,
    pub message: Message,
}

/// `CreateAttachmentInput` / `BulkCreateAttachmentInput`. `message_id` and
/// `message_created_at` are ignored (set from the new message) by the `create*WithAttachments`
/// paths; `id` / `created_at` `None` = stamped by the repository.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreateAttachmentInput {
    pub id: Option<i64>,
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub message_id: i64,
    pub message_created_at: i64,
    /// `image` | `video` | `audio` | `gif` | `file`.
    pub file_type: String,
    pub mime_type: String,
    pub origin_path: String,
    pub created_at: Option<i64>,
    pub height: Option<i64>,
    pub width: Option<i64>,
    pub keep_original: Option<bool>,
    pub name: Option<String>,
    pub size: Option<i64>,
    pub source_id: Option<String>,
    pub thumbnail_path: Option<String>,
}

/// `MessageWithAttachments`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageWithAttachments {
    pub message: Message,
    pub attachment_count: Option<i64>,
    pub attachments: Vec<Attachment>,
}

/// `{ result, isNew }` of `createOrUpdateWithAttachments`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateWithAttachmentsResult {
    pub result: MessageWithAttachments,
    pub is_new: bool,
}

/// `{ id, sourceId }` of `bulkCreate`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedMessageRef {
    pub id: i64,
    pub source_id: Option<String>,
}

/// `{ conversationId, id }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MessageRef {
    pub conversation_id: i64,
    pub id: i64,
}

/// `PaginationCursor` (`id` `None` = the TS empty-string id: `createdAt <` only).
#[derive(Debug, Clone, PartialEq)]
pub struct PaginationCursor {
    pub created_at: i64,
    pub id: Option<i64>,
    pub shard_id: Option<String>,
}

/// `Pagination`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pagination {
    pub cursor: Option<PaginationCursor>,
    pub limit: i64,
}

/// `PaginatedMessages`.
#[derive(Debug, Clone, PartialEq)]
pub struct PaginatedMessages {
    pub data: Vec<MessageWithAttachments>,
    pub has_partial_results: Option<bool>,
    pub next_cursor: Option<PaginationCursor>,
}

/// `ListMessagesQuery`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListMessagesQuery {
    pub contact_inbox_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub pagination: Pagination,
    pub since_time: Option<i64>,
    pub workspace_id: i64,
}

/// `FindLastByConversationOptions`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FindLastByConversationOptions {
    pub attachment_count_only: bool,
    pub limit: Option<i64>,
    pub message_types: Vec<String>,
    pub require_complete_results: bool,
    pub since_time: Option<i64>,
    pub with_attachments: bool,
    pub workspace_id: Option<i64>,
}

/// `FindManyByConversationOptions`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FindManyByConversationOptions {
    pub limit: i64,
    pub message_types: Vec<String>,
    pub require_complete_results: bool,
    pub since_time: Option<i64>,
    pub text_not_null: bool,
    pub workspace_id: Option<i64>,
}

/// `FindAIContextMessagesOptions`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FindAiContextMessagesOptions {
    pub conversation_id: i64,
    pub limit: i64,
    pub marker_message_id: Option<i64>,
    pub message_types: Vec<String>,
    pub since_time: Option<i64>,
    pub text_not_null: bool,
    pub workspace_id: i64,
}

/// `FindTriggerMessageOptions`.
#[derive(Debug, Clone, PartialEq)]
pub struct FindTriggerMessageOptions {
    pub conversation_id: i64,
    pub id: i64,
    pub require_complete_results: bool,
    pub since_time: i64,
    pub workspace_id: i64,
}

/// `FindRichResponseByButtonParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct FindRichResponseByButtonParams {
    pub button_id: String,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub message_id: Option<i64>,
    pub since_time: Option<i64>,
    pub workspace_id: i64,
}

/// `FindMessageByIdParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct FindMessageByIdParams {
    pub conversation_id: Option<i64>,
    pub created_at: i64,
    pub id: i64,
    pub workspace_id: i64,
}

/// `FindManyBySourceIdsParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct FindManyBySourceIdsParams {
    pub contact_inbox_ids: Vec<i64>,
    pub since_time: Option<i64>,
    pub source_ids: Vec<String>,
    pub workspace_id: i64,
}

/// `HardDeleteAllByContactInboxParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct HardDeleteAllByContactInboxParams {
    pub contact_inbox_id: i64,
    pub since_time: i64,
    pub workspace_id: i64,
}

/// `HardDeleteAllByContactInboxResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct HardDeleteAllByContactInboxResult {
    pub attachment_paths: Vec<String>,
}

/// `ListIncomingTextsByContactInboxParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListIncomingTextsByContactInboxParams {
    pub contact_inbox_id: i64,
    pub limit: Option<i64>,
    pub since_time: i64,
    pub workspace_id: i64,
}

/// `MessageSourceRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageSourceRow {
    pub id: i64,
    pub conversation_id: i64,
    pub contact_inbox_id: i64,
    pub source_id: Option<String>,
    pub created_at: i64,
}

/// `MessageTextWithFileTypes`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageTextWithFileTypes {
    pub id: i64,
    pub text: Option<String>,
    pub attachment_file_types: Vec<String>,
}

/// One patch of `BulkPatchContentAttributesParams` (`text` `None` = not set).
#[derive(Debug, Clone, PartialEq)]
pub struct ContentAttributesPatch {
    pub contact_inbox_id: i64,
    pub overlay: Map<String, JsonValue>,
    pub source_id: String,
    pub text: Option<Option<String>>,
}

/// `BulkPatchContentAttributesParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkPatchContentAttributesParams {
    pub patches: Vec<ContentAttributesPatch>,
    pub since_time: Option<i64>,
    pub workspace_id: i64,
}

/// Params of `claimContentAttributes`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimContentAttributesParams {
    pub message_id: i64,
    pub workspace_id: i64,
    pub created_at: i64,
    pub guard_key: String,
    pub overlay: Map<String, JsonValue>,
}

/// `FindAttachmentByIdParams` (`message_created_at` is a shard hint; one file holds every
/// row, so it is not used).
#[derive(Debug, Clone, PartialEq)]
pub struct FindAttachmentByIdParams {
    pub id: i64,
    pub message_created_at: Option<i64>,
    pub workspace_id: i64,
}

/// `AttachmentLookupRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentLookupRow {
    pub id: i64,
    pub message_id: i64,
    pub message_created_at: i64,
    pub source_id: Option<String>,
    pub origin_path: String,
    pub mime_type: String,
    pub size: i64,
    pub created_at: i64,
    pub keep_original: Option<bool>,
}

/// `UpdateAttachmentParams.fields`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateAttachmentFields {
    pub height: Option<i64>,
    pub mime_type: String,
    pub origin_path: String,
    pub size: i64,
    pub width: Option<i64>,
}

/// `UpdateAttachmentParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateAttachmentParams {
    pub created_at: i64,
    pub fields: UpdateAttachmentFields,
    pub id: i64,
    pub workspace_id: i64,
}

/// Params of `hasAttachmentOfType`.
#[derive(Debug, Clone, PartialEq)]
pub struct HasAttachmentOfTypeParams {
    pub workspace_id: i64,
    pub message_id: i64,
    pub message_created_at: i64,
    pub file_type: String,
}

/// The `patch` of `updateContentBySourceId` (outer `None` = key not set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContentBySourcePatch {
    pub text: Option<Option<String>>,
    pub content_attributes: Option<Option<JsonValue>>,
}

/// `{ id, contentAttributes }` of `mergeContentAttributesBySourceId`.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedContentAttributes {
    pub id: i64,
    pub content_attributes: Option<JsonValue>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `MessageShardUnavailableError`.
fn shard_unavailable(message: &str) -> Error {
    Error::Coded {
        kind: "MessageShardUnavailableError",
        code: "MessageShardUnavailableError".to_owned(),
        message: message.to_owned(),
    }
}

fn since_required(method: &str) -> Error {
    Error::msg(format!(
        "sinceTime is required for {method} in sharded repository"
    ))
}

/// `isMessagePrimaryKeyViolation`.
pub fn is_message_primary_key_violation(error: &Error) -> bool {
    error.as_db().is_some_and(|e| {
        e.is_unique_violation() && e.constraint().is_some_and(|c| c.ends_with("Message_pkey"))
    })
}

fn json_opt_text(value: &Option<JsonValue>) -> Result<Value> {
    Ok(match value {
        None | Some(JsonValue::Null) => Value::Null,
        Some(v) => Value::Text(super::json_text(v)?),
    })
}

/// `COALESCE(col, '{}'::jsonb) || overlay` (shallow merge; explicit `null` kept).
fn merge_object_sql(column: &str, overlay: &Map<String, JsonValue>) -> Result<Sql> {
    let mut q = Sql::raw(format!("json_set(COALESCE(\"{column}\", '{{}}')"));
    for (key, value) in overlay {
        q.push(", ");
        q.push_bind(format!("$.\"{key}\""));
        q.push(", json(");
        q.push_bind(super::json_text(value)?);
        q.push(")");
    }
    q.push(")");
    Ok(q)
}

fn ids_json(ids: &[i64]) -> String {
    super::id_list(ids)
}

fn js_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(b) => *b,
        JsonValue::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        JsonValue::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn message_sql(tail: &str) -> String {
    format!("{} {tail}", Message::select_sql())
}

fn with_created_at(message: &CreateMessageInput) -> CreateMessageInput {
    let mut out = message.clone();
    if out.created_at.is_none() {
        out.created_at = Some(now_ms());
    }
    out
}

/// One INSERT of `message` (`ON CONFLICT (contactInboxId, sourceId, createdAt) DO NOTHING`
/// when `ignore_conflict`). `None` = the dedup index skipped it.
fn insert_message(
    conn: &Connection,
    message: &CreateMessageInput,
    ignore_conflict: bool,
) -> Result<Option<Message>> {
    let now = now_ms();
    let mut q = Sql::raw(
        "INSERT INTO \"Message\" (\"id\", \"createdAt\", \"updatedAt\", \"conversationId\", \
         \"contactInboxId\", \"workspaceId\", \"text\", \"contentAttributes\", \"messageType\", \
         \"contentType\", \"senderType\", \"senderId\", \"sourceId\", \"type\", \"parentId\") VALUES (",
    );
    let values: Vec<Value> = vec![
        Value::Integer(message.id.unwrap_or_else(new_id)),
        Value::Integer(message.created_at.unwrap_or(now)),
        Value::Integer(message.updated_at.unwrap_or(now)),
        Value::Integer(message.conversation_id),
        Value::Integer(message.contact_inbox_id),
        Value::Integer(message.workspace_id),
        opt_text(message.text.as_deref()),
        json_opt_text(&message.content_attributes)?,
        Value::Text(message.message_type.clone()),
        Value::Text(message.content_type.clone()),
        Value::Text(message.sender_type.clone()),
        opt_int(message.sender_id),
        opt_text(message.source_id.as_deref()),
        Value::Text(
            message
                .r#type
                .clone()
                .unwrap_or_else(|| "message".to_owned()),
        ),
        opt_text(message.parent_id.as_deref()),
    ];
    for (i, v) in values.into_iter().enumerate() {
        if i > 0 {
            q.push(", ");
        }
        q.push_bind(v);
    }
    q.push(")");
    if ignore_conflict {
        q.push(" ON CONFLICT (\"contactInboxId\", \"sourceId\", \"createdAt\") DO NOTHING");
    }
    q.push(&format!(" RETURNING {}", Message::columns_sql(None)));
    q.model::<Message>(conn)
}

/// `withPrimaryKeyCollisionRecovery`: stamps `createdAt` once; on a `Message_pkey` violation
/// of the first attempt it retries once with a fresh id (same `createdAt`).
fn with_primary_key_collision_recovery<T>(
    message: &CreateMessageInput,
    mut insert_once: impl FnMut(&CreateMessageInput) -> Result<T>,
) -> Result<T> {
    let mut attempt = with_created_at(message);
    if attempt.id.is_none() {
        attempt.id = Some(new_id());
    }
    match insert_once(&attempt) {
        Err(error) if is_message_primary_key_violation(&error) => {
            let retry = CreateMessageInput {
                id: Some(new_id()),
                ..attempt.clone()
            };
            tracing::warn!(
                err = %error,
                collidingId = ?attempt.id,
                retryId = ?retry.id,
                conversationId = attempt.conversation_id,
                sourceId = ?attempt.source_id,
                workspaceId = attempt.workspace_id,
                "Message primary key collision — retrying insert with a fresh id"
            );
            insert_once(&retry)
        }
        other => other,
    }
}

fn insert_attachment(conn: &Connection, a: &CreateAttachmentInput) -> Result<Attachment> {
    let now = now_ms();
    let values: Vec<Value> = vec![
        Value::Integer(a.id.unwrap_or_else(new_id)),
        Value::Integer(a.created_at.unwrap_or(now)),
        Value::Integer(now),
        Value::Integer(a.workspace_id),
        Value::Integer(a.conversation_id),
        Value::Integer(a.message_id),
        Value::Integer(a.message_created_at),
        Value::Text(a.file_type.clone()),
        opt_text(a.source_id.as_deref()),
        Value::Text(a.mime_type.clone()),
        opt_int(a.width),
        opt_int(a.height),
        Value::Integer(a.size.unwrap_or(0)),
        opt_text(a.thumbnail_path.as_deref()),
        Value::Text(a.origin_path.clone()),
        opt_text(a.name.as_deref()),
        a.keep_original
            .map_or(Value::Null, |b| Value::Integer(i64::from(b))),
    ];
    let text = format!(
        "INSERT INTO \"Attachment\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \
         \"conversationId\", \"messageId\", \"messageCreatedAt\", \"fileType\", \"sourceId\", \
         \"mimeType\", \"width\", \"height\", \"size\", \"thumbnailPath\", \"originPath\", \"name\", \
         \"keepOriginal\") VALUES ({}) RETURNING {}",
        super::placeholders(1, values.len()),
        Attachment::columns_sql(None)
    );
    Sql::new(text, values)
        .model::<Attachment>(conn)?
        .ok_or_else(|| Error::msg("attachment insert returned no row"))
}

/// `queryAttachmentsForMessages` with the `(messageId, messageCreatedAt)` pairs.
fn attachments_for(conn: &Connection, messages: &[(i64, i64)]) -> Result<Vec<Attachment>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let pairs: Vec<JsonValue> = messages
        .iter()
        .map(|(id, at)| serde_json::json!([id, at]))
        .collect();
    let text = format!(
        "{} WHERE (\"messageId\", \"messageCreatedAt\") IN \
         (SELECT value ->> 0, value ->> 1 FROM json_each(?1)) ORDER BY \"createdAt\", \"id\"",
        Attachment::select_sql()
    );
    Sql::new(text, vec![Value::Text(JsonValue::Array(pairs).to_string())])
        .models::<Attachment>(conn)
}

fn with_attachments(
    conn: &Connection,
    messages: Vec<Message>,
) -> Result<Vec<MessageWithAttachments>> {
    let keys: Vec<(i64, i64)> = messages.iter().map(|m| (m.id, m.created_at)).collect();
    let attachments = attachments_for(conn, &keys)?;
    Ok(messages
        .into_iter()
        .map(|message| {
            let own = attachments
                .iter()
                .filter(|a| a.message_id == message.id)
                .cloned()
                .collect();
            MessageWithAttachments {
                message,
                attachment_count: None,
                attachments: own,
            }
        })
        .collect())
}

fn with_attachment_counts(
    conn: &Connection,
    messages: Vec<Message>,
) -> Result<Vec<MessageWithAttachments>> {
    let keys: Vec<(i64, i64)> = messages.iter().map(|m| (m.id, m.created_at)).collect();
    let attachments = attachments_for(conn, &keys)?;
    Ok(messages
        .into_iter()
        .map(|message| {
            let count = attachments
                .iter()
                .filter(|a| a.message_id == message.id)
                .count() as i64;
            MessageWithAttachments {
                message,
                attachment_count: Some(count),
                attachments: Vec::new(),
            }
        })
        .collect())
}

fn bare(messages: Vec<Message>) -> Vec<MessageWithAttachments> {
    messages
        .into_iter()
        .map(|message| MessageWithAttachments {
            message,
            attachment_count: None,
            attachments: Vec::new(),
        })
        .collect()
}

fn push_types(q: &mut Sql, types: &[String]) {
    if types.is_empty() {
        return;
    }
    q.push(" AND \"messageType\" IN (SELECT value FROM json_each(");
    q.push_bind(serde_json::to_string(types).unwrap_or_else(|_| "[]".to_owned()));
    q.push("))");
}

/// `updateAcrossShards`: `UPDATE ... SET <set>, "updatedAt" WHERE id AND workspace AND createdAt
/// [AND extra] RETURNING conversationId, id` (first row).
fn update_by_key(
    conn: &Connection,
    message_id: i64,
    workspace_id: i64,
    created_at: i64,
    set: Sql,
    extra_where: Option<Sql>,
) -> Result<Option<MessageRef>> {
    let mut q = Sql::raw("UPDATE \"Message\" SET ");
    q.append(set);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(message_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"createdAt\" = ");
    q.push_bind(created_at);
    if let Some(extra) = extra_where {
        q.push(" AND ");
        q.append(extra);
    }
    q.push(" RETURNING \"conversationId\", \"id\"");
    let rows = q.query_map(conn, |r| {
        Ok(MessageRef {
            conversation_id: r.get(0)?,
            id: r.get(1)?,
        })
    })?;
    Ok(rows.into_iter().next())
}

fn set_value(column: &str, value: Value) -> Sql {
    Sql::bind(format!("\"{column}\" = "), value)
}

/// `patchBySourceId`: no time bound in the UPDATE (the 90-day window only chose shards).
fn patch_by_source_id(
    conn: &Connection,
    source_id: &str,
    workspace_id: i64,
    set: Sql,
) -> Result<Option<MessageRef>> {
    let mut q = Sql::raw("UPDATE \"Message\" SET ");
    q.append(set);
    q.push(" WHERE \"sourceId\" = ");
    q.push_bind(source_id.to_owned());
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" RETURNING \"conversationId\", \"id\"");
    let rows = q.query_map(conn, |r| {
        Ok(MessageRef {
            conversation_id: r.get(0)?,
            id: r.get(1)?,
        })
    })?;
    Ok(rows.into_iter().next())
}

/// `findOnWriteShardBySource`: the row the dedup conflict proved exists.
fn find_on_write_by_source(
    conn: &Connection,
    message: &CreateMessageInput,
) -> Result<Option<Message>> {
    let (Some(source_id), Some(created_at)) = (message.source_id.as_deref(), message.created_at)
    else {
        return Ok(None);
    };
    Sql::new(
        message_sql(
            "WHERE \"contactInboxId\" = ?1 AND \"sourceId\" = ?2 AND \"createdAt\" = ?3 LIMIT 1",
        ),
        vec![
            Value::Integer(message.contact_inbox_id),
            Value::Text(source_id.to_owned()),
            Value::Integer(created_at),
        ],
    )
    .model::<Message>(conn)
}

/// The input as a row (the last-resort result of an unreadable dedup conflict).
fn message_from_input(message: &CreateMessageInput) -> Message {
    let now = now_ms();
    Message {
        id: message.id.unwrap_or_default(),
        created_at: message.created_at.unwrap_or(now),
        updated_at: message.updated_at.unwrap_or(now),
        conversation_id: message.conversation_id,
        contact_inbox_id: message.contact_inbox_id,
        workspace_id: message.workspace_id,
        text: message.text.clone(),
        content_attributes: message.content_attributes.clone().map(Json),
        message_type: message.message_type.clone(),
        content_type: message.content_type.clone(),
        sender_type: message.sender_type.clone(),
        sender_id: message.sender_id,
        source_id: message.source_id.clone(),
        deleted_at: None,
        r#type: message
            .r#type
            .clone()
            .unwrap_or_else(|| "message".to_owned()),
        parent_id: message.parent_id.clone(),
        attributes: None,
        send_error: None,
    }
}

fn log_save_failure(action: &str, message: &CreateMessageInput, error: &Error) {
    let db = error.as_db();
    tracing::error!(
        err = %error,
        causeCode = ?db.and_then(|e| e.sqlstate()),
        causeConstraint = ?db.and_then(|e| e.constraint()),
        action,
        conversationId = message.conversation_id,
        sourceId = ?message.source_id,
        workspaceId = message.workspace_id,
        "Failed to save incoming message via {action}"
    );
}

fn dedup_since(message: &CreateMessageInput) -> Option<i64> {
    get_safe_since_time(
        message.created_at.map(SinceTimeInput::Date),
        Some(ECHO_DEDUP_LOOKBACK_MS),
    )
}

fn source_key(message: &CreateMessageInput) -> Option<&str> {
    message.source_id.as_deref().filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// `create`.
pub fn create(conn: &Connection, message: &CreateMessageInput) -> Result<Message> {
    with_primary_key_collision_recovery(message, |input| {
        insert_message(conn, input, false)?
            .ok_or_else(|| Error::msg("message insert returned no row"))
    })
}

/// `bulkCreate`: chunks of 1000, `ON CONFLICT (contactInboxId, sourceId, createdAt) DO NOTHING`.
pub fn bulk_create(
    conn: &Connection,
    messages: &[CreateMessageInput],
) -> Result<Vec<CreatedMessageRef>> {
    let Some(first) = messages.first() else {
        return Ok(Vec::new());
    };
    if messages
        .iter()
        .any(|m| m.workspace_id != first.workspace_id)
    {
        return Err(Error::msg(
            "bulkCreate: all messages must belong to the same workspace",
        ));
    }
    let mut inserted = Vec::new();
    for chunk in messages.chunks(BULK_CREATE_CHUNK_SIZE) {
        for message in chunk {
            if let Some(row) = insert_message(conn, &with_created_at(message), true)? {
                inserted.push(CreatedMessageRef {
                    id: row.id,
                    source_id: row.source_id,
                });
            }
        }
    }
    Ok(inserted)
}

/// `bulkCreateAttachments` (returns the ids).
pub fn bulk_create_attachments(
    conn: &Connection,
    attachments: &[CreateAttachmentInput],
) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        ids.push(insert_attachment(conn, attachment)?.id);
    }
    Ok(ids)
}

fn create_with_attachments_internal(
    conn: &Connection,
    message: &CreateMessageInput,
    attachments: &[CreateAttachmentInput],
    ignore_conflict: bool,
) -> Result<Option<MessageWithAttachments>> {
    let Some(new_message) = insert_message(conn, message, ignore_conflict)? else {
        return Ok(None);
    };
    let mut rows = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let mut value = attachment.clone();
        value.created_at = Some(attachment.created_at.unwrap_or_else(now_ms));
        value.message_id = new_message.id;
        value.message_created_at = new_message.created_at;
        rows.push(insert_attachment(conn, &value)?);
    }
    Ok(Some(MessageWithAttachments {
        message: new_message,
        attachment_count: None,
        attachments: rows,
    }))
}

/// `createWithAttachments`.
pub fn create_with_attachments(
    conn: &Connection,
    message: &CreateMessageInput,
    attachments: &[CreateAttachmentInput],
) -> Result<MessageWithAttachments> {
    with_primary_key_collision_recovery(message, |input| {
        create_with_attachments_internal(conn, input, attachments, false)?
            .ok_or_else(|| shard_unavailable("createWithAttachments: insert returned no row"))
    })
}

/// `createOrUpdate`: dedup guard read, idempotent insert, re-read on a dedup conflict.
pub fn create_or_update(
    conn: &Connection,
    message: &CreateMessageInput,
) -> Result<CreateMessageResult> {
    let Some(source_id) = source_key(message) else {
        return Ok(CreateMessageResult {
            message: create(conn, message)?,
            is_new: true,
        });
    };
    let since = dedup_since(message);
    if let Some(existing) = find_by_source_id(
        conn,
        source_id,
        message.conversation_id,
        message.workspace_id,
        since,
    )? {
        return Ok(CreateMessageResult {
            message: existing,
            is_new: false,
        });
    }
    let created =
        with_primary_key_collision_recovery(message, |input| insert_message(conn, input, true));
    let created = match created {
        Ok(v) => v,
        Err(error) => {
            log_save_failure("createOrUpdate", message, &error);
            return Err(error);
        }
    };
    if let Some(created) = created {
        return Ok(CreateMessageResult {
            message: created,
            is_new: true,
        });
    }
    let raced = match find_by_source_id(
        conn,
        source_id,
        message.conversation_id,
        message.workspace_id,
        since,
    )? {
        Some(row) => Some(row),
        None => find_on_write_by_source(conn, message)?,
    };
    tracing::info!(
        conversationId = message.conversation_id,
        sourceId = source_id,
        workspaceId = message.workspace_id,
        found = raced.is_some(),
        "Duplicate message skipped (dedup conflict)"
    );
    match raced {
        Some(row) => Ok(CreateMessageResult {
            message: row,
            is_new: false,
        }),
        None => {
            tracing::warn!(
                conversationId = message.conversation_id,
                sourceId = source_id,
                workspaceId = message.workspace_id,
                "Dedup conflict but row unreadable even on the write shard"
            );
            Ok(CreateMessageResult {
                message: message_from_input(message),
                is_new: false,
            })
        }
    }
}

/// `createOrUpdateWithAttachments`.
pub fn create_or_update_with_attachments(
    conn: &Connection,
    message: &CreateMessageInput,
    attachments: &[CreateAttachmentInput],
) -> Result<CreateWithAttachmentsResult> {
    let Some(source_id) = source_key(message) else {
        let created = with_primary_key_collision_recovery(message, |input| {
            create_with_attachments_internal(conn, input, attachments, false)?.ok_or_else(|| {
                shard_unavailable("createOrUpdateWithAttachments: insert returned no row")
            })
        })?;
        return Ok(CreateWithAttachmentsResult {
            result: created,
            is_new: true,
        });
    };
    let build_existing = |existing: Message| -> Result<CreateWithAttachmentsResult> {
        let found = find_by_id(
            conn,
            &FindMessageByIdParams {
                conversation_id: None,
                created_at: existing.created_at,
                id: existing.id,
                workspace_id: message.workspace_id,
            },
        )?;
        Ok(CreateWithAttachmentsResult {
            result: found.unwrap_or(MessageWithAttachments {
                message: existing,
                attachment_count: None,
                attachments: Vec::new(),
            }),
            is_new: false,
        })
    };
    let since = dedup_since(message);
    if let Some(existing) = find_by_source_id(
        conn,
        source_id,
        message.conversation_id,
        message.workspace_id,
        since,
    )? {
        return build_existing(existing);
    }
    let created = with_primary_key_collision_recovery(message, |input| {
        create_with_attachments_internal(conn, input, attachments, true)
    });
    let created = match created {
        Ok(v) => v,
        Err(error) => {
            log_save_failure("createOrUpdateWithAttachments", message, &error);
            return Err(error);
        }
    };
    if let Some(created) = created {
        return Ok(CreateWithAttachmentsResult {
            result: created,
            is_new: true,
        });
    }
    let raced = match find_by_source_id(
        conn,
        source_id,
        message.conversation_id,
        message.workspace_id,
        since,
    )? {
        Some(row) => Some(row),
        None => find_on_write_by_source(conn, message)?,
    };
    tracing::info!(
        conversationId = message.conversation_id,
        sourceId = source_id,
        workspaceId = message.workspace_id,
        found = raced.is_some(),
        "Duplicate message skipped (dedup conflict)"
    );
    match raced {
        Some(row) => build_existing(row),
        None => {
            tracing::warn!(
                conversationId = message.conversation_id,
                sourceId = source_id,
                workspaceId = message.workspace_id,
                "Dedup conflict but row unreadable even on the write shard"
            );
            Ok(CreateWithAttachmentsResult {
                result: MessageWithAttachments {
                    message: message_from_input(message),
                    attachment_count: None,
                    attachments: Vec::new(),
                },
                is_new: false,
            })
        }
    }
}

/// `updateContentAttributes`.
pub fn update_content_attributes(
    conn: &Connection,
    message_id: i64,
    workspace_id: i64,
    content_attributes: &JsonValue,
    created_at: i64,
) -> Result<Option<i64>> {
    let set = set_value(
        "contentAttributes",
        json_opt_text(&Some(content_attributes.clone()))?,
    );
    Ok(update_by_key(conn, message_id, workspace_id, created_at, set, None)?.map(|r| r.id))
}

/// `updateMessageAttributes` (`{ liked, hidden }`).
pub fn update_message_attributes(
    conn: &Connection,
    message_id: i64,
    workspace_id: i64,
    liked: bool,
    hidden: bool,
    created_at: i64,
) -> Result<Option<i64>> {
    let attributes = serde_json::json!({ "liked": liked, "hidden": hidden });
    let set = set_value("attributes", Value::Text(attributes.to_string()));
    Ok(update_by_key(conn, message_id, workspace_id, created_at, set, None)?.map(|r| r.id))
}

/// `updateMessageText`.
pub fn update_message_text(
    conn: &Connection,
    message_id: i64,
    workspace_id: i64,
    new_text: &str,
    created_at: i64,
) -> Result<Option<MessageRef>> {
    let set = set_value("text", Value::Text(new_text.to_owned()));
    update_by_key(conn, message_id, workspace_id, created_at, set, None)
}

/// `updateTextBySourceId`.
pub fn update_text_by_source_id(
    conn: &Connection,
    source_id: &str,
    workspace_id: i64,
    new_text: &str,
) -> Result<Option<MessageRef>> {
    let mut set = set_value("text", Value::Text(new_text.to_owned()));
    set.push(", \"updatedAt\" = ");
    set.push_bind(now_ms());
    patch_by_source_id(conn, source_id, workspace_id, set)
}

/// `updateContentBySourceId` (an empty patch still bumps `updatedAt`).
pub fn update_content_by_source_id(
    conn: &Connection,
    source_id: &str,
    workspace_id: i64,
    patch: &ContentBySourcePatch,
) -> Result<Option<i64>> {
    let mut set = Sql::bind("\"updatedAt\" = ", now_ms());
    if let Some(text) = &patch.text {
        set.push(", \"text\" = ");
        set.push_bind(opt_text(text.as_deref()));
    }
    if let Some(ca) = &patch.content_attributes {
        set.push(", \"contentAttributes\" = ");
        set.push_bind(json_opt_text(ca)?);
    }
    Ok(patch_by_source_id(conn, source_id, workspace_id, set)?.map(|r| r.id))
}

/// `mergeContentAttributesBySourceId`: one jsonb-merge UPDATE (never a whole-column overwrite).
pub fn merge_content_attributes_by_source_id(
    conn: &Connection,
    source_id: &str,
    workspace_id: i64,
    overlay: &Map<String, JsonValue>,
) -> Result<Option<MergedContentAttributes>> {
    let mut q = Sql::raw("UPDATE \"Message\" SET \"contentAttributes\" = ");
    q.append(merge_object_sql("contentAttributes", overlay)?);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"sourceId\" = ");
    q.push_bind(source_id.to_owned());
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" RETURNING \"id\", \"contentAttributes\"");
    let rows = q.query_map(conn, |r| {
        let ca: Option<Json> = r.get(1)?;
        Ok(MergedContentAttributes {
            id: r.get(0)?,
            content_attributes: ca.map(|j| j.0),
        })
    })?;
    Ok(rows.into_iter().next())
}

/// `claimContentAttributes`: merges `overlay` only while `contentAttributes -> guardKey` is
/// NULL (SQL NULL or a missing key; a stored JSON `null` is not NULL in Postgres either).
pub fn claim_content_attributes(
    conn: &Connection,
    params: &ClaimContentAttributesParams,
) -> Result<Option<i64>> {
    let mut set = Sql::raw("\"contentAttributes\" = ");
    set.append(merge_object_sql("contentAttributes", &params.overlay)?);
    let guard = Sql::bind(
        "json_type(\"contentAttributes\", '$.\"' || ",
        Value::Text(params.guard_key.clone()),
    )
    .closed(" || '\"') IS NULL");
    Ok(update_by_key(
        conn,
        params.message_id,
        params.workspace_id,
        params.created_at,
        set,
        Some(guard),
    )?
    .map(|r| r.id))
}

/// `bulkPatchContentAttributes` (needs `sinceTime`).
pub fn bulk_patch_content_attributes(
    conn: &Connection,
    params: &BulkPatchContentAttributesParams,
) -> Result<()> {
    if params.patches.is_empty() {
        return Ok(());
    }
    let Some(since) = params.since_time else {
        return Err(since_required("bulkPatchContentAttributes"));
    };
    for patch in &params.patches {
        let mut q = Sql::raw("UPDATE \"Message\" SET ");
        if let Some(text) = &patch.text {
            q.push("\"text\" = ");
            q.push_bind(opt_text(text.as_deref()));
            q.push(", ");
        }
        q.push("\"contentAttributes\" = ");
        q.append(merge_object_sql("contentAttributes", &patch.overlay)?);
        q.push(", \"updatedAt\" = ");
        q.push_bind(now_ms());
        q.push(" WHERE \"workspaceId\" = ");
        q.push_bind(params.workspace_id);
        q.push(" AND \"contactInboxId\" = ");
        q.push_bind(patch.contact_inbox_id);
        q.push(" AND \"sourceId\" = ");
        q.push_bind(patch.source_id.clone());
        q.push(" AND \"createdAt\" >= ");
        q.push_bind(since);
        q.execute(conn)?;
    }
    Ok(())
}

/// `deleteAttachmentsByMessageId`.
pub fn delete_attachments_by_message_id(
    conn: &Connection,
    message_id: i64,
    workspace_id: i64,
    _created_at: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"Attachment\" WHERE \"messageId\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![message_id, workspace_id],
    )?;
    Ok(())
}

/// `updateSendError`.
pub fn update_send_error(
    conn: &Connection,
    id: i64,
    send_error: Option<&str>,
    workspace_id: i64,
    created_at: i64,
) -> Result<Option<i64>> {
    let set = set_value("sendError", opt_text(send_error));
    Ok(update_by_key(conn, id, workspace_id, created_at, set, None)?.map(|r| r.id))
}

/// `updateSourceId`.
pub fn update_source_id(
    conn: &Connection,
    id: i64,
    source_id: &str,
    workspace_id: i64,
    created_at: i64,
) -> Result<Option<i64>> {
    let set = set_value("sourceId", Value::Text(source_id.to_owned()));
    Ok(update_by_key(conn, id, workspace_id, created_at, set, None)?.map(|r| r.id))
}

/// `deleteBySourceId`: soft-deletes the row and its children (`parentId` = the row id). The
/// `createdAt` hint only chose shards; the row is found at any age.
pub fn delete_by_source_id(
    conn: &Connection,
    source_id: &str,
    workspace_id: i64,
    _created_at: i64,
) -> Result<Vec<MessageRef>> {
    let parent: Option<i64> = Sql::new(
        "SELECT \"id\" FROM \"Message\" WHERE \"workspaceId\" = ?1 AND \"sourceId\" = ?2 LIMIT 1",
        vec![
            Value::Integer(workspace_id),
            Value::Text(source_id.to_owned()),
        ],
    )
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .next();
    let now = now_ms();
    let mut q = Sql::raw("UPDATE \"Message\" SET \"deletedAt\" = ");
    q.push_bind(now);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now);
    q.push(" WHERE \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND (\"sourceId\" = ");
    q.push_bind(source_id.to_owned());
    if let Some(parent_id) = parent {
        q.push(" OR \"parentId\" = ");
        q.push_bind(parent_id.to_string());
    }
    q.push(") RETURNING \"conversationId\", \"id\"");
    let rows = q.query_map(conn, |r| {
        Ok(MessageRef {
            conversation_id: r.get(0)?,
            id: r.get(1)?,
        })
    })?;
    let mut out: Vec<MessageRef> = Vec::new();
    for row in rows {
        if !out.iter().any(|r| r.id == row.id) {
            out.push(row);
        }
    }
    Ok(out)
}

/// `deleteById` (soft delete; returns the ids).
pub fn delete_by_id(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    created_at: i64,
) -> Result<Vec<i64>> {
    let now = now_ms();
    let rows = Sql::new(
        "UPDATE \"Message\" SET \"deletedAt\" = ?1, \"updatedAt\" = ?1 WHERE \"workspaceId\" = ?2 \
         AND \"id\" = ?3 AND \"createdAt\" = ?4 RETURNING \"id\"",
        vec![
            Value::Integer(now),
            Value::Integer(workspace_id),
            Value::Integer(id),
            Value::Integer(created_at),
        ],
    )
    .query_map(conn, |r| r.get(0))?;
    Ok(rows)
}

/// `hardDeleteAllByContactInbox`: deletes the contact inbox's messages since `sinceTime` and
/// their attachments; returns the attachment paths (origin + thumbnail, once each).
pub fn hard_delete_all_by_contact_inbox(
    conn: &Connection,
    params: &HardDeleteAllByContactInboxParams,
) -> Result<HardDeleteAllByContactInboxResult> {
    let attachment_where = "\"Attachment\".\"workspaceId\" = ?1 AND EXISTS (SELECT 1 FROM \"Message\" \
         WHERE \"Message\".\"workspaceId\" = ?1 AND \"Message\".\"createdAt\" >= ?2 \
         AND \"Message\".\"id\" = \"Attachment\".\"messageId\" \
         AND \"Message\".\"createdAt\" = \"Attachment\".\"messageCreatedAt\" \
         AND \"Message\".\"contactInboxId\" = ?3)";
    let args = vec![
        Value::Integer(params.workspace_id),
        Value::Integer(params.since_time),
        Value::Integer(params.contact_inbox_id),
    ];
    let paths = Sql::new(
        format!(
            "SELECT \"originPath\", \"thumbnailPath\" FROM \"Attachment\" WHERE {attachment_where}"
        ),
        args.clone(),
    )
    .query_map(conn, |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    Sql::new(
        format!("DELETE FROM \"Attachment\" WHERE {attachment_where}"),
        args.clone(),
    )
    .execute(conn)?;
    Sql::new(
        "DELETE FROM \"Message\" WHERE \"workspaceId\" = ?1 AND \"contactInboxId\" = ?3 AND \"createdAt\" >= ?2",
        args,
    )
    .execute(conn)?;
    let mut attachment_paths: Vec<String> = Vec::new();
    for (origin, thumbnail) in paths {
        for path in std::iter::once(Some(origin))
            .chain(std::iter::once(thumbnail))
            .flatten()
        {
            if !path.is_empty() && !attachment_paths.contains(&path) {
                attachment_paths.push(path);
            }
        }
    }
    Ok(HardDeleteAllByContactInboxResult { attachment_paths })
}

/// `updateAttachment` (returns the updated row count).
pub fn update_attachment(conn: &Connection, params: &UpdateAttachmentParams) -> Result<usize> {
    let mut q = Sql::raw("UPDATE \"Attachment\" SET \"originPath\" = ");
    q.push_bind(params.fields.origin_path.clone());
    q.push(", \"mimeType\" = ");
    q.push_bind(params.fields.mime_type.clone());
    q.push(", \"size\" = ");
    q.push_bind(params.fields.size);
    if let Some(height) = params.fields.height {
        q.push(", \"height\" = ");
        q.push_bind(height);
    }
    if let Some(width) = params.fields.width {
        q.push(", \"width\" = ");
        q.push_bind(width);
    }
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(params.id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(params.workspace_id);
    q.push(" AND \"createdAt\" = ");
    q.push_bind(params.created_at);
    q.execute(conn)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// `hasAttachmentOfType`.
pub fn has_attachment_of_type(
    conn: &Connection,
    params: &HasAttachmentOfTypeParams,
) -> Result<bool> {
    let rows: Vec<i64> = Sql::new(
        "SELECT \"id\" FROM \"Attachment\" WHERE \"messageId\" = ?1 AND \"messageCreatedAt\" = ?2 \
         AND \"fileType\" = ?3 LIMIT 1",
        vec![
            Value::Integer(params.message_id),
            Value::Integer(params.message_created_at),
            Value::Text(params.file_type.clone()),
        ],
    )
    .query_map(conn, |r| r.get(0))?;
    Ok(!rows.is_empty())
}

/// `findById` (with the message's attachments).
pub fn find_by_id(
    conn: &Connection,
    params: &FindMessageByIdParams,
) -> Result<Option<MessageWithAttachments>> {
    let mut q = Sql::raw(Message::select_sql());
    q.push(" WHERE \"id\" = ");
    q.push_bind(params.id);
    q.push(" AND \"createdAt\" = ");
    q.push_bind(params.created_at);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(params.workspace_id);
    if let Some(conversation_id) = params.conversation_id {
        q.push(" AND \"conversationId\" = ");
        q.push_bind(conversation_id);
    }
    q.push(" LIMIT 1");
    let Some(message) = q.model::<Message>(conn)? else {
        return Ok(None);
    };
    Ok(with_attachments(conn, vec![message])?.into_iter().next())
}

/// `findTriggerMessage` (`findByIdInConversation`).
pub fn find_trigger_message(
    conn: &Connection,
    options: &FindTriggerMessageOptions,
) -> Result<Option<MessageWithAttachments>> {
    let Some(message) = Sql::new(
        message_sql(
            "WHERE \"id\" = ?1 AND \"conversationId\" = ?2 AND \"workspaceId\" = ?3 AND \"createdAt\" >= ?4 LIMIT 1",
        ),
        vec![
            Value::Integer(options.id),
            Value::Integer(options.conversation_id),
            Value::Integer(options.workspace_id),
            Value::Integer(options.since_time),
        ],
    )
    .model::<Message>(conn)?
    else {
        return Ok(None);
    };
    Ok(with_attachments(conn, vec![message])?.into_iter().next())
}

/// `findBySourceId` (needs `sinceTime`; the newest match).
pub fn find_by_source_id(
    conn: &Connection,
    source_id: &str,
    conversation_id: i64,
    workspace_id: i64,
    since_time: Option<i64>,
) -> Result<Option<Message>> {
    let Some(since) = since_time else {
        return Err(since_required("findBySourceId"));
    };
    Sql::new(
        message_sql(
            "WHERE \"sourceId\" = ?1 AND \"conversationId\" = ?2 AND \"workspaceId\" = ?3 AND \"createdAt\" >= ?4 \
             ORDER BY \"createdAt\" DESC LIMIT 1",
        ),
        vec![
            Value::Text(source_id.to_owned()),
            Value::Integer(conversation_id),
            Value::Integer(workspace_id),
            Value::Integer(since),
        ],
    )
    .model::<Message>(conn)
}

/// `findManyBySourceIds` (needs `sinceTime`).
pub fn find_many_by_source_ids(
    conn: &Connection,
    params: &FindManyBySourceIdsParams,
) -> Result<Vec<MessageSourceRow>> {
    if params.contact_inbox_ids.is_empty() || params.source_ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(since) = params.since_time else {
        return Err(since_required("findManyBySourceIds"));
    };
    Sql::new(
        "SELECT \"id\", \"conversationId\", \"contactInboxId\", \"sourceId\", \"createdAt\" FROM \"Message\" \
         WHERE \"workspaceId\" = ?1 AND \"contactInboxId\" IN (SELECT value FROM json_each(?2)) \
         AND \"sourceId\" IN (SELECT value FROM json_each(?3)) AND \"createdAt\" >= ?4 ORDER BY \"createdAt\", \"id\"",
        vec![
            Value::Integer(params.workspace_id),
            Value::Text(ids_json(&params.contact_inbox_ids)),
            Value::Text(super::text_list(&params.source_ids)),
            Value::Integer(since),
        ],
    )
    .query_map(conn, |r| {
        Ok(MessageSourceRow {
            id: r.get(0)?,
            conversation_id: r.get(1)?,
            contact_inbox_id: r.get(2)?,
            source_id: r.get(3)?,
            created_at: r.get(4)?,
        })
    })
}

/// `findAttachmentById`.
pub fn find_attachment_by_id(
    conn: &Connection,
    params: &FindAttachmentByIdParams,
) -> Result<Option<AttachmentLookupRow>> {
    let rows = Sql::new(
        "SELECT \"id\", \"messageId\", \"messageCreatedAt\", \"sourceId\", \"originPath\", \"mimeType\", \"size\", \
         \"createdAt\", \"keepOriginal\" FROM \"Attachment\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 LIMIT 1",
        vec![Value::Integer(params.id), Value::Integer(params.workspace_id)],
    )
    .query_map(conn, |r| {
        Ok(AttachmentLookupRow {
            id: r.get(0)?,
            message_id: r.get(1)?,
            message_created_at: r.get(2)?,
            source_id: r.get(3)?,
            origin_path: r.get(4)?,
            mime_type: r.get(5)?,
            size: r.get(6)?,
            created_at: r.get(7)?,
            keep_original: r.get(8)?,
        })
    })?;
    Ok(rows.into_iter().next())
}

fn rich_response_of(content_attributes: Option<Json>) -> Option<JsonValue> {
    let rich = content_attributes?.0.get("richResponse")?.clone();
    js_truthy(&rich).then_some(rich)
}

/// `findRichResponseByButton`: the rich response of `messageId` (when it has the button), else
/// the newest message of the conversation whose rich response has the button. A rich
/// response on `messageId` without the button returns `None` without the fallback.
pub fn find_rich_response_by_button(
    conn: &Connection,
    params: &FindRichResponseByButtonParams,
) -> Result<Option<JsonValue>> {
    let since = params
        .since_time
        .unwrap_or_else(|| now_ms() - RICH_RESPONSE_FALLBACK_LOOKBACK_MS);
    if let Some(message_id) = params.message_id {
        let rows: Vec<Option<Json>> = Sql::new(
            "SELECT \"contentAttributes\" FROM \"Message\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 \
             AND \"conversationId\" = ?3 AND \"contactInboxId\" = ?4 AND \"createdAt\" >= ?5 LIMIT 1",
            vec![
                Value::Integer(message_id),
                Value::Integer(params.workspace_id),
                Value::Integer(params.conversation_id),
                Value::Integer(params.contact_inbox_id),
                Value::Integer(since),
            ],
        )
        .query_map(conn, |r| r.get(0))?;
        if let Some(rich) = rows.into_iter().next().and_then(rich_response_of) {
            // `richResponse.buttonPayloads[buttonId]` of a rich response without
            // `buttonPayloads` throws in the TS; the catch goes on to the fallback.
            if let Some(payloads) = rich.get("buttonPayloads").filter(|p| p.is_object()) {
                return Ok(payloads
                    .get(&params.button_id)
                    .is_some_and(js_truthy)
                    .then_some(rich));
            }
        }
    }
    let rows: Vec<Option<Json>> = Sql::new(
        "SELECT \"contentAttributes\" FROM \"Message\" WHERE \"workspaceId\" = ?1 AND \"conversationId\" = ?2 \
         AND \"contactInboxId\" = ?3 AND \"createdAt\" >= ?4 AND EXISTS (SELECT 1 FROM \
         json_each(\"contentAttributes\", '$.richResponse.buttonPayloads') WHERE key = ?5) \
         ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT 1",
        vec![
            Value::Integer(params.workspace_id),
            Value::Integer(params.conversation_id),
            Value::Integer(params.contact_inbox_id),
            Value::Integer(since),
            Value::Text(params.button_id.clone()),
        ],
    )
    .query_map(conn, |r| r.get(0))?;
    Ok(rows.into_iter().next().and_then(rich_response_of))
}

/// `findAIContextMessages`: messages after the marker (newest `limit`), in chronological order.
pub fn find_ai_context_messages(
    conn: &Connection,
    options: &FindAiContextMessagesOptions,
) -> Result<Vec<Message>> {
    let Some(since) = options.since_time else {
        return Err(shard_unavailable(
            "sinceTime is required for sharded AI context reads",
        ));
    };
    let marker: Option<(i64, i64)> = match options.marker_message_id {
        Some(marker_id) => Sql::new(
            "SELECT \"createdAt\", \"id\" FROM \"Message\" WHERE \"id\" = ?1 AND \"conversationId\" = ?2 \
             AND \"workspaceId\" = ?3 LIMIT 1",
            vec![
                Value::Integer(marker_id),
                Value::Integer(options.conversation_id),
                Value::Integer(options.workspace_id),
            ],
        )
        .query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?
        .into_iter()
        .next(),
        None => None,
    };
    let mut q = Sql::raw(Message::select_sql());
    q.push(" WHERE \"conversationId\" = ");
    q.push_bind(options.conversation_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(options.workspace_id);
    q.push(" AND \"createdAt\" >= ");
    q.push_bind(since);
    push_types(&mut q, &options.message_types);
    if options.text_not_null {
        q.push(" AND \"text\" IS NOT NULL");
    }
    if let Some((created_at, id)) = marker {
        q.push(" AND (\"createdAt\", \"id\") > (");
        q.push_bind(created_at);
        q.push(", ");
        q.push_bind(id);
        q.push(")");
    }
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(options.limit);
    let mut rows = q.models::<Message>(conn)?;
    rows.reverse();
    Ok(rows)
}

fn conversation_query(
    conversation_id: i64,
    since: i64,
    workspace_id: Option<i64>,
    message_types: &[String],
    text_not_null: bool,
    limit: i64,
) -> Sql {
    let mut q = Sql::raw(Message::select_sql());
    q.push(" WHERE \"conversationId\" = ");
    q.push_bind(conversation_id);
    q.push(" AND \"createdAt\" >= ");
    q.push_bind(since);
    if let Some(ws) = workspace_id {
        q.push(" AND \"workspaceId\" = ");
        q.push_bind(ws);
    }
    push_types(&mut q, message_types);
    if text_not_null {
        q.push(" AND \"text\" IS NOT NULL");
    }
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(limit);
    q
}

/// `findLastByConversation` (needs `sinceTime`; newest first).
pub fn find_last_by_conversation(
    conn: &Connection,
    conversation_id: i64,
    options: &FindLastByConversationOptions,
) -> Result<Vec<MessageWithAttachments>> {
    let Some(since) = options.since_time else {
        return Err(since_required("findLastByConversation"));
    };
    let q = conversation_query(
        conversation_id,
        since,
        options.workspace_id,
        &options.message_types,
        false,
        options.limit.unwrap_or(1),
    );
    let messages = q.models::<Message>(conn)?;
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    if options.with_attachments {
        return with_attachments(conn, messages);
    }
    if options.attachment_count_only {
        return with_attachment_counts(conn, messages);
    }
    Ok(bare(messages))
}

/// `findManyByConversation` (needs `sinceTime`; newest first).
pub fn find_many_by_conversation(
    conn: &Connection,
    conversation_id: i64,
    options: &FindManyByConversationOptions,
) -> Result<Vec<Message>> {
    let Some(since) = options.since_time else {
        return Err(since_required("findManyByConversation"));
    };
    conversation_query(
        conversation_id,
        since,
        options.workspace_id,
        &options.message_types,
        options.text_not_null,
        options.limit,
    )
    .models::<Message>(conn)
}

/// `findManyByIds` (needs `sinceTime`; `workspace_id` only chose the write shard in the TS).
pub fn find_many_by_ids(
    conn: &Connection,
    ids: &[i64],
    contact_inbox_id: i64,
    since_time: Option<i64>,
    _workspace_id: Option<i64>,
) -> Result<Vec<MessageTextWithFileTypes>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(since) = since_time else {
        return Err(since_required("findManyByIds"));
    };
    let messages = Sql::new(
        message_sql(
            "WHERE \"contactInboxId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) AND \"createdAt\" >= ?3",
        ),
        vec![
            Value::Integer(contact_inbox_id),
            Value::Text(ids_json(ids)),
            Value::Integer(since),
        ],
    )
    .models::<Message>(conn)?;
    Ok(with_attachments(conn, messages)?
        .into_iter()
        .map(|m| MessageTextWithFileTypes {
            id: m.message.id,
            text: m.message.text,
            attachment_file_types: m.attachments.into_iter().map(|a| a.file_type).collect(),
        })
        .collect())
}

/// `listIncomingTextsByContactInbox`: incoming, non-deleted texts since `sinceTime`, newest
/// first, read in keyset chunks of at most 1000 rows.
pub fn list_incoming_texts_by_contact_inbox(
    conn: &Connection,
    params: &ListIncomingTextsByContactInboxParams,
) -> Result<Vec<String>> {
    let mut messages: Vec<String> = Vec::new();
    let mut cursor: Option<(i64, i64)> = None;
    loop {
        let remaining = match params.limit {
            Some(limit) if messages.len() as i64 >= limit => break,
            Some(limit) => LIST_INCOMING_TEXTS_BATCH_SIZE.min(limit - messages.len() as i64),
            None => LIST_INCOMING_TEXTS_BATCH_SIZE,
        };
        let mut q = Sql::raw(
            "SELECT \"createdAt\", \"id\", \"text\" FROM \"Message\" WHERE \"workspaceId\" = ",
        );
        q.push_bind(params.workspace_id);
        q.push(" AND \"contactInboxId\" = ");
        q.push_bind(params.contact_inbox_id);
        q.push(" AND \"messageType\" = 'incoming' AND \"text\" IS NOT NULL AND \"deletedAt\" IS NULL AND \"createdAt\" >= ");
        q.push_bind(params.since_time);
        if let Some((created_at, id)) = cursor {
            q.push(" AND (\"createdAt\", \"id\") < (");
            q.push_bind(created_at);
            q.push(", ");
            q.push_bind(id);
            q.push(")");
        }
        q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
        q.push_bind(remaining);
        let rows: Vec<(i64, i64, Option<String>)> =
            q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let count = rows.len() as i64;
        let last = rows.last().map(|(c, i, _)| (*c, *i));
        messages.extend(rows.into_iter().filter_map(|(_, _, text)| text));
        if count < remaining {
            break;
        }
        match last {
            Some(key) => cursor = Some(key),
            None => break,
        }
    }
    Ok(messages)
}

/// `listByConversation`: newest first, keyset cursor, `limit + 1` over-fetch. No time filter
/// (`sinceTime` only chose shards).
pub fn list_by_conversation(
    conn: &Connection,
    query: &ListMessagesQuery,
) -> Result<PaginatedMessages> {
    let limit = query.pagination.limit;
    let mut q = Sql::raw(Message::select_sql());
    q.push(" WHERE \"workspaceId\" = ");
    q.push_bind(query.workspace_id);
    if let Some(conversation_id) = query.conversation_id {
        q.push(" AND \"conversationId\" = ");
        q.push_bind(conversation_id);
    }
    if let Some(contact_inbox_id) = query.contact_inbox_id {
        q.push(" AND \"contactInboxId\" = ");
        q.push_bind(contact_inbox_id);
    }
    if let Some(cursor) = &query.pagination.cursor {
        match cursor.id {
            Some(id) => {
                q.push(" AND (\"createdAt\", \"id\") < (");
                q.push_bind(cursor.created_at);
                q.push(", ");
                q.push_bind(id);
                q.push(")");
            }
            None => {
                q.push(" AND \"createdAt\" < ");
                q.push_bind(cursor.created_at);
            }
        }
    }
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(limit.max(0) + 1);
    let mut messages = q.models::<Message>(conn)?;
    let has_more = messages.len() as i64 > limit;
    messages.truncate(limit.max(0) as usize);
    if messages.is_empty() {
        return Ok(PaginatedMessages {
            data: Vec::new(),
            has_partial_results: None,
            next_cursor: None,
        });
    }
    let next_cursor = if has_more {
        messages.last().map(|m| PaginationCursor {
            created_at: m.created_at,
            id: Some(m.id),
            shard_id: None,
        })
    } else {
        None
    };
    Ok(PaginatedMessages {
        data: with_attachments(conn, messages)?,
        has_partial_results: None,
        next_cursor,
    })
}
