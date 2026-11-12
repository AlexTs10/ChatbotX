//! Port of `repositories/ai-handover-bulk-run/repository.ts`.
//!
//! Every time-dependent query takes `now` (unix ms) instead of SQL `NOW()`.
//! `FOR UPDATE SKIP LOCKED` is not needed: the one writer serializes `pick_due`.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::AiHandoverBulkRun;
use super::sql::Sql;
use super::utils::{PaginationInput, get_pagination_with_defaults};
use super::{Model, Result, new_id, random_uuid};

/// `AI_HANDOVER_BULK_LIVE_STATUSES`.
pub const AI_HANDOVER_BULK_LIVE_STATUSES: &[&str] = &["pending", "running", "cancelling"];

const MINUTE: i64 = 60_000;
/// `releasedStaleInterval` (1 minute).
const RELEASED_STALE_MS: i64 = MINUTE;
/// `leaseStaleInterval` (15 minutes).
const LEASE_STALE_MS: i64 = 15 * MINUTE;
/// A pending run is re-dispatched after 2 minutes.
const PENDING_STALE_MS: i64 = 2 * MINUTE;

/// `CreateAiHandoverBulkRunInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAiHandoverBulkRunInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub channel: String,
    pub revision: i64,
    /// `enable` / `disable`.
    pub action: String,
    pub message: Option<String>,
    pub requested_by_user_id: Option<i64>,
    pub requested_at: i64,
}

/// `AiHandoverBulkRunProgressInput`: `None` leaves a column alone; `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgressInput {
    pub add_processed: Option<i64>,
    pub add_skipped: Option<i64>,
    pub add_failed: Option<i64>,
    pub cursor_contact_inbox_id: Option<Option<String>>,
    pub in_flight_from_id: Option<Option<String>>,
    pub in_flight_to_id: Option<Option<String>>,
    pub total_count: Option<i64>,
    pub current_error: Option<String>,
}

/// `FinishAiHandoverBulkRunInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishInput {
    /// `completed`, `failed` or `cancelled`.
    pub status: String,
    /// `None` keeps the recorded reason; `Some(None)` clears it.
    pub current_error: Option<Option<String>>,
    pub total_count: Option<i64>,
}

/// `PickedAiHandoverBulkRun`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedRun {
    pub id: i64,
    pub workspace_id: i64,
    pub attempts: i64,
    pub chunk_seq: i64,
    pub status: String,
}

/// The `requestedBy` user of [`list_history`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedBy {
    pub id: i64,
    pub name: Option<String>,
    pub email: String,
}

/// One row of [`list_history`].
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryRow {
    pub run: AiHandoverBulkRun,
    pub requested_by: Option<RequestedBy>,
}

/// `pauseElapsed`.
fn pause_elapsed(now: i64) -> Sql {
    Sql::bind("\"pausedUntil\" IS NULL OR \"pausedUntil\" <= ", now)
        .closed(" OR \"status\" = 'cancelling'")
}

fn live_list() -> &'static str {
    "('pending', 'running', 'cancelling')"
}

/// `guardedRun`: the run id, a running/cancelling status and the claim token.
fn guarded(run_id: i64, claim_token: &str) -> Sql {
    Sql::new(
        "\"id\" = ? AND \"status\" IN ('running', 'cancelling') AND \"claimToken\" = ?",
        vec![Value::Integer(run_id), Value::Text(claim_token.to_owned())],
    )
}

