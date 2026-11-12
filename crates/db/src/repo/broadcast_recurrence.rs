//! Port of `repositories/broadcast/recurrence-repository.ts`.
//!
//! `FOR UPDATE` locks are not needed (one SQLite writer); the lock functions are plain
//! reads that callers run inside their write transaction.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Broadcast, BroadcastRecurrence, BroadcastRecurrenceOccurrence};
use super::queries::date_bucket::{Granularity, zoned_bucket_key};
use super::sql::Sql;
use super::{Json, Model, Result, new_id, now_ms};

/// `DueRecurrence`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRecurrence {
    pub id: i64,
    pub workspace_id: i64,
    pub next_run_at: i64,
}

/// `DueRecurrenceCursor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueRecurrenceCursor {
    pub next_run_at: i64,
    pub id: i64,
}

/// `LedgerRowInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRowInput {
    pub workspace_id: i64,
    pub recurrence_id: i64,
    pub local_slot: String,
    pub occurrence_at: i64,
    pub outcome: String,
    pub reason: Option<String>,
    pub child_broadcast_id: Option<i64>,
    pub materialized_at: i64,
    pub superseded_count: Option<i64>,
}

/// `LedgerBucketCell`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerBucketCell {
    pub bucket: String,
    pub outcome: String,
    pub reason: Option<String>,
    pub run_status: Option<String>,
    pub slots: i64,
}

/// The `run` of a ledger row (`Pick<BroadcastModel, "id" | "status" | "contactCount">`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRun {
    pub id: i64,
    pub status: String,
    pub contact_count: Option<i64>,
}

/// `LedgerRowWithRun`.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerRowWithRun {
    pub occurrence: BroadcastRecurrenceOccurrence,
    pub run: Option<LedgerRun>,
}

/// The patch of [`update`]: `None` = column untouched; `Some(None)` = set NULL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecurrencePatch {
    pub rule: Option<Json>,
    pub next_run_at: Option<Option<i64>>,
    pub accounted_through_at: Option<Option<i64>>,
    pub occurrence_count: Option<i64>,
    pub last_occurrence_at: Option<Option<i64>>,
    pub paused_at: Option<Option<i64>>,
    pub ended_at: Option<Option<i64>>,
}

/// `selectDbNow`: the database clock (unix ms).
pub fn select_db_now(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER)",
        [],
        |row| row.get(0),
    )?)
}

/// `listDue`: series due at `now`, `(nextRunAt, id)` order, keyset after `cursor`.
pub fn list_due(
    conn: &Connection,
    limit: i64,
    cursor: Option<DueRecurrenceCursor>,
    now: i64,
) -> Result<Vec<DueRecurrence>> {
    let mut q = Sql::bind(
        "SELECT \"id\", \"workspaceId\", \"nextRunAt\" FROM \"BroadcastRecurrence\" WHERE \"nextRunAt\" IS NOT NULL AND \"nextRunAt\" <= ",
        now,
    );
    if let Some(c) = cursor {
        q.push(" AND (\"nextRunAt\", \"id\") > (");
        q.push_bind(c.next_run_at);
        q.push(", ");
        q.push_bind(c.id);
        q.push(")");
    }
    q.push(" ORDER BY \"nextRunAt\" ASC, \"id\" ASC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |row| {
        Ok(DueRecurrence {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            next_run_at: row.get(2)?,
        })
    })
}

/// `findOldestDueAgeMs`: how long the oldest due series has waited (`None` if none).
pub fn find_oldest_due_age_ms(conn: &Connection, now: i64) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT ?1 - min(\"nextRunAt\") FROM \"BroadcastRecurrence\" WHERE \"nextRunAt\" IS NOT NULL AND \"nextRunAt\" <= ?1",
        [now],
        |row| row.get(0),
    )?)
}

/// `lockForWrite`: the series of `broadcast_ids` in the workspace, by id (no query for an
/// empty list).
pub fn lock_for_write(
    conn: &Connection,
    workspace_id: i64,
    broadcast_ids: &[i64],
) -> Result<Vec<BroadcastRecurrence>> {
    if broadcast_ids.is_empty() {
        return Ok(Vec::new());
    }
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"broadcastId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\" ASC",
        &[&workspace_id, &super::id_list(broadcast_ids)],
    )
}

/// `lockAllForWorkspace`.
pub fn lock_all_for_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<BroadcastRecurrence>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"id\" ASC",
        &[&workspace_id],
    )
}

