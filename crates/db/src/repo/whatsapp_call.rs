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
    if next == "rejected" && current == "failed" {
        return true;
    }
    status_rank(next) > status_rank(current)
}

/// `resolveWhatsappCallOutcome({ status })` (no `canceledByBusiness`).
pub fn resolve_outcome(status: &str) -> &'static str {
    match status {
        "completed" => "completed",
        "rejected" => "rejected",
        _ => "failed",
    }
}

/// `WhatsappCallUuidMismatchError`.
pub fn uuid_mismatch_error(wacid: &str) -> Error {
    Error::Coded {
        kind: "WhatsappCallUuidMismatchError",
        code: "call-uuid-mismatch".to_owned(),
        message: format!("call-uuid-mismatch: wacid {wacid} is already bound to another call row"),
    }
}

/// `WhatsappCallPendingOutboundExistsError`.
pub fn pending_outbound_exists_error(contact_inbox_id: i64) -> Error {
    Error::Coded {
        kind: "WhatsappCallPendingOutboundExistsError",
        code: "pending-outbound-exists".to_owned(),
        message: format!(
            "pending-outbound-exists: contactInbox {contact_inbox_id} already has a live business-initiated call"
        ),
    }
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
    one(conn, "\"id\" = ?1", vec![Value::Integer(id)])
}

/// `findByWacid`.
pub fn find_by_wacid(conn: &Connection, wacid: &str) -> Result<Option<WhatsappCall>> {
    one(conn, "\"wacid\" = ?1", vec![Value::Text(wacid.to_owned())])
}

/// `findByIdForWorkspace`.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<WhatsappCall>> {
    one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    )
}

/// `findByAttemptId`.
pub fn find_by_attempt_id(conn: &Connection, attempt_id: &str) -> Result<Option<WhatsappCall>> {
    one(
        conn,
        "\"attemptId\" = ?1",
        vec![Value::Text(attempt_id.to_owned())],
    )
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
    let q = Sql::new(
        "INSERT INTO \"WhatsappCall\" (\"id\", \"wacid\", \"direction\", \"status\", \"workspaceId\", \"inboxId\", \
         \"contactInboxId\", \"conversationId\", \"startedAt\", \"endedAt\", \"durationSeconds\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT (\"wacid\") WHERE \"wacid\" IS NOT NULL DO NOTHING",
        vec![
            Value::Integer(new_id()),
            Value::Text(input.wacid.clone()),
            Value::Text(input.direction.clone()),
            Value::Text(input.status.clone()),
            Value::Integer(input.workspace_id),
            Value::Integer(input.inbox_id),
            Value::Integer(input.contact_inbox_id),
            Value::Integer(input.conversation_id),
            opt_int(input.started_at),
            opt_int(input.ended_at),
            opt_int(input.duration_seconds),
        ],
    );
    if let Some(call) = returning_one(conn, q)? {
        return Ok(CreateIfAbsentResult { call, is_new: true });
    }
    match find_by_wacid(conn, &input.wacid)? {
        Some(call) => Ok(CreateIfAbsentResult {
            call,
            is_new: false,
        }),
        None => Err(Error::msg(format!(
            "WhatsappCall upsert race lost for wacid {}",
            input.wacid
        ))),
    }
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
    let q = Sql::new(
        "INSERT INTO \"WhatsappCall\" (\"id\", \"attemptId\", \"workspaceId\", \"inboxId\", \"contactInboxId\", \
         \"conversationId\", \"answeredByUserId\", \"initiatedByUserId\", \"wacid\", \"direction\", \"status\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 'businessInitiated', 'ringing')",
        vec![
            Value::Integer(new_id()),
            Value::Text(input.attempt_id.clone()),
            Value::Integer(input.workspace_id),
            Value::Integer(input.inbox_id),
            Value::Integer(input.contact_inbox_id),
            Value::Integer(input.conversation_id),
            opt_int(input.answered_by_user_id),
            opt_int(input.initiated_by_user_id),
        ],
    );
    match returning_one(conn, q).map_err(|e| resolved(conn, e)) {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(Error::msg(format!(
            "WhatsappCall createPendingOutbound insert returned no row for attemptId {}",
            input.attempt_id
        ))),
        Err(e) if is_unique_violation(&e, "WhatsappCall_pendingOutbound_key") => {
            Err(pending_outbound_exists_error(input.contact_inbox_id))
        }
        Err(e) => Err(e),
    }
}