/// `createForRevision`: a pending run; `None` when the Page has a live run or already ran
/// this revision (the two partial unique indexes).
pub fn create_for_revision(
    conn: &Connection,
    input: &CreateAiHandoverBulkRunInput,
) -> Result<Option<AiHandoverBulkRun>> {
    let now = super::now_ms();
    let rows = super::returning::<AiHandoverBulkRun>(
        conn,
        "INSERT INTO \"AIHandoverBulkRun\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"inboxId\", \"channel\", \
         \"revision\", \"action\", \"status\", \"message\", \"requestedByUserId\", \"requestedAt\", \"startedAt\", \"finishedAt\", \
         \"lastHeartbeatAt\", \"pausedUntil\", \"claimToken\", \"attempts\", \"chunkSeq\", \"totalCount\", \"processedCount\", \
         \"skippedCount\", \"failedCount\", \"cursorContactInboxId\", \"inFlightFromId\", \"inFlightToId\", \"currentError\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9, ?10, NULL, NULL, NULL, NULL, NULL, 0, 0, NULL, 0, 0, 0, NULL, NULL, NULL, NULL) \
         ON CONFLICT DO NOTHING",
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.inbox_id,
            &input.channel,
            &input.revision,
            &input.action,
            &input.message,
            &input.requested_by_user_id,
            &input.requested_at,
        ],
    )?;
    Ok(rows.into_iter().next())
}

/// `listInboxesAwaitingRun`: Pages a run is due for (connected inbox, a changed
/// revision, ON only with the automation on, no run of the revision and no live run), by
/// inbox id (keyset after `after_inbox_id`).
pub fn list_inboxes_awaiting_run(
    conn: &Connection,
    limit: i64,
    after_inbox_id: Option<i64>,
) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT s.\"workspaceId\", s.\"inboxId\" FROM \"AIHandoverSettings\" AS s \
         INNER JOIN \"Inbox\" AS i ON i.\"id\" = s.\"inboxId\" \
         WHERE s.\"applyToAllRevision\" > 0 AND i.\"status\" = 'connected' \
           AND (?1 IS NULL OR s.\"inboxId\" > ?1) \
           AND (s.\"applyToAllCustomers\" = 0 OR s.\"enabled\" = 1) \
           AND NOT EXISTS (SELECT 1 FROM \"AIHandoverBulkRun\" AS r WHERE r.\"inboxId\" = s.\"inboxId\" \
                AND (r.\"revision\" = s.\"applyToAllRevision\" OR r.\"status\" IN {})) \
         ORDER BY s.\"inboxId\" LIMIT ?2",
        live_list()
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![after_inbox_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `findByRevision`.
pub fn find_by_revision(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    revision: i64,
) -> Result<Option<AiHandoverBulkRun>> {
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"inboxId\" = ?2 AND \"revision\" = ?3",
        &[&workspace_id, &inbox_id, &revision],
    )
}