/// `findForMaterialize`: the series of the workspace the job was enqueued for and its
/// template broadcast.
pub fn find_for_materialize(
    conn: &Connection,
    recurrence_id: i64,
    workspace_id: i64,
) -> Result<Option<(BroadcastRecurrence, Broadcast)>> {
    let Some(recurrence) = super::find_one::<BroadcastRecurrence>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&recurrence_id, &workspace_id],
    )?
    else {
        return Ok(None);
    };
    let template = super::find_one::<Broadcast>(conn, "\"id\" = ?1", &[&recurrence.broadcast_id])?;
    Ok(template.map(|t| (recurrence, t)))
}

/// `findByBroadcastId`.
pub fn find_by_broadcast_id(
    conn: &Connection,
    broadcast_id: i64,
) -> Result<Option<BroadcastRecurrence>> {
    super::find_one(conn, "\"broadcastId\" = ?1", &[&broadcast_id])
}

/// `insertIfMissing`: the series of a broadcast (no-op when it exists).
pub fn insert_if_missing(
    conn: &Connection,
    workspace_id: i64,
    broadcast_id: i64,
    rule: &Json,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"BroadcastRecurrence\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"broadcastId\", \"rule\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5) ON CONFLICT (\"broadcastId\") DO NOTHING",
        rusqlite::params![new_id(), now, workspace_id, broadcast_id, rule],
    )?;
    Ok(())
}

/// `update`: applies `patch`, compare-and-set on `nextRunAt` when `expected_next_run_at`
/// is given. `true` when a row matched.
pub fn update(
    conn: &Connection,
    id: i64,
    patch: &RecurrencePatch,
    expected_next_run_at: Option<i64>,
) -> Result<bool> {
    let mut q = Sql::bind(
        "UPDATE \"BroadcastRecurrence\" SET \"updatedAt\" = ",
        now_ms(),
    );
    let opt = |v: Option<i64>| v.map_or(Value::Null, Value::Integer);
    if let Some(rule) = &patch.rule {
        q.push(", \"rule\" = ");
        q.push_bind(rule.0.to_string());
    }
    for (col, v) in [
        ("nextRunAt", patch.next_run_at),
        ("accountedThroughAt", patch.accounted_through_at),
        ("lastOccurrenceAt", patch.last_occurrence_at),
        ("pausedAt", patch.paused_at),
        ("endedAt", patch.ended_at),
    ] {
        if let Some(v) = v {
            q.push(&format!(", \"{col}\" = "));
            q.push_bind(opt(v));
        }
    }
    if let Some(count) = patch.occurrence_count {
        q.push(", \"occurrenceCount\" = ");
        q.push_bind(count);
    }
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    if let Some(expected) = expected_next_run_at {
        q.push(" AND \"nextRunAt\" = ");
        q.push_bind(expected);
    }
    Ok(q.execute(conn)? > 0)
}

/// `insertLedgerRows`: idempotent on `(recurrenceId, localSlot, occurrenceAt)`; an empty
/// batch writes nothing.
pub fn insert_ledger_rows(conn: &Connection, rows: &[LedgerRowInput]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"BroadcastRecurrenceOccurrence\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"recurrenceId\", \"localSlot\", \
         \"occurrenceAt\", \"outcome\", \"reason\", \"childBroadcastId\", \"materializedAt\", \"supersededCount\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, COALESCE(?11, 1)) \
         ON CONFLICT (\"recurrenceId\", \"localSlot\", \"occurrenceAt\") DO NOTHING",
    )?;
    for r in rows {
        stmt.execute(rusqlite::params![
            new_id(),
            now,
            r.workspace_id,
            r.recurrence_id,
            r.local_slot,
            r.occurrence_at,
            r.outcome,
            r.reason,
            r.child_broadcast_id,
            r.materialized_at,
            r.superseded_count
        ])?;
    }
    Ok(())
}

/// `findLedgerRow`.
pub fn find_ledger_row(
    conn: &Connection,
    recurrence_id: i64,
    local_slot: &str,
    occurrence_at: i64,
) -> Result<Option<BroadcastRecurrenceOccurrence>> {
    super::find_one(
        conn,
        "\"recurrenceId\" = ?1 AND \"localSlot\" = ?2 AND \"occurrenceAt\" = ?3",
        &[&recurrence_id, &local_slot, &occurrence_at],
    )
}

/// `countLedgerSlots`: the sum of `supersededCount` (0 without rows).
pub fn count_ledger_slots(conn: &Connection, recurrence_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT coalesce(sum(\"supersededCount\"), 0) FROM \"BroadcastRecurrenceOccurrence\" WHERE \"recurrenceId\" = ?1",
        [recurrence_id],
        |row| row.get(0),
    )?)
}

