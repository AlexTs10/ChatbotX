//! `packages/business/src/workspace-lifecycle` (Q.D11 slice 9): the freeze
//! predicates, `drainInBatches`, `freezeWorkspaceRuntime` and
//! `purgeWorkspaceHeavyData`.
//!
//! Each batch is its own write transaction, as in the original. The Postgres
//! `lock_timeout` / `statement_timeout` guards and the analytics slice halving
//! on a timeout have no SQLite counterpart (one writer, no statement timeout),
//! so a slice is always one day.
//!
//! Not here (owned by the channel / integration units): the channel teardown
//! (`disconnectWorkspaceChannels`, connection-state transitions, coexist
//! teardown), `disconnectWorkspaceIntegrations`, `deactivateOwnerWorkspaces`,
//! the recurrence ledger accounting of `disarmSeries`, and the leaderboard
//! Redis drop.

use chatbotx_db::repo::{self, now_ms, workspace_purge as purge};
use chatbotx_db::rusqlite::{self, Connection};
use chatbotx_db::{Db, DbError};

/// `WorkspaceFreezeReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeReason {
    MissingWorkspace,
    OwnerBlocked,
    ScheduledForDeletion,
}

/// `resolveWorkspaceFreezeReason({ workspace, accessState })`: `workspace` is
/// its `scheduledDeletionAt` (`None` for a missing workspace).
pub fn resolve_freeze_reason(
    workspace: Option<Option<i64>>,
    owner_blocked: bool,
) -> Option<FreezeReason> {
    match workspace {
        None => Some(FreezeReason::MissingWorkspace),
        Some(Some(_)) => Some(FreezeReason::ScheduledForDeletion),
        Some(None) if owner_blocked => Some(FreezeReason::OwnerBlocked),
        Some(None) => None,
    }
}

/// `WorkspacePurgeIncompleteError` or a database error.
#[derive(Debug)]
pub enum PurgeError {
    Incomplete { workspace_id: i64, table: String },
    Db(DbError),
}

impl std::fmt::Display for PurgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PurgeError::Incomplete { table, .. } => {
                write!(f, "Workspace purge is incomplete for {table}")
            }
            PurgeError::Db(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PurgeError {}

impl From<DbError> for PurgeError {
    fn from(err: DbError) -> PurgeError {
        PurgeError::Db(err)
    }
}

fn incomplete(workspace_id: i64, table: &str) -> PurgeError {
    PurgeError::Incomplete {
        workspace_id,
        table: table.to_owned(),
    }
}

fn db_err(err: repo::Error) -> DbError {
    match err {
        repo::Error::Db(err) => err,
        other => DbError::other(other.to_string()),
    }
}

const HEAVY_PURGE_BATCH_SIZE: i64 = 5000;
const INTER_CHUNK_DELAY_MS: u64 = 100;
const HYPERTABLE_CONVERSATION_BATCH_SIZE: i64 = 10;
const HEAVY_PURGE_MAX_BATCHES_PER_TABLE: u32 = 2000;
const HYPERTABLE_MAX_CONVERSATION_BATCHES: u32 = 1000;
const ANALYTICS_HYPERTABLE_MAX_SLICES: u32 = 1000;
const ANALYTICS_SLICE_MS: i64 = 24 * 60 * 60 * 1000;
const LEADERBOARD_MAX_BATCHES: u32 = 2000;
const SEQUENCE_ENROLLMENT_CANCEL_BATCH_SIZE: i64 = 500;
const WORKSPACE_DISPATCH_CANCEL_BATCH_SIZE: i64 = 500;
const SMART_DELAY_CANCEL_BATCH_SIZE: i64 = 500;
const FREEZE_MAX_BATCHES: u32 = 400;

/// `HEAVY_PURGE_BATCH_SIZES`.
fn heavy_batch_size(table: &str, default: i64) -> i64 {
    match table {
        "Contact" | "Conversation" => 1000,
        _ => default,
    }
}

/// `assertPurgeDeadline`.
fn assert_deadline(deadline_at: i64, workspace_id: i64, table: &str) -> Result<(), PurgeError> {
    if now_ms() >= deadline_at {
        Err(incomplete(workspace_id, table))
    } else {
        Ok(())
    }
}

/// One `drainInBatches` batch: `(count, value)`.
type Batch<T> = (i64, T);

/// `drainInBatches`: runs `run` (one write transaction each) until a batch is
/// empty or short; `after` sees each non-empty batch's value. The sum of the
/// counts; incomplete at the deadline or after `max_batches` full batches.
#[allow(clippy::too_many_arguments)]
async fn drain<T, R, A>(
    db: &Db,
    workspace_id: i64,
    table: &str,
    batch_size: i64,
    max_batches: u32,
    deadline_at: i64,
    run: R,
    mut after: A,
) -> Result<i64, PurgeError>
where
    T: Send + 'static,
    R: Fn(&Connection) -> repo::Result<Batch<T>> + Clone + Send + 'static,
    A: AsyncFnMut(T),
{
    let mut total = 0;
    for _ in 0..max_batches {
        assert_deadline(deadline_at, workspace_id, table)?;
        let run = run.clone();
        let (count, value) = db.write(move |tx| run(tx).map_err(db_err)).await?;
        total += count;
        if count == 0 {
            return Ok(total);
        }
        after(value).await;
        if count < batch_size {
            return Ok(total);
        }
    }
    Err(incomplete(workspace_id, table))
}

async fn pause() {
    tokio::time::sleep(std::time::Duration::from_millis(INTER_CHUNK_DELAY_MS)).await;
}

/// What the caller removes from Redis / the job queue after a freeze:
/// `removeDispatchesFromSchedule` and the smart-delay `integrationQueue.remove`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FrozenRuntime {
    pub cancelled_broadcasts: usize,
    pub completed_enrollments: i64,
    /// `(id, bucket)` of each canceled pending dispatch.
    pub canceled_dispatches: Vec<(i64, i64)>,
    /// `(id, triggerAt)` of each canceled smart delay (`buildJobId` input).
    pub canceled_smart_delays: Vec<(i64, i64)>,
}

