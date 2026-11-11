//! Waiting for a job (spec 2.9): the in-process completion registry that
//! replaces `QueueEvents` + `job.waitUntilFinished`, and the helpers of
//! `packages/worker-config/src/lib/job-wait.ts`.

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::clock::Clock;
use crate::enqueue::BoxFuture;
use crate::queue::QueueName;
use crate::store::JobInspector;

/// An event sent after the commit of a job state change.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Completed {
        queue: QueueName,
        job_id: String,
        return_value: Value,
    },
    /// One failed attempt; `final_attempt` = no retry follows.
    Failed {
        queue: QueueName,
        job_id: String,
        reason: String,
        final_attempt: bool,
    },
    Stalled {
        queue: QueueName,
        job_id: String,
    },
    Delayed {
        queue: QueueName,
        job_id: String,
        run_at_ms: i64,
    },
}

/// Why a wait ended without a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitError {
    /// No finish event before the timeout.
    Timeout,
    /// The job (one attempt of it) failed; the text is the failure reason
    /// (`Error.message`).
    Failed(String),
    /// `JobCompletionStateUnknownError`: the job state could not be read.
    CompletionStateUnknown(String),
    /// The registry was closed (shutdown) while waiting.
    Shutdown,
}

impl fmt::Display for WaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for WaitError {}

/// `job.waitUntilFinished(queueEvents, ttl)` as a trait (fakes in tests).
pub trait JobWaiter: Send + Sync {
    fn wait_until_finished<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>>;
}

/// The job a caller waits on: id plus the attempt fields of the `Job`
/// object it got from `add`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitTarget {
    pub queue: Option<QueueName>,
    pub job_id: String,
    pub attempts_made: u32,
    /// `job.opts.attempts`.
    pub attempts: Option<u32>,
}

/// The process-wide completion registry (one `broadcast` channel of
/// [`JobEvent`]s; spec 2.9).
pub struct CompletionRegistry {
    _private: (),
}

impl CompletionRegistry {
    /// A new, separate registry (tests).
    pub fn new() -> Arc<CompletionRegistry> {
        todo!()
    }

    /// The shared registry of this process (`getQueueEvents`): the same
    /// `Arc` until [`CompletionRegistry::close_global`].
    pub fn global() -> Arc<CompletionRegistry> {
        todo!()
    }

    /// `closeAllQueueEvents`: closes the shared registry (pending waits end
    /// with `WaitError::Shutdown`); the next `global()` is a fresh one.
    pub async fn close_global() {
        todo!()
    }

    /// Ends every pending wait with `WaitError::Shutdown`.
    pub fn close(&self) {
        todo!()
    }

    /// Whether [`CompletionRegistry::close`] was called.
    pub fn is_closed(&self) -> bool {
        todo!()
    }

    /// Publishes one event (called by the store after commit).
    pub fn publish(&self, event: JobEvent) {
        todo!()
    }
}

impl JobWaiter for CompletionRegistry {
    fn wait_until_finished<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>> {
        todo!()
    }
}

/// `waitForJobCompletionWithRetries(job, queue, queueEvents, timeoutMs)`:
/// waits again after a failed attempt while attempts remain (read with
/// `inspector.get_job`), inside one overall deadline from `clock`.
/// A failed `get_job` is `WaitError::CompletionStateUnknown`.
pub async fn wait_for_completion_with_retries(
    waiter: &dyn JobWaiter,
    inspector: &dyn JobInspector,
    clock: &dyn Clock,
    job: &WaitTarget,
    timeout_ms: u64,
) -> Result<Value, WaitError> {
    todo!()
}

/// `waitForJobResult` (strict): the result, or the wait error unchanged.
pub async fn wait_for_job_result(
    waiter: &dyn JobWaiter,
    job: &WaitTarget,
    timeout_ms: u64,
) -> Result<Value, WaitError> {
    todo!()
}

/// `waitForIntegrationJobCompletion` (best effort, default 10 000 ms):
/// never fails; a failed wait is logged.
pub async fn wait_for_integration_job_completion(
    waiter: &dyn JobWaiter,
    job: &WaitTarget,
    timeout_ms: Option<u64>,
) {
    todo!()
}
