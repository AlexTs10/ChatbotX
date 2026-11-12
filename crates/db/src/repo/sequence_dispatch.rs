//! Port of `repositories/sequence-dispatch/repository.ts` (P3.3). `FOR UPDATE SKIP LOCKED`
//! is not needed under the one SQLite writer.
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
}

/// `claim`: CAS `pending` -> `running` scoped by id AND workspace; `true` when claimed.
pub fn claim(conn: &Connection, id: i64, workspace_id: i64, lock_owner: &str) -> Result<bool> {
    todo!("P3.3")
}

/// `listPendingWorkspaceIds`: `(id, workspaceId)` of the pending ones of `ids`.
pub fn list_pending_workspace_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, i64)>> {
    todo!("P3.3")
}

/// `listPendingForReconcile`: pending rows due by `max_run_at_ms`, by `runAtMs` ascending.
pub fn list_pending_for_reconcile(
    conn: &Connection,
    max_run_at_ms: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<PendingForReconcile>> {
    todo!("P3.3")
}

/// `listPendingIds`: which of `ids` are still pending.
pub fn list_pending_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `deleteTerminalBatch`: deletes up to `batch_size` terminal rows (`completed`,
/// `failed`, `canceled`) last updated more than `retention_ttl_days` before `now`.
pub fn delete_terminal_batch(
    conn: &Connection,
    retention_ttl_days: i64,
    batch_size: i64,
    now: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// `cancelPendingBatchForWorkspace`: cancels up to `limit` pending rows of the workspace;
/// returns their `(id, bucket)`.
pub fn cancel_pending_batch_for_workspace(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    todo!("P3.3")
}

/// `findStepWithFlow`.
pub fn find_step_with_flow(conn: &Connection, id: i64) -> Result<Option<SequenceStepWithFlow>> {
    todo!("P3.3")
}
