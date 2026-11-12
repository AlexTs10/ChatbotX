//! Port of `repositories/sequence-dispatch/repository.ts` (P3.3). `FOR UPDATE SKIP LOCKED`
//! is not needed under the one SQLite writer.

use rusqlite::Connection;

use super::models::{Contact, ContactOnSequence, Flow, Sequence, SequenceDispatch, SequenceStep};
use super::{Result, id_list, now_ms};

/// `DispatchWithRelations`.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchWithRelations {
    pub dispatch: SequenceDispatch,
    pub sequence: Option<Sequence>,
    pub contact: Option<Contact>,
    pub enrollment: Option<ContactOnSequence>,
}

/// `SequenceStepWithFlow`.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceStepWithFlow {
    pub step: SequenceStep,
    pub flow: Option<Flow>,
}

/// One row of [`list_pending_for_reconcile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingForReconcile {
    pub id: i64,
    pub bucket: i64,
    pub run_at_ms: i64,
    pub workspace_id: i64,
    pub contact_id: i64,
}

/// `findWithRelations`.
pub fn find_with_relations(
    conn: &Connection,
    id: i64,
    status: &str,
    workspace_id: i64,
) -> Result<Option<DispatchWithRelations>> {
    let Some(dispatch) = super::find_one::<SequenceDispatch>(
        conn,
        "\"id\" = ?1 AND \"status\" = ?2 AND \"workspaceId\" = ?3",
        &[&id, &status, &workspace_id],
    )?
    else {
        return Ok(None);
    };
    let sequence = super::find_one::<Sequence>(conn, "\"id\" = ?1", &[&dispatch.sequence_id])?;
    let contact = super::find_one::<Contact>(conn, "\"id\" = ?1", &[&dispatch.contact_id])?;
    let enrollment = super::find_one::<ContactOnSequence>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&dispatch.enrollment_id, &dispatch.workspace_id],
    )?;
    Ok(Some(DispatchWithRelations {
        dispatch,
        sequence,
        contact,
        enrollment,
    }))
}

/// `claim`: CAS `pending` -> `running` scoped by id AND workspace; `true` when claimed.
pub fn claim(conn: &Connection, id: i64, workspace_id: i64, lock_owner: &str) -> Result<bool> {
    let now = now_ms();
    let updated = conn.execute(
        "UPDATE \"SequenceDispatch\" SET \"status\" = 'running', \"lockedAt\" = ?4, \"lockOwner\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'pending'",
        rusqlite::params![id, workspace_id, lock_owner, now],
    )?;
    Ok(updated > 0)
}

/// `listPendingWorkspaceIds`: `(id, workspaceId)` of the pending ones of `ids`.
pub fn list_pending_workspace_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, i64)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"workspaceId\" FROM \"SequenceDispatch\" \
         WHERE \"id\" IN (SELECT value FROM json_each(?1)) AND \"status\" = 'pending'",
    )?;
    let rows = stmt
        .query_map([id_list(ids)], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listPendingForReconcile`: pending rows due by `max_run_at_ms`, by `runAtMs` ascending.
pub fn list_pending_for_reconcile(
    conn: &Connection,
    max_run_at_ms: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<PendingForReconcile>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"bucket\", \"runAtMs\", \"workspaceId\", \"contactId\" FROM \"SequenceDispatch\" \
         WHERE \"status\" = 'pending' AND \"runAtMs\" <= ?1 ORDER BY \"runAtMs\" ASC LIMIT ?3 OFFSET ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![max_run_at_ms, offset, limit], |row| {
            Ok(PendingForReconcile {
                id: row.get(0)?,
                bucket: row.get(1)?,
                run_at_ms: row.get(2)?,
                workspace_id: row.get(3)?,
                contact_id: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listPendingIds`: which of `ids` are still pending.
pub fn list_pending_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"SequenceDispatch\" WHERE \"id\" IN (SELECT value FROM json_each(?1)) AND \"status\" = 'pending'",
    )?;
    let rows = stmt
        .query_map([id_list(ids)], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// `deleteTerminalBatch`: deletes up to `batch_size` terminal rows (`completed`,
/// `failed`, `canceled`) last updated more than `retention_ttl_days` before `now`.
pub fn delete_terminal_batch(
    conn: &Connection,
    retention_ttl_days: i64,
    batch_size: i64,
    now: i64,
) -> Result<usize> {
    let cutoff = now - retention_ttl_days * 86_400_000;
    Ok(conn.execute(
        "DELETE FROM \"SequenceDispatch\" WHERE (\"id\", \"workspaceId\") IN ( \
           SELECT \"id\", \"workspaceId\" FROM \"SequenceDispatch\" \
           WHERE \"status\" IN ('completed', 'failed', 'canceled') AND \"updatedAt\" < ?1 LIMIT ?2)",
        rusqlite::params![cutoff, batch_size],
    )?)
}

/// `cancelPendingBatchForWorkspace`: cancels up to `limit` pending rows of the workspace;
/// returns their `(id, bucket)`.
pub fn cancel_pending_batch_for_workspace(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "UPDATE \"SequenceDispatch\" SET \"status\" = 'canceled', \"updatedAt\" = ?3 \
         WHERE (\"id\", \"workspaceId\") IN (SELECT \"id\", \"workspaceId\" FROM \"SequenceDispatch\" \
           WHERE \"workspaceId\" = ?1 AND \"status\" = 'pending' LIMIT ?2) \
         RETURNING \"id\", \"bucket\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, limit, now_ms()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `findStepWithFlow`.
pub fn find_step_with_flow(conn: &Connection, id: i64) -> Result<Option<SequenceStepWithFlow>> {
    let Some(step) = super::find_one::<SequenceStep>(conn, "\"id\" = ?1", &[&id])? else {
        return Ok(None);
    };
    let flow = match step.flow_id {
        Some(flow_id) => super::find_one::<Flow>(conn, "\"id\" = ?1", &[&flow_id])?,
        None => None,
    };
    Ok(Some(SequenceStepWithFlow { step, flow }))
}
