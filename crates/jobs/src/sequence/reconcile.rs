//! `ReconcileJob` (`apps/worker/src/sequence-scheduler/worker.ts`, spec 3.1):
//! re-adds pending dispatches due within 24 h to the schedule (every hour),
//! removes schedule members without a pending row (every 6 h), and deletes
//! old terminal dispatches in batches (every 24 h).

use std::sync::Arc;
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;

use crate::clock::Clock;
use crate::enqueue::{BoxFuture, JobsError};
use crate::sequence::buckets::{SCHEDULER_BUCKET_RANGE_ENV, assigned_buckets};
use crate::sequence::schedule::{DispatchSchedule, ScheduleKind};
use crate::shutdown::CancelToken;

/// `BOOTSTRAP_INTERVAL_MS`.
pub const RECONCILE_INTERVAL_MS: u64 = 3_600_000;
/// `CLEANUP_INTERVAL_MS`.
pub const CLEANUP_INTERVAL_MS: u64 = 21_600_000;
/// `SEQUENCE_DISPATCH_RETENTION_INTERVAL_MS`.
pub const RETENTION_INTERVAL_MS: u64 = 86_400_000;
/// `SEQUENCE_DISPATCH_RETENTION_TTL_DAYS` default.
pub const RETENTION_TTL_DAYS: u32 = 30;
/// Retention and reconcile batch size.
pub const RECONCILE_BATCH_SIZE: u32 = 1000;
/// The reconcile window: pending dispatches due within 24 h.
pub const RECONCILE_WINDOW_MS: i64 = 86_400_000;
/// The pause at the end of a reconcile pass.
pub const RECONCILE_PAUSE_MS: u64 = 1000;

/// Constructor options (`new ReconcileJob({...})`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileOptions {
    pub interval_ms: Option<u64>,
    pub cleanup_interval_ms: Option<u64>,
    pub retention_interval_ms: Option<u64>,
    pub retention_batch_size: Option<u32>,
    pub retention_ttl_days: Option<u32>,
    /// `None` = `assigned_buckets(SCHEDULER_BUCKET_RANGE)`.
    pub buckets: Option<Vec<u32>>,
}

/// Sort order of the reconcile scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAtOrder {
    Asc,
}

/// The reconcile scan: `status = 'pending' AND runAtMs <= run_at_ms_lte`
/// (no workspace filter), `ORDER BY runAtMs`, `LIMIT limit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDueQuery {
    pub run_at_ms_lte: i64,
    pub workspace_id: Option<String>,
    pub limit: u32,
    pub order: RunAtOrder,
    /// Paging cursor: rows after this `(run_at_ms, id)`.
    pub after: Option<(i64, String)>,
}

/// One pending dispatch to re-schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDue {
    pub id: String,
    pub bucket: u32,
    pub run_at_ms: i64,
}

/// The dispatch reads of the reconcile job.
pub trait ReconcileRepo: Send + Sync {
    fn find_pending_due(
        &self,
        query: PendingDueQuery,
    ) -> BoxFuture<'_, Result<Vec<PendingDue>, JobsError>>;
    /// Of `ids`, the ones that have a pending row (`columns: {id}`,
    /// `id IN ids AND status = 'pending'`).
    fn find_pending_ids<'a>(
        &'a self,
        ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, JobsError>>;
}

/// Collaborators of the reconcile job.
#[derive(Clone)]
pub struct ReconcileDeps {
    pub schedule: Arc<dyn DispatchSchedule>,
    pub repo: Arc<dyn ReconcileRepo>,
    /// The database for the retention delete (`SequenceDispatch`).
    pub db: Option<Arc<Db>>,
    pub clock: Arc<dyn Clock>,
}

/// Result of one retention pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionOutcome {
    /// Rows deleted.
    pub deleted: u64,
    /// DELETE statements run (one per batch).
    pub batches: u32,
}

