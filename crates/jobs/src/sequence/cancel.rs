//! `dispatch-cancel.ts` and `sequence-dispatch.ts`
//! (`sequenceDispatchUtils.bulkCancelPendingDispatches`).

use chatbotx_db::rusqlite::Connection;

use chatbotx_db::rusqlite::params;

use crate::sequence::repo::{
    SequenceCtx, SequenceError, StatementKind, TxObserver, note, parse_id, write_tx,
};
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
        note(
            self.observer.as_deref(),
            StatementKind::Select,
            "SequenceDispatch",
        );
        let rows = conn
            .prepare_cached(
                r#"SELECT "id", "bucket", "sequenceId", "contactId", "stepId" FROM "SequenceDispatch"
                   WHERE "enrollmentId" = ?1 AND "workspaceId" = ?2 AND "status" = 'pending'
                   ORDER BY "id""#,
            )?
            .query_map(
                params![parse_id(enrollment_id)?, parse_id(workspace_id)?],
                |r| {
                    Ok(PendingDispatch {
                        id: r.get::<_, i64>(0)?.to_string(),
                        bucket: r.get(1)?,
                        sequence_id: r.get::<_, i64>(2)?.to_string(),
                        contact_id: r.get::<_, i64>(3)?.to_string(),
                        step_id: r.get::<_, i64>(4)?.to_string(),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn cancel_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        ids: &[String],
    ) -> Result<Vec<DispatchRef>, SequenceError> {
        note(
            self.observer.as_deref(),
            StatementKind::Update,
            "SequenceDispatch",
        );
        let ids: Vec<i64> = ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<_, _>>()?;
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        // Integer ids only: safe to inline. Guarded by workspace + status.
        let sql = format!(
            r#"UPDATE "SequenceDispatch" SET "status" = 'canceled',
               "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
               WHERE "id" IN ({list}) AND "workspaceId" = ?1 AND "status" = 'pending'
               RETURNING "id", "bucket""#
        );
        let mut rows: Vec<DispatchRef> = conn
            .prepare(&sql)?
            .query_map(params![parse_id(workspace_id)?], |r| {
                Ok(DispatchRef {
                    id: r.get::<_, i64>(0)?.to_string(),
                    bucket: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        rows.sort_by_key(|d| d.id.parse::<i64>().unwrap_or_default());
        Ok(rows)
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
    let pending = repo.find_pending(conn, workspace_id, enrollment_id)?;
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = pending.iter().map(|d| d.id.clone()).collect();
    if repo.cancel_pending(conn, workspace_id, &ids)?.is_empty() {
        return Ok(Vec::new());
    }
    Ok(pending
        .into_iter()
        .map(|d| DispatchRef {
            id: d.id,
            bucket: d.bucket,
        })
        .collect())
}

/// `cancelPendingDispatches({ ..., client })` on a caller connection: the
/// database part only (the caller removes the dispatches from the schedule
/// after its commit).
pub fn cancel_pending_dispatches_in(
    conn: &Connection,
    observer: Option<&dyn TxObserver>,
    params: &CancelPendingParams,
) -> Result<Vec<DispatchRef>, SequenceError> {
    let repo = ObservedRepo { observer };
    let pending = repo.find_pending(conn, &params.workspace_id, &params.enrollment_id)?;
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = pending.iter().map(|d| d.id.clone()).collect();
    repo.cancel_pending(conn, &params.workspace_id, &ids)?;
    Ok(pending
        .into_iter()
        .map(|d| DispatchRef {
            id: d.id,
            bucket: d.bucket,
        })
        .collect())
}

/// [`SqlPendingDispatchRepo`] with a borrowed observer.
struct ObservedRepo<'a> {
    observer: Option<&'a dyn TxObserver>,
}

impl ObservedRepo<'_> {
    fn sql(&self) -> SqlPendingDispatchRepo {
        SqlPendingDispatchRepo { observer: None }
    }

    fn find_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        enrollment_id: &str,
    ) -> Result<Vec<PendingDispatch>, SequenceError> {
        note(self.observer, StatementKind::Select, "SequenceDispatch");
        self.sql().find_pending(conn, workspace_id, enrollment_id)
    }

    fn cancel_pending(
        &self,
        conn: &Connection,
        workspace_id: &str,
        ids: &[String],
    ) -> Result<Vec<DispatchRef>, SequenceError> {
        note(self.observer, StatementKind::Update, "SequenceDispatch");
        self.sql().cancel_pending(conn, workspace_id, ids)
    }
}

/// `cancelPendingDispatches(params)`: cancel in one write on `ctx.db`, then
/// (unless deferred) remove each canceled dispatch from `ctx.schedule`.
pub async fn cancel_pending_dispatches(
    ctx: &SequenceCtx,
    params: CancelPendingParams,
) -> Result<Vec<DispatchRef>, SequenceError> {
    let observer = ctx.observer.clone();
    let in_tx = params.clone();
    let canceled = write_tx(&ctx.db, move |conn| {
        cancel_pending_dispatches_in(conn, observer.as_deref(), &in_tx)
    })
    .await?;
    if params.remove_from_schedule {
        remove_dispatches_from_schedule(ctx.schedule.as_ref(), &canceled).await?;
    }
    Ok(canceled)
}

/// `removeDispatchesFromSchedule(dispatches)`: tries every removal, then
/// fails with `SequenceError::ScheduleRemoval` naming each failed
/// `<id> bucket=<bucket>`.
pub async fn remove_dispatches_from_schedule(
    schedule: &dyn DispatchSchedule,
    dispatches: &[DispatchRef],
) -> Result<(), SequenceError> {
    let mut failed = Vec::new();
    for dispatch in dispatches {
        if let Err(err) = schedule
            .remove_from_schedule(dispatch.bucket, &dispatch.id)
            .await
        {
            tracing::warn!(
                dispatch_id = %dispatch.id,
                bucket = dispatch.bucket,
                err = %err,
                "Failed to remove sequence dispatch from schedule"
            );
            failed.push((dispatch.id.clone(), dispatch.bucket));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(SequenceError::ScheduleRemoval { failed })
    }
}