/// `listHistory`: the Page's runs, newest first, with the requesting user; `page_count`
/// = `ceil(total / limit)`.
pub fn list_history(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    pagination: PaginationInput,
) -> Result<(Vec<HistoryRow>, i64)> {
    let p = get_pagination_with_defaults(pagination);
    let sql = format!(
        "SELECT {}, u.\"id\", u.\"name\", u.\"email\" FROM \"AIHandoverBulkRun\" AS r \
         LEFT JOIN \"User\" AS u ON u.\"id\" = r.\"requestedByUserId\" \
         WHERE r.\"workspaceId\" = ?1 AND r.\"inboxId\" = ?2 ORDER BY r.\"createdAt\" DESC, r.\"id\" DESC LIMIT ?3 OFFSET ?4",
        AiHandoverBulkRun::columns_sql(Some("r"))
    );
    let n = AiHandoverBulkRun::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let data = stmt
        .query_map(
            rusqlite::params![workspace_id, inbox_id, p.limit, p.offset],
            |row| {
                let run = AiHandoverBulkRun::from_row(row)?;
                let id: Option<i64> = row.get(n)?;
                let requested_by = match id {
                    Some(id) => Some(RequestedBy {
                        id,
                        name: row.get(n + 1)?,
                        email: row.get(n + 2)?,
                    }),
                    None => None,
                };
                Ok(HistoryRow { run, requested_by })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM \"AIHandoverBulkRun\" WHERE \"workspaceId\" = ?1 AND \"inboxId\" = ?2",
        [workspace_id, inbox_id],
        |row| row.get(0),
    )?;
    let page_count = if p.limit > 0 {
        (total + p.limit - 1) / p.limit
    } else {
        0
    };
    Ok((data, page_count))
}

/// `claim`: takes the lease of a pending, released or stale-lease live run whose pause is
/// over (a cancelling run always): pending -> running, a new claim token, `startedAt` kept.
pub fn claim(
    conn: &Connection,
    run_id: i64,
    workspace_id: i64,
    now: i64,
) -> Result<Option<AiHandoverBulkRun>> {
    let mut q = Sql::new(
        "UPDATE \"AIHandoverBulkRun\" SET \"status\" = CASE WHEN \"status\" = 'pending' THEN 'running' ELSE \"status\" END, \
         \"claimToken\" = ?, \"pausedUntil\" = NULL, \"startedAt\" = COALESCE(\"startedAt\", ?), \"lastHeartbeatAt\" = ?, \
         \"updatedAt\" = ? WHERE \"id\" = ? AND \"workspaceId\" = ? AND \"status\" IN ('pending', 'running', 'cancelling') AND (",
        vec![
            Value::Text(random_uuid()),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(run_id),
            Value::Integer(workspace_id),
        ],
    );
    q.append(pause_elapsed(now));
    q.push(") AND (\"status\" = 'pending' OR \"claimToken\" IS NULL OR \"lastHeartbeatAt\" < ");
    q.push_bind(now - LEASE_STALE_MS);
    q.push(&format!(
        ") RETURNING {}",
        AiHandoverBulkRun::columns_sql(None)
    ));
    q.model(conn)
}

/// `recordProgress`: adds the settled counts and writes the given cursors under the claim
/// guard; a settled thread (count > 0) resets `attempts`. Returns the status (`running`
/// / `cancelling`), or `None` when the claim was lost.
pub fn record_progress(
    conn: &Connection,
    run_id: i64,
    claim_token: &str,
    progress: &ProgressInput,
    now: i64,
) -> Result<Option<String>> {
    let processed = progress.add_processed.unwrap_or(0);
    let skipped = progress.add_skipped.unwrap_or(0);
    let failed = progress.add_failed.unwrap_or(0);
    let mut q = Sql::new(
        "UPDATE \"AIHandoverBulkRun\" SET \"processedCount\" = \"processedCount\" + ?, \"skippedCount\" = \"skippedCount\" + ?, \
         \"failedCount\" = \"failedCount\" + ?",
        vec![
            Value::Integer(processed),
            Value::Integer(skipped),
            Value::Integer(failed),
        ],
    );
    let set_text = |q: &mut Sql, col: &str, v: &Option<Option<String>>| {
        if let Some(v) = v {
            q.push(&format!(", \"{col}\" = "));
            q.push_bind(v.clone().map_or(Value::Null, Value::Text));
        }
    };
    set_text(
        &mut q,
        "cursorContactInboxId",
        &progress.cursor_contact_inbox_id,
    );
    set_text(&mut q, "inFlightFromId", &progress.in_flight_from_id);
    set_text(&mut q, "inFlightToId", &progress.in_flight_to_id);
    if let Some(total) = progress.total_count {
        q.push(", \"totalCount\" = ");
        q.push_bind(total);
    }
    if let Some(error) = &progress.current_error {
        q.push(", \"currentError\" = ");
        q.push_bind(error.clone());
    }
    if processed + skipped + failed > 0 {
        q.push(", \"attempts\" = 0");
    }
    q.push(", \"lastHeartbeatAt\" = ");
    q.push_bind(now);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now);
    q.push(" WHERE ");
    q.append(guarded(run_id, claim_token));
    q.push(" RETURNING \"status\"");
    Ok(q.query_map(conn, |row| row.get::<_, String>(0))?
        .into_iter()
        .next())
}