/// `ReconcileJob`.
pub struct ReconcileJob {
    options: ResolvedOptions,
    deps: ReconcileDeps,
    buckets: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedOptions {
    interval_ms: u64,
    cleanup_interval_ms: u64,
    retention_interval_ms: u64,
    retention_batch_size: u32,
    retention_ttl_days: u32,
}

impl ReconcileJob {
    pub fn new(options: ReconcileOptions, deps: ReconcileDeps) -> ReconcileJob {
        let nonzero = |v: Option<u64>, default: u64| v.filter(|v| *v != 0).unwrap_or(default);
        let buckets = options.buckets.clone().unwrap_or_else(|| {
            assigned_buckets(std::env::var(SCHEDULER_BUCKET_RANGE_ENV).ok().as_deref())
        });
        ReconcileJob {
            options: ResolvedOptions {
                interval_ms: nonzero(options.interval_ms, RECONCILE_INTERVAL_MS),
                cleanup_interval_ms: nonzero(options.cleanup_interval_ms, CLEANUP_INTERVAL_MS),
                retention_interval_ms: nonzero(
                    options.retention_interval_ms,
                    RETENTION_INTERVAL_MS,
                ),
                retention_batch_size: options
                    .retention_batch_size
                    .filter(|v| *v != 0)
                    .unwrap_or(RECONCILE_BATCH_SIZE),
                retention_ttl_days: options
                    .retention_ttl_days
                    .filter(|v| *v != 0)
                    .unwrap_or(RETENTION_TTL_DAYS),
            },
            deps,
            buckets,
        }
    }

    /// Re-adds every pending dispatch due within 24 h to its bucket's
    /// schedule set, in batches of 1000, then pauses 1000 ms.
    pub async fn reconcile(&self) -> Result<(), JobsError> {
        let run_at_ms_lte = self.deps.clock.now_ms().saturating_add(RECONCILE_WINDOW_MS);
        let mut after: Option<(i64, String)> = None;
        loop {
            let rows = self
                .deps
                .repo
                .find_pending_due(PendingDueQuery {
                    run_at_ms_lte,
                    workspace_id: None,
                    limit: RECONCILE_BATCH_SIZE,
                    order: RunAtOrder::Asc,
                    after: after.clone(),
                })
                .await
                .inspect_err(|err| tracing::error!(err = %err, "Error in reconciliation"))?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                // `allSettled`: one failed add does not stop the pass.
                if let Err(err) = self
                    .deps
                    .schedule
                    .add_to_schedule(row.bucket, &row.id, row.run_at_ms)
                    .await
                {
                    tracing::warn!(err = %err, dispatch_id = %row.id, "reconcile add failed");
                }
            }
            if rows.len() < RECONCILE_BATCH_SIZE as usize {
                tokio::time::sleep(Duration::from_millis(RECONCILE_PAUSE_MS)).await;
                break;
            }
            after = rows.last().map(|r| (r.run_at_ms, r.id.clone()));
        }
        Ok(())
    }