/// `findActiveByContactInbox`: the newest `ringing` / `accepted` row (any direction).
pub fn find_active_by_contact_inbox(
    conn: &Connection,
    inbox_id: i64,
    contact_inbox_id: i64,
) -> Result<Option<WhatsappCall>> {
    Sql::new(
        format!(
            "{} WHERE \"inboxId\" = ?1 AND \"contactInboxId\" = ?2 AND \"status\" IN ('ringing', 'accepted') \
             ORDER BY \"createdAt\" DESC LIMIT 1",
            WhatsappCall::select_sql()
        ),
        vec![Value::Integer(inbox_id), Value::Integer(contact_inbox_id)],
    )
    .model(conn)
}

/// `attachWacid`. A wacid owned by another row merges the newer row into the older one
/// (`attemptId`, recording and transcript fields: older wins, newer fills) and deletes the
/// newer row. The `isNull(wacid)` UPDATE that hits `WhatsappCall_wacid_key` counts as "no row
/// updated" (the TS then reads the owner); the newer row is deleted before the older one takes
/// its `attemptId` (`WhatsappCall_attemptId_key` is not deferrable).
pub fn attach_wacid(conn: &Connection, id: i64, wacid: &str) -> Result<Option<WhatsappCall>> {
    let Some(current) = find_by_id(conn, id)? else {
        return Ok(None);
    };
    match current.wacid.as_deref() {
        Some(w) if w == wacid => return Ok(Some(current)),
        Some(_) => return Err(uuid_mismatch_error(wacid)),
        None => {}
    }
    let q = update_sql(
        Sql::bind("\"wacid\" = ", wacid.to_owned()),
        by_id(id).closed(" AND \"wacid\" IS NULL"),
    );
    match returning_one(conn, q).map_err(|e| resolved(conn, e)) {
        Ok(Some(updated)) => return Ok(Some(updated)),
        Ok(None) => {}
        Err(e) if is_unique_violation(&e, "WhatsappCall_wacid_key") => {}
        Err(e) => return Err(e),
    }
    let owner = match find_by_wacid(conn, wacid)? {
        Some(owner) if owner.id != id => owner,
        _ => return find_by_id(conn, id),
    };
    let (older, newer) = if owner.created_at <= current.created_at {
        (owner, current)
    } else {
        (current, owner)
    };
    conn.execute("DELETE FROM \"WhatsappCall\" WHERE \"id\" = ?1", [newer.id])?;
    let mut set = Sql::bind(
        "\"attemptId\" = ",
        opt_text(older.attempt_id.as_deref().or(newer.attempt_id.as_deref())),
    );
    set.push(", \"recordingPath\" = ");
    set.push_bind(opt_text(
        older
            .recording_path
            .as_deref()
            .or(newer.recording_path.as_deref()),
    ));
    set.push(", \"recordedAt\" = ");
    set.push_bind(opt_int(older.recorded_at.or(newer.recorded_at)));
    set.push(", \"transcript\" = ");
    set.push_bind(opt_text(
        older.transcript.as_deref().or(newer.transcript.as_deref()),
    ));
    set.push(", \"transcribedAt\" = ");
    set.push_bind(opt_int(older.transcribed_at.or(newer.transcribed_at)));
    set.push(", \"transcriptSegments\" = ");
    let segments = older
        .transcript_segments
        .clone()
        .or(newer.transcript_segments.clone());
    set.push_bind(json_value(segments.map(|j| j.0))?);
    returning_one(conn, update_sql(set, by_id(older.id)))
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
    latest_by_contact(conn, contact_id, "recordingPath")
}

/// `findLatestTranscribedByContactId`.
pub fn find_latest_transcribed_by_contact_id(
    conn: &Connection,
    contact_id: i64,
) -> Result<Option<WhatsappCall>> {
    latest_by_contact(conn, contact_id, "transcript")
}

/// `findRingingByWorkspace`: ringing, wacid present, unanswered; newest first, at most 20.
pub fn find_ringing_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<WhatsappCall>> {
    Sql::new(
        format!(
            "{} WHERE \"workspaceId\" = ?1 AND \"status\" = 'ringing' AND \"wacid\" IS NOT NULL \
             AND \"answeredByUserId\" IS NULL ORDER BY \"createdAt\" DESC LIMIT ?2",
            WhatsappCall::select_sql()
        ),
        vec![
            Value::Integer(workspace_id),
            Value::Integer(FIND_RINGING_BY_WORKSPACE_LIMIT),
        ],
    )
    .models(conn)
}