/// `cancelInFlightBroadcastsForWorkspace`: stops every armed recurrence
/// (`disarmSeries(reason: "paused")` without the ledger accounting) and cancels
/// the scheduled, sending and recurring broadcasts.
fn cancel_in_flight_broadcasts(conn: &Connection, workspace_id: i64) -> repo::Result<usize> {
    let now = now_ms();
    for recurrence in repo::broadcast_recurrence::lock_all_for_workspace(conn, workspace_id)? {
        if recurrence.next_run_at.is_some() {
            conn.execute(
                r#"UPDATE "BroadcastRecurrence" SET "nextRunAt" = NULL, "pausedAt" = ?2, "updatedAt" = ?2 WHERE "id" = ?1"#,
                rusqlite::params![recurrence.id, now],
            )?;
        }
    }
    Ok(conn.execute(
        r#"UPDATE "Broadcast" SET "status" = 'cancelled', "updatedAt" = ?2
           WHERE "workspaceId" = ?1 AND "status" IN ('scheduled', 'sending', 'recurring')"#,
        rusqlite::params![workspace_id, now],
    )?)
}

/// `smartDelayService.cancelActiveForWorkspace`.
fn cancel_active_smart_delays(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> repo::Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        r#"UPDATE "ContactOnSmartDelay" SET "status" = 'canceled'
           WHERE "id" IN (SELECT "id" FROM "ContactOnSmartDelay"
             WHERE "workspaceId" = ?1 AND "status" IN ('pending', 'scheduled') LIMIT ?2)
           RETURNING "id", "triggerAt""#,
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, limit], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `freezeWorkspaceRuntime(workspaceId, deadlineAt)`.
pub async fn freeze_workspace_runtime(
    db: &Db,
    workspace_id: i64,
    deadline_at: i64,
) -> Result<FrozenRuntime, PurgeError> {
    let mut out = FrozenRuntime {
        cancelled_broadcasts: db
            .write(move |tx| cancel_in_flight_broadcasts(tx, workspace_id).map_err(db_err))
            .await?,
        ..FrozenRuntime::default()
    };
    out.completed_enrollments = drain(
        db,
        workspace_id,
        "ContactOnSequence",
        SEQUENCE_ENROLLMENT_CANCEL_BATCH_SIZE,
        FREEZE_MAX_BATCHES,
        deadline_at,
        move |c| {
            let n = repo::sequence::complete_active_enrollment_batch(
                c,
                workspace_id,
                SEQUENCE_ENROLLMENT_CANCEL_BATCH_SIZE,
            )?;
            Ok((n as i64, ()))
        },
        async |()| {},
    )
    .await?;
    tracing::info!(
        completed_enrollments = out.completed_enrollments,
        workspace_id,
        "workspace-freeze: completed sequence enrollments"
    );

    let dispatches = &mut out.canceled_dispatches;
    drain(
        db,
        workspace_id,
        "SequenceDispatch",
        WORKSPACE_DISPATCH_CANCEL_BATCH_SIZE,
        FREEZE_MAX_BATCHES,
        deadline_at,
        move |c| {
            let rows = repo::sequence_dispatch::cancel_pending_batch_for_workspace(
                c,
                workspace_id,
                WORKSPACE_DISPATCH_CANCEL_BATCH_SIZE,
            )?;
            Ok((rows.len() as i64, rows))
        },
        async |rows: Vec<(i64, i64)>| dispatches.extend(rows),
    )
    .await?;
    tracing::info!(
        canceled_dispatches = out.canceled_dispatches.len(),
        workspace_id,
        "workspace-freeze: canceled sequence dispatches"
    );

    let delays = &mut out.canceled_smart_delays;
    drain(
        db,
        workspace_id,
        "ContactOnSmartDelay",
        SMART_DELAY_CANCEL_BATCH_SIZE,
        FREEZE_MAX_BATCHES,
        deadline_at,
        move |c| {
            let rows = cancel_active_smart_delays(c, workspace_id, SMART_DELAY_CANCEL_BATCH_SIZE)?;
            Ok((rows.len() as i64, rows))
        },
        async |rows: Vec<(i64, i64)>| delays.extend(rows),
    )
    .await?;
    tracing::info!(
        canceled_smart_delays = out.canceled_smart_delays.len(),
        workspace_id,
        "workspace-freeze: canceled smart delays"
    );
    Ok(out)
}