    /// Removes members of both sets (each assigned bucket) that have no
    /// pending row (`remove_from_all`).
    pub async fn cleanup_orphans(&self) -> Result<(), JobsError> {
        for &bucket in &self.buckets {
            let mut ids: Vec<String> = Vec::new();
            for kind in [ScheduleKind::Schedule, ScheduleKind::Retry] {
                match self.deps.schedule.members(kind, bucket).await {
                    Ok(members) => {
                        for id in members {
                            if !ids.contains(&id) {
                                ids.push(id);
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!(err = %err, bucket, kind = ?kind, "Error getting ZSET members");
                    }
                }
            }
            if ids.is_empty() {
                continue;
            }
            let valid =
                self.deps.repo.find_pending_ids(&ids).await.inspect_err(
                    |err| tracing::error!(err = %err, "Error during orphan cleanup"),
                )?;
            for id in ids.iter().filter(|id| !valid.contains(id)) {
                if let Err(err) = self.deps.schedule.remove_from_all(bucket, id).await {
                    tracing::warn!(err = %err, dispatch_id = %id, "orphan removal failed");
                }
            }
        }
        Ok(())
    }

    /// `DELETE FROM "SequenceDispatch" WHERE "status" IN (terminal) AND older
    /// than the TTL ... LIMIT batch`, repeated until a batch is short.
    pub async fn delete_terminal_dispatches(&self) -> Result<RetentionOutcome, JobsError> {
        let Some(db) = self.deps.db.clone() else {
            return Err(JobsError::Unavailable(
                "sequence dispatch retention needs the database".to_owned(),
            ));
        };
        let ttl_days = i64::from(self.options.retention_ttl_days.max(1));
        let batch = i64::from(self.options.retention_batch_size.max(1));
        let cutoff = self
            .deps
            .clock
            .now_ms()
            .saturating_sub(ttl_days * 86_400_000);
        let mut outcome = RetentionOutcome {
            deleted: 0,
            batches: 0,
        };
        loop {
            let n = db
                .write(move |tx| {
                    Ok(tx.execute(
                        r#"DELETE FROM "SequenceDispatch" WHERE "id" IN (
                             SELECT "id" FROM "SequenceDispatch"
                             WHERE "status" IN ('completed', 'failed', 'canceled') AND "updatedAt" < ?1
                             LIMIT ?2)"#,
                        params![cutoff, batch],
                    )? as u64)
                })
                .await?;
            outcome.deleted += n;
            outcome.batches += 1;
            if (n as i64) < batch {
                break;
            }
        }
        Ok(outcome)
    }

    /// The periodic loops (`start`): one reconcile now, then reconcile,
    /// orphan cleanup and retention at their intervals until `stop`.
    pub async fn run(self: Arc<Self>, stop: CancelToken) {
        if let Err(err) = self.reconcile().await {
            tracing::error!(err = %err, "Error in reconcile job");
        }
        let mut reconcile = interval(self.options.interval_ms);
        let mut cleanup = interval(self.options.cleanup_interval_ms);
        let mut retention = interval(self.options.retention_interval_ms);
        loop {
            tokio::select! {
                _ = reconcile.tick() => {
                    if let Err(err) = self.reconcile().await {
                        tracing::error!(err = %err, "Error in reconcile job");
                    }
                }
                _ = cleanup.tick() => {
                    if let Err(err) = self.cleanup_orphans().await {
                        tracing::error!(err = %err, "Error in cleanup job");
                    }
                }
                _ = retention.tick() => {
                    if let Err(err) = self.delete_terminal_dispatches().await {
                        tracing::error!(err = %err, "Error in sequence dispatch retention job");
                    }
                }
                () = stop.cancelled() => break,
            }
        }
    }
}

/// A `setInterval`: the first tick after one period.
fn interval(period_ms: u64) -> tokio::time::Interval {
    let period = Duration::from_millis(period_ms.max(1));
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer
}

/// The SQL reads of the reconcile job on the P2.4 `SequenceDispatch` table.
pub struct SqlReconcileRepo {
    pub db: Arc<Db>,
}

impl ReconcileRepo for SqlReconcileRepo {
    fn find_pending_due(
        &self,
        query: PendingDueQuery,
    ) -> BoxFuture<'_, Result<Vec<PendingDue>, JobsError>> {
        Box::pin(async move {
            let workspace = match &query.workspace_id {
                Some(ws) => Some(
                    ws.parse::<i64>()
                        .map_err(|_| JobsError::InvalidOptions(format!("invalid id {ws:?}")))?,
                ),
                None => None,
            };
            let (after_at, after_id) = match &query.after {
                Some((at, id)) => (Some(*at), id.parse::<i64>().ok()),
                None => (None, None),
            };
            Ok(self
                .db
                .read(move |conn| {
                    let mut stmt = conn.prepare_cached(
                        r#"SELECT "id", "bucket", "runAtMs" FROM "SequenceDispatch"
                           WHERE "status" = 'pending' AND "runAtMs" <= ?1
                             AND (?2 IS NULL OR "workspaceId" = ?2)
                             AND (?3 IS NULL OR ("runAtMs", "id") > (?3, ?4))
                           ORDER BY "runAtMs", "id" LIMIT ?5"#,
                    )?;
                    let rows = stmt
                        .query_map(
                            params![
                                query.run_at_ms_lte,
                                workspace,
                                after_at,
                                after_id,
                                query.limit
                            ],
                            |r| {
                                Ok(PendingDue {
                                    id: r.get::<_, i64>(0)?.to_string(),
                                    bucket: r.get(1)?,
                                    run_at_ms: r.get(2)?,
                                })
                            },
                        )?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?)
        })
    }

    fn find_pending_ids<'a>(
        &'a self,
        ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<String>, JobsError>> {
        Box::pin(async move {
            let numbers: Vec<i64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
            if numbers.is_empty() {
                return Ok(Vec::new());
            }
            let list = numbers
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            Ok(self
                .db
                .read(move |conn| {
                    let sql = format!(
                        r#"SELECT "id" FROM "SequenceDispatch" WHERE "id" IN ({list}) AND "status" = 'pending'"#
                    );
                    let rows = conn
                        .prepare(&sql)?
                        .query_map([], |r| r.get::<_, i64>(0))?
                        .map(|r| r.map(|id| id.to_string()))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?)
        })
    }
}
