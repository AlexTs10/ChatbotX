//! Dispatch state changes of the consumer (`apps/worker/src/sequence-scheduler/
//! services/retry-scheduler.service.ts` and `revert-dispatch.ts`) on the
//! P2.4 `SequenceDispatch` table. Each UPDATE is guarded by
//! `id + workspaceId + status = 'running'`.

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;

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
        self.set_terminal(dispatch_id, workspace_id, "failed", error_message)
            .await?;
        tracing::error!(dispatch_id, error_message, "Dispatch marked as failed");
        Ok(())
    }

    /// `status = <status>`, `lastError`, `updatedAt = now` (and `failedAt`
    /// for a failure), only for the running row of that workspace.
    async fn set_terminal(
        &self,
        dispatch_id: &str,
        workspace_id: &str,
        status: &'static str,
        last_error: &str,
    ) -> Result<(), JobsError> {
        let (id, ws) = (number(dispatch_id)?, number(workspace_id)?);
        let now = self.clock.now_ms();
        let last_error = last_error.to_owned();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE "SequenceDispatch" SET "status" = ?3, "lastError" = ?4, "updatedAt" = ?5,
                       "failedAt" = CASE WHEN ?3 = 'failed' THEN ?5 ELSE "failedAt" END
                       WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'running'"#,
                    params![id, ws, status, last_error, now],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// `status = 'canceled'`, `lastError = reason`, `updatedAt = now`; no
    /// error log.
    pub async fn mark_dispatch_canceled(
        &self,
        dispatch_id: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<(), JobsError> {
        self.set_terminal(dispatch_id, workspace_id, "canceled", reason)
            .await
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
    let (id, ws) = (number(dispatch_id)?, number(workspace_id)?);
    let now = clock.now_ms();
    db.write(move |tx| {
        tx.execute(
            r#"UPDATE "SequenceDispatch" SET "status" = 'pending', "lockedAt" = NULL,
               "lockOwner" = NULL, "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'running'"#,
            params![id, ws, now],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

fn number(id: &str) -> Result<i64, JobsError> {
    id.trim()
        .parse()
        .map_err(|_| JobsError::InvalidOptions(format!("invalid id {id:?}")))
}
