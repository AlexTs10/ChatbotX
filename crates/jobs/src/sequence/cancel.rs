//! `dispatch-cancel.ts` and `sequence-dispatch.ts`
//! (`sequenceDispatchUtils.bulkCancelPendingDispatches`).

use chatbotx_db::rusqlite::Connection;

use crate::sequence::repo::{SequenceCtx, SequenceError, TxObserver};
use crate::sequence::schedule::DispatchSchedule;

/// `{ id, bucket }` of a canceled dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRef {
    pub id: String,
    pub bucket: u32,
}

/// A pending dispatch as `findMany` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDispatch {
    pub id: String,
    pub bucket: u32,
    pub sequence_id: String,
    pub contact_id: String,
    pub step_id: String,
}

/// `cancelPendingDispatches` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelPendingParams {
    pub enrollment_id: String,
    pub workspace_id: String,
    /// `removeFromSchedule` option (default `true`): `false` defers the
    /// schedule removal to the caller (after its commit).
    pub remove_from_schedule: bool,
}

impl CancelPendingParams {
    /// `remove_from_schedule = true`.
    pub fn new(enrollment_id: &str, workspace_id: &str) -> CancelPendingParams {
        CancelPendingParams {
            enrollment_id: enrollment_id.to_owned(),
            workspace_id: workspace_id.to_owned(),
            remove_from_schedule: true,
        }
    }
}

/// The two statements of a bulk cancel, as a repository (fakes in tests).
pub trait PendingDispatchRepo: Send + Sync {
    /// `findMany` where `enrollmentId`, `workspaceId`, `status = 'pending'`.
    fn find_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        enrollment_id: &str,
    ) -> Result<Vec<PendingDispatch>, SequenceError>;

    /// One `UPDATE ... SET status = 'canceled' WHERE id IN (..) AND
    /// workspaceId = ? AND status = 'pending' RETURNING id, bucket`.
    fn cancel_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        ids: &[String],
    ) -> Result<Vec<DispatchRef>, SequenceError>;
}

/// The SQL repository.
#[derive(Clone, Default)]
pub struct SqlPendingDispatchRepo {
    pub observer: Option<std::sync::Arc<dyn TxObserver>>,
}

impl PendingDispatchRepo for SqlPendingDispatchRepo {
    fn find_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        enrollment_id: &str,
    ) -> Result<Vec<PendingDispatch>, SequenceError> {
        todo!()
    }

    fn cancel_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        ids: &[String],
    ) -> Result<Vec<DispatchRef>, SequenceError> {
        todo!()
    }
}

/// `sequenceDispatchUtils.bulkCancelPendingDispatches({ dbClient,
/// workspaceId, enrollmentId })`: no pending row = no UPDATE; an UPDATE
/// that matches nothing (concurrent cancel) = empty result.
pub fn bulk_cancel_pending_dispatches_in(
    conn: &Connection,
    repo: &dyn PendingDispatchRepo,
    workspace_id: &str,
    enrollment_id: &str,
) -> Result<Vec<DispatchRef>, SequenceError> {
    todo!()
}

/// `cancelPendingDispatches({ ..., client })` on a caller connection: the
/// database part only (the caller removes the dispatches from the schedule
/// after its commit).
pub fn cancel_pending_dispatches_in(
    conn: &Connection,
    observer: Option<&dyn TxObserver>,
    params: &CancelPendingParams,
) -> Result<Vec<DispatchRef>, SequenceError> {
    todo!()
}

/// `cancelPendingDispatches(params)`: cancel in one write on `ctx.db`, then
/// (unless deferred) remove each canceled dispatch from `ctx.schedule`.
pub async fn cancel_pending_dispatches(
    ctx: &SequenceCtx,
    params: CancelPendingParams,
) -> Result<Vec<DispatchRef>, SequenceError> {
    todo!()
}

/// `removeDispatchesFromSchedule(dispatches)`: tries every removal, then
/// fails with `SequenceError::ScheduleRemoval` naming each failed
/// `<id> bucket=<bucket>`.
pub async fn remove_dispatches_from_schedule(
    schedule: &dyn DispatchSchedule,
    dispatches: &[DispatchRef],
) -> Result<(), SequenceError> {
    todo!()
}