/// `sweepStaleRinging`: ringing rows created before `older_than`, oldest first.
pub fn sweep_stale_ringing(
    conn: &Connection,
    older_than: i64,
    limit: i64,
) -> Result<Vec<WhatsappCall>> {
    Sql::new(
        format!(
            "{} WHERE \"status\" = 'ringing' AND \"createdAt\" < ?1 ORDER BY \"createdAt\" LIMIT ?2",
            WhatsappCall::select_sql()
        ),
        vec![Value::Integer(older_than), Value::Integer(limit)],
    )
    .models(conn)
}

/// `listRecordingsPastRetention`: recordings older than their inbox's
/// `IntegrationWhatsapp.callRecordingRetentionDays` at `now`; bounded by `limit`.
pub fn list_recordings_past_retention(
    conn: &Connection,
    limit: i64,
    now: i64,
) -> Result<Vec<WhatsappCall>> {
    Sql::new(
        format!(
            "SELECT {} FROM \"WhatsappCall\" w JOIN \"IntegrationWhatsapp\" iw ON w.\"inboxId\" = iw.\"inboxId\" \
             WHERE w.\"recordingPath\" IS NOT NULL AND w.\"recordedAt\" IS NOT NULL \
             AND w.\"recordedAt\" < ?1 - iw.\"callRecordingRetentionDays\" * 86400000 LIMIT ?2",
            WhatsappCall::columns_sql(Some("w"))
        ),
        vec![Value::Integer(now), Value::Integer(limit)],
    )
    .models(conn)
}

/// `clearRecording`.
pub fn clear_recording(conn: &Connection, id: i64) -> Result<()> {
    update_sql(
        Sql::raw("\"recordingPath\" = NULL, \"recordedAt\" = NULL"),
        by_id(id),
    )
    .execute(conn)?;
    Ok(())
}

/// `markRecordingArrangement`.
pub fn mark_recording_arrangement(
    conn: &Connection,
    id: i64,
    recording_requested: bool,
    recording_failure_reason: Option<&str>,
) -> Result<Option<WhatsappCall>> {
    let mut set = Sql::bind("\"recordingRequested\" = ", bool_val(recording_requested));
    set.push(", \"recordingFailureReason\" = ");
    set.push_bind(opt_text(recording_failure_reason));
    returning_one(conn, update_sql(set, by_id(id)))
}

/// `attachRecording` (only while `recordedAt` is NULL).
pub fn attach_recording(
    conn: &Connection,
    id: i64,
    recording_path: &str,
    recorded_at: i64,
) -> Result<Option<WhatsappCall>> {
    let mut set = Sql::bind("\"recordingPath\" = ", recording_path.to_owned());
    set.push(", \"recordedAt\" = ");
    set.push_bind(recorded_at);
    returning_one(
        conn,
        update_sql(set, by_id(id).closed(" AND \"recordedAt\" IS NULL")),
    )
}

/// `releaseRecordingStamp` (only the stamp of `recorded_at`).
pub fn release_recording_stamp(
    conn: &Connection,
    id: i64,
    recorded_at: i64,
) -> Result<Option<WhatsappCall>> {
    returning_one(
        conn,
        update_sql(
            Sql::raw("\"recordingPath\" = NULL, \"recordedAt\" = NULL"),
            and(by_id(id), "\"recordedAt\" = ", recorded_at),
        ),
    )
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
    let mut set = Sql::bind("\"transcript\" = ", input.transcript.clone());
    set.push(", \"transcribedAt\" = ");
    set.push_bind(input.transcribed_at);
    if let Some(segments) = &input.segments {
        set.push(", \"transcriptSegments\" = ");
        set.push_bind(json_value(segments.clone())?);
    }
    returning_one(
        conn,
        update_sql(set, by_id(input.id).closed(" AND \"transcript\" IS NULL")),
    )
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
    let set = ai_summary_set(input)?;
    returning_one(
        conn,
        update_sql(
            set,
            by_id(input.id).closed(" AND \"aiSummarizedAt\" IS NULL"),
        ),
    )
}

/// `overwriteAiSummary`.
pub fn overwrite_ai_summary(
    conn: &Connection,
    input: &AiSummaryInput,
) -> Result<Option<WhatsappCall>> {
    let set = ai_summary_set(input)?;
    returning_one(conn, update_sql(set, by_id(input.id)))
}

