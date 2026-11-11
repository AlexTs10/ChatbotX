//! The sequence producer (`apps/worker/src/sequence-scheduler/worker-producer.ts`,
//! spec 3.1-3.2): claims due dispatches from the schedule and retry sets
//! of each assigned bucket and publishes them to the `sequenceScheduler`
//! queue with job id `sequence-<dispatchId>`.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::clock::Clock;
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::sequence::schedule::{DispatchSchedule, ScheduleKind};

/// Default claim limit per set and tick (`getDue(..., 100)`).
pub const DEFAULT_CLAIM_LIMIT: u32 = 100;
/// Default dispatch lock TTL (30 s).
pub const DEFAULT_LOCK_TTL_MS: u64 = 30_000;

/// Constructor options (`Partial<SchedulerWorkerConfig>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerWorkerOptions {
    /// `None` = `assigned_buckets(SCHEDULER_BUCKET_RANGE)`.
    pub buckets: Option<Vec<u32>>,
    pub claim_limit: Option<u32>,
    pub lock_ttl_ms: Option<u64>,
    /// `None` or `Some(0)` = resolve from env (`resolve_tick_interval_ms`).
    pub tick_interval_ms: Option<u64>,
}

/// The resolved config (`SchedulerWorker.config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerWorkerConfig {
    pub buckets: Vec<u32>,
    pub claim_limit: u32,
    pub lock_ttl_ms: u64,
    pub tick_interval_ms: u64,
}

/// `{ id, workspaceId }` of a dispatch that is still pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDispatchRef {
    pub id: String,
    pub workspace_id: String,
}

/// `sequenceDispatchRepository.listPendingWorkspaceIds(ids)`.
pub trait PendingDispatchLookup: Send + Sync {
    fn list_pending_workspace_ids<'a>(
        &'a self,
        dispatch_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>>;
}

/// What `start` acquires once: the schedule handle
/// (`sequenceConnections.useExisting` + `SchedulerClient`) and the typed
/// `sequenceScheduler` queue (`getSequenceSchedulerQueue`, `None` =
/// unavailable).
pub trait ProducerBackend: Send + Sync {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>>;
    fn sequence_queue(&self) -> BoxFuture<'_, Result<Option<Arc<dyn Enqueuer>>, JobsError>>;
}

/// Collaborators of the producer.
#[derive(Clone)]
pub struct SchedulerWorkerDeps {
    pub backend: Arc<dyn ProducerBackend>,
    pub pending: Arc<dyn PendingDispatchLookup>,
    pub clock: Arc<dyn Clock>,
    /// `isShutdownRequested()`.
    pub shutdown_requested: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// One claimed dispatch and the set it was claimed from (for re-insert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedDispatch {
    pub dispatch_id: String,
    pub source: ScheduleKind,
}

/// Per-bucket set sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BucketStats {
    pub schedule: u64,
    pub retry: u64,
}

/// `getHealth()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerHealth {
    pub running: bool,
    pub buckets: Vec<u32>,
    pub stats: BTreeMap<u32, BucketStats>,
}

/// `SchedulerWorker`.
pub struct SchedulerWorker {
    _private: (),
}

impl SchedulerWorker {
    /// `new SchedulerWorker(options)`; `env` is the process env map (read
    /// for `SCHEDULER_TICK_INTERVAL_MS` and `SCHEDULER_BUCKET_RANGE`).
    pub fn new(
        options: SchedulerWorkerOptions,
        env: &BTreeMap<String, String>,
        deps: SchedulerWorkerDeps,
    ) -> Arc<SchedulerWorker> {
        todo!()
    }

    /// Test hook (the TypeScript `makeReadyWorker`): a worker marked running
    /// with `schedule` and `queue` already attached and no bucket task.
    pub fn ready(
        options: SchedulerWorkerOptions,
        env: &BTreeMap<String, String>,
        deps: SchedulerWorkerDeps,
        schedule: Arc<dyn DispatchSchedule>,
        queue: Arc<dyn Enqueuer>,
    ) -> Arc<SchedulerWorker> {
        todo!()
    }

    pub fn config(&self) -> &SchedulerWorkerConfig {
        todo!()
    }

    pub fn is_running(&self) -> bool {
        todo!()
    }

    /// Buckets that have a tick task (the `timers` map keys), sorted.
    pub fn timer_buckets(&self) -> Vec<u32> {
        todo!()
    }

    /// Acquires the schedule and the queue once (a second `start` does
    /// nothing), then starts one tick task per bucket, unless shutdown was
    /// requested meanwhile. A missing queue is
    /// `JobsError::Unavailable("Sequence scheduler queue is unavailable")`.
    pub async fn start(&self) -> Result<(), JobsError> {
        todo!()
    }

    /// Stops the tick tasks (idempotent) and waits for a running tick to
    /// finish publishing what it claimed.
    pub async fn stop(&self) {
        todo!()
    }

    /// One tick of one bucket: `get_due` on both sets; nothing due → return
    /// before locking; else claim each id under its dispatch lock
    /// (`lock_ttl_ms / 1000` s, remove from its set), then publish.
    pub async fn process_bucket(&self, bucket: u32) {
        todo!()
    }

    /// Publishes the claimed dispatches that still have a pending row in one
    /// `add_bulk`; on failure logs `Failed to publish ...` `{bucket, count}`
    /// and re-inserts them (schedule entries with `batch_add_to_schedule`,
    /// retry entries with `add_to_retry(.., now)`); a failed re-insert logs
    /// `Failed to re-insert ...` `{err, bucket, dispatch_ids}`.
    pub async fn publish_dispatches(&self, bucket: u32, claimed: &[ClaimedDispatch]) {
        todo!()
    }

    pub async fn health(&self) -> Result<SchedulerHealth, JobsError> {
        todo!()
    }
}
