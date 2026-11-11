//! Dispatch state changes of the consumer (`apps/worker/src/sequence-scheduler/
//! services/retry-scheduler.service.ts` and `revert-dispatch.ts`) on the
//! P2.4 `SequenceDispatch` table. Each UPDATE is guarded by
//! `id + workspaceId + status = 'running'`.

use std::sync::Arc;

use chatbotx_db::Db;

use crate::clock::Clock;
use crate::enqueue::JobsError;

/// `RetrySchedulerService`.
pub struct RetrySchedulerService {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
}

impl RetrySchedulerService {
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> RetrySchedulerService {
        RetrySchedulerService { db, clock }
    }

    /// `status = 'failed'`, `lastError = error_message`, `updatedAt = now`;
    /// logs an error `{dispatch_id, error_message}`.
    pub async fn mark_dispatch_failed(
        &self,
        dispatch_id: &str,
        workspace_id: &str,
        error_message: &str,
    ) -> Result<(), JobsError> {
        todo!()
    }

    /// `status = 'canceled'`, `lastError = reason`, `updatedAt = now`; no
    /// error log.
    pub async fn mark_dispatch_canceled(
        &self,
        dispatch_id: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<(), JobsError> {
        todo!()
    }
}

/// `revertDispatchToPending(dispatchId, workspaceId)`: `status = 'pending'`,
/// `lockedAt = NULL`, `lockOwner = NULL`, `updatedAt = now`, only for the
/// running row of that workspace.
pub async fn revert_dispatch_to_pending(
    db: &Db,
    clock: &dyn Clock,
    dispatch_id: &str,
    workspace_id: &str,
) -> Result<(), JobsError> {
    todo!()
}