/// `listLedgerBucketKeys`: the newest bucket keys (day/week/month of `occurrenceAt` in
/// `timezone`) before `cursor`.
pub fn list_ledger_bucket_keys(
    conn: &Connection,
    recurrence_id: i64,
    granularity: Granularity,
    timezone: &str,
    limit: i64,
    cursor: Option<&str>,
) -> Result<Vec<String>> {
    let bucket = zoned_bucket_key("\"occurrenceAt\"", granularity, timezone);
    let mut q = Sql::raw("SELECT ");
    q.append(bucket.clone());
    q.push(" FROM \"BroadcastRecurrenceOccurrence\" WHERE \"recurrenceId\" = ");
    q.push_bind(recurrence_id);
    if let Some(c) = cursor {
        q.push(" AND ");
        q.append(bucket);
        q.push(" < ");
        q.push_bind(c.to_owned());
    }
    q.push(" GROUP BY 1 ORDER BY 1 DESC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |row| row.get(0))
}

/// `listLedgerBucketCells`: slot sums per `(bucket, outcome, reason, run status)` of the
/// given buckets (no query for none).
pub fn list_ledger_bucket_cells(
    conn: &Connection,
    recurrence_id: i64,
    granularity: Granularity,
    timezone: &str,
    buckets: &[String],
) -> Result<Vec<LedgerBucketCell>> {
    if buckets.is_empty() {
        return Ok(Vec::new());
    }
    let bucket = zoned_bucket_key("o.\"occurrenceAt\"", granularity, timezone);
    let mut q = Sql::raw("SELECT ");
    q.append(bucket.clone());
    q.push(
        ", o.\"outcome\", o.\"reason\", b.\"status\", sum(o.\"supersededCount\") FROM \"BroadcastRecurrenceOccurrence\" AS o \
         LEFT JOIN \"Broadcast\" AS b ON b.\"id\" = o.\"childBroadcastId\" WHERE o.\"recurrenceId\" = ",
    );
    q.push_bind(recurrence_id);
    q.push(" AND ");
    q.append(bucket);
    q.push(" IN (SELECT value FROM json_each(");
    q.push_bind(super::text_list(buckets));
    q.push(")) GROUP BY 1, o.\"outcome\", o.\"reason\", b.\"status\"");
    q.query_map(conn, |row| {
        Ok(LedgerBucketCell {
            bucket: row.get(0)?,
            outcome: row.get(1)?,
            reason: row.get(2)?,
            run_status: row.get(3)?,
            slots: row.get(4)?,
        })
    })
}

/// `listLedgerRows`: newest first (`occurrenceAt`, `id`), with the run each created slot
/// produced (`None` for a skipped one).
pub fn list_ledger_rows(
    conn: &Connection,
    recurrence_id: i64,
    from: Option<i64>,
    to: Option<i64>,
    cursor: Option<(i64, i64)>,
    limit: i64,
) -> Result<Vec<LedgerRowWithRun>> {
    let mut q = Sql::raw(format!(
        "SELECT {}, b.\"id\", b.\"status\", b.\"contactCount\" FROM \"BroadcastRecurrenceOccurrence\" AS o \
         LEFT JOIN \"Broadcast\" AS b ON b.\"id\" = o.\"childBroadcastId\" WHERE o.\"recurrenceId\" = ",
        BroadcastRecurrenceOccurrence::columns_sql(Some("o"))
    ));
    q.push_bind(recurrence_id);
    if let Some(f) = from {
        q.push(" AND o.\"occurrenceAt\" >= ");
        q.push_bind(f);
    }
    if let Some(t) = to {
        q.push(" AND o.\"occurrenceAt\" < ");
        q.push_bind(t);
    }
    if let Some((at, id)) = cursor {
        q.push(" AND (o.\"occurrenceAt\", o.\"id\") < (");
        q.push_bind(at);
        q.push(", ");
        q.push_bind(id);
        q.push(")");
    }
    q.push(" ORDER BY o.\"occurrenceAt\" DESC, o.\"id\" DESC LIMIT ");
    q.push_bind(limit);
    let n = BroadcastRecurrenceOccurrence::COLUMNS.len();
    q.query_map(conn, |row| {
        let id: Option<i64> = row.get(n)?;
        let run = match id {
            Some(id) => Some(LedgerRun {
                id,
                status: row.get(n + 1)?,
                contact_count: row.get(n + 2)?,
            }),
            None => None,
        };
        Ok(LedgerRowWithRun {
            occurrence: BroadcastRecurrenceOccurrence::from_row(row)?,
            run,
        })
    })
}