/// `yieldForContinuation`: releases the claim and bumps `chunkSeq`; a pause
/// (`paused_until`) also resets `attempts`. Returns the new `chunkSeq`, or `None` when the
/// guard failed.
pub fn yield_for_continuation(
    conn: &Connection,
    run_id: i64,
    claim_token: &str,
    paused_until: Option<i64>,
    now: i64,
) -> Result<Option<i64>> {
    let mut q = Sql::bind(
        "UPDATE \"AIHandoverBulkRun\" SET \"claimToken\" = NULL, \"chunkSeq\" = \"chunkSeq\" + 1, \"pausedUntil\" = ",
        paused_until.map_or(Value::Null, Value::Integer),
    );
    if paused_until.is_some() {
        q.push(", \"attempts\" = 0");
    }
    q.push(", \"lastHeartbeatAt\" = ");
    q.push_bind(now);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now);
    q.push(" WHERE ");
    q.append(guarded(run_id, claim_token));
    q.push(" RETURNING \"chunkSeq\"");
    Ok(q.query_map(conn, |row| row.get::<_, i64>(0))?
        .into_iter()
        .next())
}

/// `finish`: the terminal status; clears the lease, the pause and the in-flight markers.
/// Returns the number of updated rows (0 when the guard failed).
pub fn finish(
    conn: &Connection,
    run_id: i64,
    claim_token: &str,
    outcome: &FinishInput,
    now: i64,
) -> Result<usize> {
    let mut q = Sql::bind(
        "UPDATE \"AIHandoverBulkRun\" SET \"status\" = ",
        outcome.status.clone(),
    );
    if let Some(error) = &outcome.current_error {
        q.push(", \"currentError\" = ");
        q.push_bind(error.clone().map_or(Value::Null, Value::Text));
    }
    if let Some(total) = outcome.total_count {
        q.push(", \"totalCount\" = ");
        q.push_bind(total);
    }
    q.push(", \"claimToken\" = NULL, \"pausedUntil\" = NULL, \"inFlightFromId\" = NULL, \"inFlightToId\" = NULL, \"finishedAt\" = ");
    q.push_bind(now);
    q.push(", \"lastHeartbeatAt\" = ");
    q.push_bind(now);
    q.push(", \"updatedAt\" = ");
    q.push_bind(now);
    q.push(" WHERE ");
    q.append(guarded(run_id, claim_token));
    q.execute(conn)
}

/// `cancelLive`: a run nobody holds (pending, or released) is cancelled outright, a
/// claimed one winds down (`cancelling`); a cancel also ends a pause. Scoped by workspace
/// and inbox (and `action` when given).
pub fn cancel_live(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    action: Option<&str>,
    now: i64,
) -> Result<Option<AiHandoverBulkRun>> {
    let mut q = Sql::new(
        "UPDATE \"AIHandoverBulkRun\" SET \
         \"status\" = CASE WHEN \"status\" = 'pending' OR \"claimToken\" IS NULL THEN 'cancelled' ELSE 'cancelling' END, \
         \"finishedAt\" = CASE WHEN \"status\" = 'pending' OR \"claimToken\" IS NULL THEN ? ELSE \"finishedAt\" END, \
         \"pausedUntil\" = NULL, \"updatedAt\" = ? \
         WHERE \"workspaceId\" = ? AND \"inboxId\" = ? AND \"status\" IN ('pending', 'running')",
        vec![
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(workspace_id),
            Value::Integer(inbox_id),
        ],
    );
    if let Some(action) = action {
        q.push(" AND \"action\" = ");
        q.push_bind(action.to_owned());
    }
    q.push(&format!(
        " RETURNING {}",
        AiHandoverBulkRun::columns_sql(None)
    ));
    q.model(conn)
}

/// `reopenReleased`: a running run with no token (and no pause) goes back to pending.
pub fn reopen_released(conn: &Connection, run_id: i64, now: i64) -> Result<usize> {
    let mut q = Sql::new(
        "UPDATE \"AIHandoverBulkRun\" SET \"status\" = 'pending', \"updatedAt\" = ? \
         WHERE \"id\" = ? AND \"status\" = 'running' AND \"claimToken\" IS NULL AND (",
        vec![Value::Integer(now), Value::Integer(run_id)],
    );
    q.append(pause_elapsed(now));
    q.push(")");
    q.execute(conn)
}