/// `updateInterimStatus` (`ringing` | `accepted` | `rejected`); `rejected` also sets the
/// outcome. Returns the previous status when the guarded update applied.
pub fn update_interim_status(
    conn: &Connection,
    wacid: &str,
    status: &str,
    current: Option<&WhatsappCall>,
) -> Result<Option<String>> {
    let mut existing = current.cloned();
    for _ in 0..2 {
        if existing.is_none() {
            existing = find_by_wacid(conn, wacid)?;
        }
        let Some(seen) = existing.take() else {
            return Ok(None);
        };
        if !can_advance_status(&seen.status, status) {
            return Ok(None);
        }
        let mut set = Sql::bind("\"status\" = ", status.to_owned());
        if status == "rejected" {
            set.push(", \"outcome\" = ");
            set.push_bind(resolve_outcome(status).to_owned());
        }
        let mut q = update_sql(
            set,
            and(
                Sql::bind("\"wacid\" = ", wacid.to_owned()),
                "\"status\" = ",
                seen.status.clone(),
            ),
        );
        q.push(" RETURNING \"id\"");
        let rows: Vec<i64> = q.query_map(conn, |r| r.get(0))?;
        if !rows.is_empty() {
            return Ok(Some(seen.status));
        }
    }
    Ok(None)
}

/// `markAcceptedIfActive` (no-op from a terminal status).
pub fn mark_accepted_if_active(
    conn: &Connection,
    id: i64,
    answered_by_user_id: i64,
) -> Result<Option<WhatsappCall>> {
    let mut set = Sql::raw("\"status\" = 'accepted', \"answeredByUserId\" = ");
    set.push_bind(answered_by_user_id);
    returning_one(
        conn,
        update_sql(
            set,
            by_id(id).closed(" AND \"status\" NOT IN ('rejected', 'completed', 'failed')"),
        ),
    )
}

/// `touchLivenessIfStale`: bumps `updatedAt` of an `accepted` row not updated since `older_than`.
pub fn touch_liveness_if_stale(conn: &Connection, id: i64, older_than: i64) -> Result<bool> {
    let changed = Sql::new(
        "UPDATE \"WhatsappCall\" SET \"updatedAt\" = ?1 WHERE \"id\" = ?2 AND \"status\" = 'accepted' \
         AND \"updatedAt\" < ?3",
        vec![Value::Integer(now_ms()), Value::Integer(id), Value::Integer(older_than)],
    )
    .execute(conn)?;
    Ok(changed > 0)
}

