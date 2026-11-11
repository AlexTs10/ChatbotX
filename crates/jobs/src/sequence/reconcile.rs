//! `ReconcileJob` (`apps/worker/src/sequence-scheduler/worker.ts`, spec 3.1):
//! re-adds pending dispatches due within 24 h to the schedule (every hour),
//! removes schedule members without a pending row (every 6 h), and deletes
//! old terminal dispatches in batches (every 24 h).

use std::sync::Arc;

use chatbotx_db::Db;

use crate::clock::Clock;
use crate::enqueue::{BoxFuture, JobsError};
use crate::sequence::schedule::DispatchSchedule;

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
    _private: (),
}

impl ReconcileJob {
    pub fn new(options: ReconcileOptions, deps: ReconcileDeps) -> ReconcileJob {
        todo!()
    }

    /// Re-adds every pending dispatch due within 24 h to its bucket's
    /// schedule set, in batches of 1000, then pauses 1000 ms.
    pub async fn reconcile(&self) -> Result<(), JobsError> {
        todo!()
    }

    /// Removes members of both sets (each assigned bucket) that have no
    /// pending row (`remove_from_all`).
    pub async fn cleanup_orphans(&self) -> Result<(), JobsError> {
        todo!()
    }

    /// `DELETE FROM "SequenceDispatch" WHERE "status" IN (terminal) AND older
    /// than the TTL ... LIMIT batch`, repeated until a batch is short.
    pub async fn delete_terminal_dispatches(&self) -> Result<RetentionOutcome, JobsError> {
        todo!()
    }
}
