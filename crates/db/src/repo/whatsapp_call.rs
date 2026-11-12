//! Port of `repositories/whatsapp-call/repository.ts` (P3.3).
//!
//! Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`). "now" inputs are parameters. The
//! one SQLite writer serializes the guarded compare-and-set updates (`status = <seen status>`),
//! so their retry loop runs at most twice as in the TS.

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::Value as JsonValue;

use super::models::WhatsappCall;
use super::sql::{Sql, bool_val, opt_int, opt_text};
use super::{Error, Model, Result, iso, new_id, now_ms, parse_iso};

/// `FIND_RINGING_BY_WORKSPACE_LIMIT`.
pub const FIND_RINGING_BY_WORKSPACE_LIMIT: i64 = 20;

/// `WHATSAPP_CALL_TERMINAL_STATUSES`.
pub const WHATSAPP_CALL_TERMINAL_STATUSES: [&str; 3] = ["rejected", "completed", "failed"];

fn status_rank(status: &str) -> i64 {
    match status {
        "ringing" => 0,
        "accepted" => 1,
        "rejected" => 2,
        "failed" => 3,
        "completed" => 4,
        _ => -1,
    }
}

/// `canAdvanceStatus`: a strictly higher rank, or the `failed -> rejected` repair.
pub fn can_advance_status(current: &str, next: &str) -> bool {
    todo!("P3.3")
}

/// `resolveWhatsappCallOutcome({ status })` (no `canceledByBusiness`).
pub fn resolve_outcome(status: &str) -> &'static str {
    todo!("P3.3")
}

/// `WhatsappCallUuidMismatchError`.
pub fn uuid_mismatch_error(wacid: &str) -> Error {
    todo!("P3.3")
}

/// `WhatsappCallPendingOutboundExistsError`.
pub fn pending_outbound_exists_error(contact_inbox_id: i64) -> Error {
    todo!("P3.3")
}

/// Resolves the index name of a UNIQUE error inside the write job (the job wrapper resolves it
/// only when the error leaves the job).
fn resolved(conn: &Connection, error: Error) -> Error {
    match error {
        Error::Db(mut e) => {
            e.resolve_constraint(conn);
            Error::Db(e)
        }
        other => other,
    }
}

fn is_unique_violation(error: &Error, constraint: &str) -> bool {
    error
        .as_db()
        .is_some_and(|e| e.is_unique_violation() && e.constraint() == Some(constraint))
}

fn one(conn: &Connection, where_sql: &str, params: Vec<Value>) -> Result<Option<WhatsappCall>> {
    Sql::new(
        format!("{} WHERE {where_sql} LIMIT 1", WhatsappCall::select_sql()),
        params,
    )
    .model(conn)
}

fn returning_one(conn: &Connection, mut q: Sql) -> Result<Option<WhatsappCall>> {
    q.push(&format!(" RETURNING {}", WhatsappCall::columns_sql(None)));
    q.model::<WhatsappCall>(conn)
}

/// `UPDATE "WhatsappCall" SET <set>, "updatedAt" = now WHERE <where>` (no RETURNING yet).
fn update_sql(set: Sql, where_sql: Sql) -> Sql {
    let mut q = Sql::raw("UPDATE \"WhatsappCall\" SET ");
    q.append(set);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE ");
    q.append(where_sql);
    q
}

fn by_id(id: i64) -> Sql {
    Sql::bind("\"id\" = ", id)
}

fn and(mut a: Sql, text: &str, value: impl Into<Value>) -> Sql {
    a.push(" AND ");
    a.push(text);
    a.push_bind(value);
    a
}

/// `findById`.
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `findByWacid`.
pub fn find_by_wacid(conn: &Connection, wacid: &str) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `findByAttemptId`.
pub fn find_by_attempt_id(conn: &Connection, attempt_id: &str) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `WhatsappCallUpsertInput` (`status` is not terminal).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WhatsappCallUpsertInput {
    pub wacid: String,
    pub direction: String,
    pub status: String,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub duration_seconds: Option<i64>,
}

/// `{ call, isNew }` of `createIfAbsent`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateIfAbsentResult {
    pub call: WhatsappCall,
    pub is_new: bool,
}