/// `recoverStrandedAccepted`: terminalizes a stale `accepted` row as `completed`.
pub fn recover_stranded_accepted(
    conn: &Connection,
    id: i64,
    older_than: i64,
    last_error: &str,
) -> Result<Option<WhatsappCall>> {
    let mut set = Sql::raw("\"status\" = 'completed', \"outcome\" = 'completed', \"lastError\" = ");
    set.push_bind(last_error.to_owned());
    let mut guard = by_id(id).closed(" AND \"status\" = 'accepted' AND \"updatedAt\" < ");
    guard.push_bind(older_than);
    returning_one(conn, update_sql(set, guard))
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
    let mut existing = input.current.clone();
    for _ in 0..2 {
        if existing.is_none() {
            existing = find_by_id(conn, input.id)?;
        }
        let Some(seen) = existing.take() else {
            return Ok(None);
        };
        if !can_advance_status(&seen.status, &input.status) {
            if seen.status != input.status {
                return Ok(None);
            }
            return fill_missing_terminal_fields(conn, input, seen).map(Some);
        }
        let mut set = Sql::default();
        let mut first = true;
        let optional: [(&str, Option<Value>); 6] = [
            ("startedAt", input.started_at.map(opt_int)),
            ("endedAt", input.ended_at.map(opt_int)),
            ("durationSeconds", input.duration_seconds.map(opt_int)),
            ("messageId", input.message_id.map(opt_int)),
            (
                "lastError",
                input.last_error.as_ref().map(|v| opt_text(v.as_deref())),
            ),
            ("answeredByUserId", input.answered_by_user_id.map(opt_int)),
        ];
        for (column, value) in optional {
            if let Some(value) = value {
                push_set(&mut set, &mut first, column, value);
            }
        }
        push_set(
            &mut set,
            &mut first,
            "outcome",
            Value::Text(input.outcome.clone()),
        );
        push_set(
            &mut set,
            &mut first,
            "status",
            Value::Text(input.status.clone()),
        );
        let q = update_sql(
            set,
            and(by_id(input.id), "\"status\" = ", seen.status.clone()),
        );
        if let Some(updated) = returning_one(conn, q)? {
            return Ok(Some(updated));
        }
    }
    Ok(None)
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
    let cols = WhatsappCall::columns_sql(Some("w"));
    let n = WhatsappCall::COLUMNS.len();
    let mut q = Sql::raw(format!(
        "SELECT {cols}, c.\"id\", c.\"fullName\", c.\"avatar\", i.\"id\", i.\"name\", \
         au.\"id\", au.\"name\", au.\"email\", iu.\"id\", iu.\"name\", iu.\"email\" \
         FROM \"WhatsappCall\" w \
         JOIN \"ContactInbox\" ci ON ci.\"id\" = w.\"contactInboxId\" \
         JOIN \"Contact\" c ON c.\"id\" = ci.\"contactId\" \
         JOIN \"Inbox\" i ON i.\"id\" = w.\"inboxId\" \
         JOIN \"Conversation\" cv ON cv.\"id\" = w.\"conversationId\" \
         LEFT JOIN \"User\" au ON au.\"id\" = w.\"answeredByUserId\" \
         LEFT JOIN \"User\" iu ON iu.\"id\" = w.\"initiatedByUserId\" \
         WHERE w.\"workspaceId\" = "
    ));
    q.push_bind(input.workspace_id);
    if let WhatsappCallHistoryScope::Own {
        user_id,
        assigned_only,
    } = &input.scope
    {
        q.push(" AND (w.\"answeredByUserId\" = ");
        q.push_bind(*user_id);
        q.push(" OR w.\"initiatedByUserId\" = ");
        q.push_bind(*user_id);
        q.push(")");
        if *assigned_only {
            q.push(" AND cv.\"assignedUserId\" = ");
            q.push_bind(*user_id);
        }
    }
    let f = &input.filters;
    if f.ongoing {
        q.push(" AND w.\"status\" IN ('ringing', 'accepted')");
    }
    if let Some(direction) = &f.direction {
        q.push(" AND w.\"direction\" = ");
        q.push_bind(direction.clone());
    }
    if let Some(inbox_id) = f.inbox_id {
        q.push(" AND w.\"inboxId\" = ");
        q.push_bind(inbox_id);
    }
    if let (WhatsappCallHistoryScope::AllCalls, Some(agent)) = (&input.scope, f.agent_user_id) {
        q.push(" AND (w.\"answeredByUserId\" = ");
        q.push_bind(agent);
        q.push(" OR w.\"initiatedByUserId\" = ");
        q.push_bind(agent);
        q.push(")");
    }
    if let Some(outcome) = &f.outcome {
        q.push(" AND (w.\"outcome\" = ");
        q.push_bind(outcome.clone());
        if let Some(status) = legacy_status_fallback(outcome) {
            q.push(" OR (w.\"outcome\" IS NULL AND w.\"status\" = ");
            q.push_bind(status.to_owned());
            q.push(")");
        }
        q.push(")");
    }
    if let Some(cursor) = &input.cursor {
        let at = parse_iso(&cursor.created_at)
            .ok_or_else(|| Error::msg(format!("invalid cursor createdAt {}", cursor.created_at)))?;
        q.push(" AND (w.\"createdAt\", w.\"id\") < (");
        q.push_bind(at);
        q.push(", ");
        q.push_bind(cursor.id);
        q.push(")");
    }
    q.push(" ORDER BY w.\"createdAt\" DESC, w.\"id\" DESC LIMIT ");
    q.push_bind(input.limit);
    q.query_map(conn, |r| {
        let call = WhatsappCall::from_row(r)?;
        let user = |at: usize| -> rusqlite::Result<Option<CallUser>> {
            let id: Option<i64> = r.get(at)?;
            Ok(match id {
                Some(id) => Some(CallUser {
                    id,
                    name: r.get(at + 1)?,
                    email: r.get(at + 2)?,
                }),
                None => None,
            })
        };
        Ok(WhatsappCallListRow {
            created_at_cursor: iso(call.created_at),
            contact: CallContact {
                id: r.get(n)?,
                full_name: r.get(n + 1)?,
                avatar: r.get(n + 2)?,
            },
            inbox_id: r.get(n + 3)?,
            inbox_name: r.get(n + 4)?,
            answered_by_user: user(n + 5)?,
            initiated_by_user: user(n + 8)?,
            call,
        })
    })
}