async fn has_rows(db: &Db, table: &'static str, workspace_id: i64) -> Result<bool, PurgeError> {
    Ok(db
        .read(move |c| purge::has_workspace_rows(c, table, workspace_id).map_err(db_err))
        .await?)
}

/// Drains a direct workspace table, then checks it is empty.
async fn drain_table(
    db: &Db,
    workspace_id: i64,
    table: &'static str,
    batch_size: i64,
    deadline_at: i64,
) -> Result<i64, PurgeError> {
    let deleted = drain(
        db,
        workspace_id,
        table,
        batch_size,
        HEAVY_PURGE_MAX_BATCHES_PER_TABLE,
        deadline_at,
        move |c| {
            let n = purge::delete_workspace_rows_batch(c, table, workspace_id, batch_size)?;
            Ok((n as i64, ()))
        },
        async |()| pause().await,
    )
    .await?;
    if has_rows(db, table, workspace_id).await? {
        return Err(incomplete(workspace_id, table));
    }
    Ok(deleted)
}

/// `purgeWorkspaceHeavyData({ workspaceId, deadlineAt, batchSize })`: the rows
/// deleted.
pub async fn purge_workspace_heavy_data(
    db: &Db,
    workspace_id: i64,
    deadline_at: i64,
    batch_size: Option<i64>,
) -> Result<i64, PurgeError> {
    let default_batch = batch_size.unwrap_or(HEAVY_PURGE_BATCH_SIZE);
    let mut total = delete_analytics_rows(db, workspace_id, deadline_at).await?;
    total += delete_hypertable_rows(db, workspace_id, deadline_at).await?;

    for spec in purge::PARTITIONED_WORKSPACE_TABLES {
        total += drain(
            db,
            workspace_id,
            spec.table,
            default_batch,
            HEAVY_PURGE_MAX_BATCHES_PER_TABLE,
            deadline_at,
            move |c| {
                let n = purge::delete_partitioned_batch(c, spec, workspace_id, default_batch)?;
                Ok((n as i64, ()))
            },
            async |()| pause().await,
        )
        .await?;
        let left = db
            .read(move |c| {
                purge::has_partitioned_workspace_rows(c, spec, workspace_id).map_err(db_err)
            })
            .await?;
        if left {
            return Err(incomplete(workspace_id, spec.table));
        }
    }

    total += purge_leaderboard_score_events(db, workspace_id, default_batch, deadline_at).await?;

    for &table in purge::HEAVY_WORKSPACE_TABLES {
        let size = heavy_batch_size(table, default_batch);
        total += drain_table(db, workspace_id, table, size, deadline_at).await?;
    }

    for &(stage, table) in purge::NO_FK_WORKSPACE_TABLES {
        if stage == "ctid" {
            total += drain_table(db, workspace_id, table, default_batch, deadline_at).await?;
        }
    }
    Ok(total)
}

