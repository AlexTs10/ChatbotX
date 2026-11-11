//! Worker runtime (spec 2.4-2.10, 2.13; port of
//! `apps/worker/src/lib/create-queue-worker.ts`): `WorkerSpec` (the merged
//! worker options), `Runtime::spawn_worker` (one dispatcher per queue) and
//! the worker registry of the `standalone` / `core` processes.

use std::sync::Arc;

use serde_json::Value;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::ctx::{BoxError, JobCtx, JobError};
use crate::enqueue::{BoxFuture, JobsError};
use crate::options::Retention;
use crate::queue::QueueName;
use crate::shutdown::Shutdown;
use crate::store::JobStore;

/// A job handler (the BullMQ processor).
pub type Handler = Arc<dyn Fn(JobCtx) -> BoxFuture<'static, Result<Value, JobError>> + Send + Sync>;

/// One failed attempt, as the `failed` listener sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedJob {
    pub queue: QueueName,
    pub job_id: String,
    pub reason: String,
    /// No retry follows.
    pub final_attempt: bool,
}

/// `onFailed(job, err)`: called after the failure commits, never awaited by
/// the dispatcher; an error is logged ("Failed-job handler threw").
pub type FailedHook =
    Arc<dyn Fn(FailedJob) -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// `limiter: { max, duration }` (fixed window, spec 2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limiter {
    pub max: u32,
    pub duration_ms: u64,
}

/// The worker options of one queue: `defaultWorkerOptions` merged with the
/// caller's options (caller wins).
#[derive(Clone)]
pub struct WorkerSpec {
    pub queue: QueueName,
    pub concurrency: u32,
    pub limiter: Option<Limiter>,
    /// `lockDuration`.
    pub lease_ms: u64,
    pub stalled_interval_ms: u64,
    pub max_stalled: u32,
    pub remove_on_complete: Retention,
    pub remove_on_fail: Retention,
    /// `None` = the default failed logger ("Job has failed").
    pub on_failed: Option<FailedHook>,
}

impl WorkerSpec {
    /// The defaults (spec 1.1): concurrency 5, `KeepCount(1000)` /
    /// `KeepCount(5000)`, lease 30 000 ms, stalled interval 30 000 ms,
    /// max stalled 1, no limiter, default failed logger.
    pub fn new(queue: QueueName) -> WorkerSpec {
        todo!()
    }

    pub fn concurrency(self, concurrency: u32) -> WorkerSpec {
        todo!()
    }

    pub fn limiter(self, limiter: Limiter) -> WorkerSpec {
        todo!()
    }

    pub fn lease_ms(self, lease_ms: u64) -> WorkerSpec {
        todo!()
    }

    /// Replaces the default failed logger.
    pub fn on_failed(self, hook: FailedHook) -> WorkerSpec {
        todo!()
    }

    /// The listeners a spawned worker attaches (`"error"`, `"failed"`).
    pub fn listeners(&self) -> Vec<&'static str> {
        todo!()
    }
}

/// A running worker of one queue.
pub struct WorkerHandle {
    _private: (),
}

impl WorkerHandle {
    pub fn queue(&self) -> QueueName {
        todo!()
    }

    pub fn spec(&self) -> &WorkerSpec {
        todo!()
    }

    /// `worker.close()`: stop claiming, wait for running jobs.
    pub fn close(&self) -> BoxFuture<'static, ()> {
        todo!()
    }

    /// Test hook: how many times the worker was closed.
    pub fn close_count(&self) -> u32 {
        todo!()
    }

    /// The `failed` listener: the custom `on_failed` hook, else the error
    /// log "Job has failed" (fields `queue`, `job_id`, `err`). A hook error
    /// is logged as "Failed-job handler threw". Runs detached; the handle
    /// lets a test wait for it.
    pub fn report_failed(&self, job: FailedJob) -> JoinHandle<()> {
        todo!()
    }

    /// The `error` listener: logs "Queue worker error" (fields `queue`,
    /// `err`); never requests shutdown.
    pub fn report_error(&self, err: &JobsError) {
        todo!()
    }
}

/// Owns the dispatchers of one process.
pub struct Runtime {
    _private: (),
}

impl Runtime {
    pub fn new(store: Arc<JobStore>, shutdown: Arc<Shutdown>, clock: Arc<dyn Clock>) -> Runtime {
        todo!()
    }

    /// `createQueueWorker(queue, processor, options)`: starts the dispatcher
    /// and registers a closer named after the queue. After shutdown began:
    /// `Err(JobsError::Shutdown("Cannot start <queue> worker: shutdown
    /// already requested"))`, no worker, no closer.
    pub fn spawn_worker(
        &self,
        spec: WorkerSpec,
        handler: Handler,
    ) -> Result<WorkerHandle, JobsError> {
        todo!()
    }
}

/// Which process layout (spec 1.1): `worker standalone` (all consumers +
/// schedule + sequence) or `worker core` (no schedule, no sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerMode {
    Standalone,
    Core,
}

/// The worker registry: one spec per queue that has a consumer in this repo
/// (every queue except `broadcast` and `quota`, spec 1.2).
pub fn worker_specs() -> Vec<WorkerSpec> {
    todo!()
}

/// The queues whose workers `mode` starts (dev runs the same binary).
pub fn started_queues(mode: WorkerMode) -> Vec<QueueName> {
    todo!()
}