/// `createIfAbsent`: `ON CONFLICT ("wacid") WHERE "wacid" IS NOT NULL DO NOTHING`, then the winner.
pub fn create_if_absent(
    conn: &Connection,
    input: &WhatsappCallUpsertInput,
) -> Result<CreateIfAbsentResult> {
    todo!("P3.3")
}

/// Input of `createPendingOutbound`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PendingOutboundInput {
    pub attempt_id: String,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub answered_by_user_id: Option<i64>,
    pub initiated_by_user_id: Option<i64>,
}

/// `createPendingOutbound`: a null-wacid `businessInitiated` `ringing` row; the
/// `WhatsappCall_pendingOutbound_key` violation is `WhatsappCallPendingOutboundExistsError`.
pub fn create_pending_outbound(
    conn: &Connection,
    input: &PendingOutboundInput,
) -> Result<WhatsappCall> {
    todo!("P3.3")
}

/// `findActiveByContactInbox`: the newest `ringing` / `accepted` row (any direction).
pub fn find_active_by_contact_inbox(
    conn: &Connection,
    inbox_id: i64,
    contact_inbox_id: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `attachWacid`. A wacid owned by another row merges the newer row into the older one
/// (`attemptId`, recording and transcript fields: older wins, newer fills) and deletes the
/// newer row. The `isNull(wacid)` UPDATE that hits `WhatsappCall_wacid_key` counts as "no row
/// updated" (the TS then reads the owner); the newer row is deleted before the older one takes
/// its `attemptId` (`WhatsappCall_attemptId_key` is not deferrable).
pub fn attach_wacid(conn: &Connection, id: i64, wacid: &str) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

fn json_value(value: Option<JsonValue>) -> Result<Value> {
    Ok(match value {
        None | Some(JsonValue::Null) => Value::Null,
        Some(v) => Value::Text(super::json_text(&v)?),
    })
}

fn latest_by_contact(
    conn: &Connection,
    contact_id: i64,
    column: &str,
) -> Result<Option<WhatsappCall>> {
    Sql::new(
        format!(
            "SELECT {} FROM \"WhatsappCall\" w JOIN \"ContactInbox\" ci ON w.\"contactInboxId\" = ci.\"id\" \
             WHERE ci.\"contactId\" = ?1 AND w.\"{column}\" IS NOT NULL ORDER BY w.\"createdAt\" DESC LIMIT 1",
            WhatsappCall::columns_sql(Some("w"))
        ),
        vec![Value::Integer(contact_id)],
    )
    .model(conn)
}

/// `findLatestRecordedByContactId`.
pub fn find_latest_recorded_by_contact_id(
    conn: &Connection,
    contact_id: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `findLatestTranscribedByContactId`.
pub fn find_latest_transcribed_by_contact_id(
    conn: &Connection,
    contact_id: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `findRingingByWorkspace`: ringing, wacid present, unanswered; newest first, at most 20.
pub fn find_ringing_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<WhatsappCall>> {
    todo!("P3.3")
}

/// `sweepStaleRinging`: ringing rows created before `older_than`, oldest first.
pub fn sweep_stale_ringing(
    conn: &Connection,
    older_than: i64,
    limit: i64,
) -> Result<Vec<WhatsappCall>> {
    todo!("P3.3")
}

/// `listRecordingsPastRetention`: recordings older than their inbox's
/// `IntegrationWhatsapp.callRecordingRetentionDays` at `now`; bounded by `limit`.
pub fn list_recordings_past_retention(
    conn: &Connection,
    limit: i64,
    now: i64,
) -> Result<Vec<WhatsappCall>> {
    todo!("P3.3")
}

/// `clearRecording`.
pub fn clear_recording(conn: &Connection, id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `markRecordingArrangement`.
pub fn mark_recording_arrangement(
    conn: &Connection,
    id: i64,
    recording_requested: bool,
    recording_failure_reason: Option<&str>,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `attachRecording` (only while `recordedAt` is NULL).
pub fn attach_recording(
    conn: &Connection,
    id: i64,
    recording_path: &str,
    recorded_at: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `releaseRecordingStamp` (only the stamp of `recorded_at`).
pub fn release_recording_stamp(
    conn: &Connection,
    id: i64,
    recorded_at: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// Input of `attachTranscript` (`segments` `None` = not set).
#[derive(Debug, Clone, PartialEq)]
pub struct AttachTranscriptInput {
    pub id: i64,
    pub transcript: String,
    pub transcribed_at: i64,
    pub segments: Option<Option<JsonValue>>,
}

/// `attachTranscript` (only while `transcript` is NULL).
pub fn attach_transcript(
    conn: &Connection,
    input: &AttachTranscriptInput,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// Input of `attachAiSummary` / `overwriteAiSummary` (`ai_summarized_at` `None` = now).
#[derive(Debug, Clone, PartialEq)]
pub struct AiSummaryInput {
    pub id: i64,
    pub ai_summary: JsonValue,
    pub ai_summary_provider: String,
    pub ai_summarized_at: Option<i64>,
}

fn ai_summary_set(input: &AiSummaryInput) -> Result<Sql> {
    let mut set = Sql::bind(
        "\"aiSummary\" = ",
        json_value(Some(input.ai_summary.clone()))?,
    );
    set.push(", \"aiSummaryProvider\" = ");
    set.push_bind(input.ai_summary_provider.clone());
    set.push(", \"aiSummarizedAt\" = ");
    set.push_bind(input.ai_summarized_at.unwrap_or_else(now_ms));
    Ok(set)
}

/// `attachAiSummary` (only while `aiSummarizedAt` is NULL).
pub fn attach_ai_summary(
    conn: &Connection,
    input: &AiSummaryInput,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `overwriteAiSummary`.
pub fn overwrite_ai_summary(
    conn: &Connection,
    input: &AiSummaryInput,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `updateInterimStatus` (`ringing` | `accepted` | `rejected`); `rejected` also sets the
/// outcome. Returns the previous status when the guarded update applied.
pub fn update_interim_status(
    conn: &Connection,
    wacid: &str,
    status: &str,
    current: Option<&WhatsappCall>,
) -> Result<Option<String>> {
    todo!("P3.3")
}

/// `markAcceptedIfActive` (no-op from a terminal status).
pub fn mark_accepted_if_active(
    conn: &Connection,
    id: i64,
    answered_by_user_id: i64,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `touchLivenessIfStale`: bumps `updatedAt` of an `accepted` row not updated since `older_than`.
pub fn touch_liveness_if_stale(conn: &Connection, id: i64, older_than: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `recoverStrandedAccepted`: terminalizes a stale `accepted` row as `completed`.
pub fn recover_stranded_accepted(
    conn: &Connection,
    id: i64,
    older_than: i64,
    last_error: &str,
) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// Input of `finalizeById` (outer `None` = key not passed; `Some(None)` = set NULL).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FinalizeInput {
    pub id: i64,
    /// Terminal status (`completed` | `rejected` | `failed`).
    pub status: String,
    /// Its outcome (`resolveWhatsappCallTerminalOutcomePair`).
    pub outcome: String,
    pub started_at: Option<Option<i64>>,
    pub ended_at: Option<Option<i64>>,
    pub duration_seconds: Option<Option<i64>>,
    pub message_id: Option<Option<i64>>,
    pub last_error: Option<Option<String>>,
    pub answered_by_user_id: Option<Option<i64>>,
    pub current: Option<WhatsappCall>,
}

fn push_set(set: &mut Sql, first: &mut bool, column: &str, value: Value) {
    if !*first {
        set.push(", ");
    }
    *first = false;
    set.push(&format!("\"{column}\" = "));
    set.push_bind(value);
}

/// `finalizeById`: advances to the terminal status (guarded by the seen status, retried once);
/// a same-status redelivery only fills still-NULL terminal fields; a downgrade is a no-op.
pub fn finalize_by_id(conn: &Connection, input: &FinalizeInput) -> Result<Option<WhatsappCall>> {
    todo!("P3.3")
}

/// `fillMissingTerminalFields`: sets each `FILLABLE_TERMINAL_FIELDS` column that is NULL on the
/// row and given (non-null) in the input, guarded by status and the same NULL checks.
fn fill_missing_terminal_fields(
    conn: &Connection,
    input: &FinalizeInput,
    current: WhatsappCall,
) -> Result<WhatsappCall> {
    let candidates: [(&str, bool, Option<Value>); 6] = [
        (
            "endedAt",
            current.ended_at.is_none(),
            input.ended_at.flatten().map(Value::Integer),
        ),
        (
            "startedAt",
            current.started_at.is_none(),
            input.started_at.flatten().map(Value::Integer),
        ),
        (
            "durationSeconds",
            current.duration_seconds.is_none(),
            input.duration_seconds.flatten().map(Value::Integer),
        ),
        (
            "messageId",
            current.message_id.is_none(),
            input.message_id.flatten().map(Value::Integer),
        ),
        (
            "lastError",
            current.last_error.is_none(),
            input.last_error.clone().flatten().map(Value::Text),
        ),
        (
            "outcome",
            current.outcome.is_none(),
            Some(Value::Text(input.outcome.clone())),
        ),
    ];
    let mut set = Sql::default();
    let mut first = true;
    let mut guard = and(by_id(input.id), "\"status\" = ", current.status.clone());
    for (column, is_null, value) in candidates {
        if let (true, Some(value)) = (is_null, value) {
            push_set(&mut set, &mut first, column, value);
            guard.push(&format!(" AND \"{column}\" IS NULL"));
        }
    }
    if first {
        return Ok(current);
    }
    Ok(returning_one(conn, update_sql(set, guard))?.unwrap_or(current))
}

/// `WhatsappCallHistoryScope`.
#[derive(Debug, Clone, PartialEq)]
pub enum WhatsappCallHistoryScope {
    AllCalls,
    Own { user_id: i64, assigned_only: bool },
}

/// `WhatsappCallListFilters`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WhatsappCallListFilters {
    pub direction: Option<String>,
    pub inbox_id: Option<i64>,
    pub agent_user_id: Option<i64>,
    pub outcome: Option<String>,
    pub ongoing: bool,
}

/// `WhatsappCallListCursor` (`created_at` = the `createdAtCursor` text of a row).
#[derive(Debug, Clone, PartialEq)]
pub struct WhatsappCallListCursor {
    pub created_at: String,
    pub id: i64,
}

/// Input of `listForWorkspace`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListForWorkspaceInput {
    pub workspace_id: i64,
    pub scope: WhatsappCallHistoryScope,
    pub filters: WhatsappCallListFilters,
    pub cursor: Option<WhatsappCallListCursor>,
    pub limit: i64,
}

/// `{ id, fullName, avatar }`.
#[derive(Debug, Clone, PartialEq)]
pub struct CallContact {
    pub id: i64,
    pub full_name: Option<String>,
    pub avatar: Option<String>,
}

/// `{ id, name, email }` of a joined user.
#[derive(Debug, Clone, PartialEq)]
pub struct CallUser {
    pub id: i64,
    pub name: Option<String>,
    pub email: String,
}

/// `WhatsappCallListRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct WhatsappCallListRow {
    pub call: WhatsappCall,
    /// `createdAt::text` (here the ISO millisecond text; `parse_iso` reads it back exactly).
    pub created_at_cursor: String,
    pub contact: CallContact,
    pub inbox_id: i64,
    pub inbox_name: String,
    pub answered_by_user: Option<CallUser>,
    pub initiated_by_user: Option<CallUser>,
}

fn legacy_status_fallback(outcome: &str) -> Option<&'static str> {
    match outcome {
        "completed" => Some("completed"),
        "rejected" => Some("rejected"),
        "failed" => Some("failed"),
        _ => None,
    }
}

/// `listForWorkspace`: the workspace's calls with contact, inbox and agent users, newest first
/// (`createdAt DESC, id DESC`), keyset cursor, exactly `limit` rows. Never reads `Message`.
pub fn list_for_workspace(
    conn: &Connection,
    input: &ListForWorkspaceInput,
) -> Result<Vec<WhatsappCallListRow>> {
    todo!("P3.3")
}