/// `markMaxAttemptsFailed`: ends exhausted live runs whose lease is stale and whose last
/// dispatch had a full lease to start (a cancelling run ends `cancelled`).
pub fn mark_max_attempts_failed(conn: &Connection, max_attempts: i64, now: i64) -> Result<usize> {
    let stale = now - LEASE_STALE_MS;
    let mut q = Sql::new(
        "UPDATE \"AIHandoverBulkRun\" SET \"status\" = CASE WHEN \"status\" = 'cancelling' THEN 'cancelled' ELSE 'failed' END, \
         \"currentError\" = 'maxAttempts', \"claimToken\" = NULL, \"pausedUntil\" = NULL, \"inFlightFromId\" = NULL, \
         \"inFlightToId\" = NULL, \"finishedAt\" = ?, \"updatedAt\" = ? \
         WHERE \"attempts\" >= ? AND \"status\" IN ('pending', 'running', 'cancelling') AND (",
        vec![
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(max_attempts),
        ],
    );
    q.append(pause_elapsed(now));
    q.push(") AND (\"lastHeartbeatAt\" IS NULL OR \"lastHeartbeatAt\" < ");
    q.push_bind(stale);
    q.push(") AND \"updatedAt\" < ");
    q.push_bind(stale);
    q.execute(conn)
}

/// `refundDispatch`: undoes `pick_due`'s counters for exactly the picked values (never
/// below zero).
pub fn refund_dispatch(
    conn: &Connection,
    run_id: i64,
    attempts: i64,
    chunk_seq: i64,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE \"AIHandoverBulkRun\" SET \"attempts\" = \"attempts\" - 1, \"chunkSeq\" = \"chunkSeq\" - 1 \
         WHERE \"id\" = ?1 AND \"attempts\" = ?2 AND \"chunkSeq\" = ?3 AND \"attempts\" > 0 AND \"chunkSeq\" > 0",
        rusqlite::params![run_id, attempts, chunk_seq],
    )?;
    Ok(n > 0)
}

/// `pickDue`: burns an attempt and a revision on up to `batch_size` due live runs (oldest
/// first): pending for 2 minutes, a claimed run with a stale lease, or a released run idle
/// for a minute; pauses are respected (not for cancelling runs).
pub fn pick_due(
    conn: &Connection,
    max_attempts: i64,
    batch_size: i64,
    now: i64,
) -> Result<Vec<PickedRun>> {
    let mut stmt = conn.prepare_cached(
        "UPDATE \"AIHandoverBulkRun\" SET \"attempts\" = \"attempts\" + 1, \"chunkSeq\" = \"chunkSeq\" + 1, \
         \"status\" = CASE WHEN \"status\" = 'cancelling' THEN \"status\" ELSE 'pending' END, \"updatedAt\" = ?1 \
         WHERE \"id\" IN ( \
           SELECT \"id\" FROM \"AIHandoverBulkRun\" \
           WHERE \"status\" IN ('pending', 'running', 'cancelling') AND \"attempts\" < ?2 \
             AND (\"pausedUntil\" IS NULL OR \"pausedUntil\" <= ?1 OR \"status\" = 'cancelling') \
             AND ((\"status\" = 'pending' AND \"updatedAt\" < ?3) \
               OR (\"status\" IN ('running', 'cancelling') AND \"claimToken\" IS NOT NULL AND \"lastHeartbeatAt\" < ?4) \
               OR (\"status\" IN ('running', 'cancelling') AND \"claimToken\" IS NULL AND \"lastHeartbeatAt\" < ?5)) \
           ORDER BY \"createdAt\" ASC LIMIT ?6) \
         RETURNING \"id\", \"workspaceId\", \"attempts\", \"chunkSeq\", \"status\"",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                now,
                max_attempts,
                now - PENDING_STALE_MS,
                now - LEASE_STALE_MS,
                now - RELEASED_STALE_MS,
                batch_size
            ],
            |row| {
                Ok(PickedRun {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    attempts: row.get(2)?,
                    chunk_seq: row.get(3)?,
                    status: row.get(4)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