/// `purgeLeaderboardDataForWorkspace` without the Redis drop.
async fn purge_leaderboard_score_events(
    db: &Db,
    workspace_id: i64,
    batch_size: i64,
    deadline_at: i64,
) -> Result<i64, PurgeError> {
    drain(
        db,
        workspace_id,
        "LeaderboardScoreEvent",
        batch_size,
        LEADERBOARD_MAX_BATCHES,
        deadline_at,
        move |c| {
            let n = c.execute(
                r#"DELETE FROM "LeaderboardScoreEvent" WHERE rowid IN
                   (SELECT rowid FROM "LeaderboardScoreEvent" WHERE "workspaceId" = ?1 LIMIT ?2)"#,
                rusqlite::params![workspace_id, batch_size],
            )?;
            Ok((n as i64, ()))
        },
        async |()| {},
    )
    .await
}

/// `deleteWorkspaceAnalyticsRows`: one-day slices from the oldest row.
async fn delete_analytics_rows(
    db: &Db,
    workspace_id: i64,
    deadline_at: i64,
) -> Result<i64, PurgeError> {
    let mut deleted = 0;
    for &table in purge::WORKSPACE_ANALYTICS_HYPERTABLES {
        for _ in 0..ANALYTICS_HYPERTABLE_MAX_SLICES {
            assert_deadline(deadline_at, workspace_id, table)?;
            let from = db
                .read(move |c| {
                    purge::find_next_analytics_row_at(c, table, workspace_id).map_err(db_err)
                })
                .await?;
            let Some(from) = from else { break };
            let to = from + ANALYTICS_SLICE_MS;
            deleted += db
                .write(move |tx| {
                    purge::delete_analytics_slice(tx, table, workspace_id, from, to).map_err(db_err)
                })
                .await? as i64;
        }
        if has_rows(db, table, workspace_id).await? {
            return Err(incomplete(workspace_id, table));
        }
    }
    Ok(deleted)
}

/// `deleteWorkspaceHypertableRows`: `Message`, then `Attachment`, by pages of
/// conversations.
async fn delete_hypertable_rows(
    db: &Db,
    workspace_id: i64,
    deadline_at: i64,
) -> Result<i64, PurgeError> {
    let mut deleted = 0;
    for table in ["Message", "Attachment"] {
        let mut from_table = 0;
        drain(
            db,
            workspace_id,
            table,
            HYPERTABLE_CONVERSATION_BATCH_SIZE,
            HYPERTABLE_MAX_CONVERSATION_BATCHES,
            deadline_at,
            move |c| {
                let ids = purge::list_message_conversation_page(
                    c,
                    table,
                    workspace_id,
                    HYPERTABLE_CONVERSATION_BATCH_SIZE,
                )?;
                let count = if ids.is_empty() {
                    0
                } else {
                    purge::delete_messages_and_attachments(c, workspace_id, &ids)?
                };
                Ok((ids.len() as i64, count as i64))
            },
            async |count: i64| {
                from_table += count;
                pause().await;
            },
        )
        .await?;
        deleted += from_table;
        if has_rows(db, table, workspace_id).await? {
            return Err(incomplete(workspace_id, table));
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `predicates.ts` (no TS test file; the three branches of the original).
    #[test]
    fn freeze_reason_order() {
        assert_eq!(
            resolve_freeze_reason(None, true),
            Some(FreezeReason::MissingWorkspace)
        );
        assert_eq!(
            resolve_freeze_reason(Some(Some(1)), true),
            Some(FreezeReason::ScheduledForDeletion)
        );
        assert_eq!(
            resolve_freeze_reason(Some(None), true),
            Some(FreezeReason::OwnerBlocked)
        );
        assert_eq!(resolve_freeze_reason(Some(None), false), None);
    }
}
