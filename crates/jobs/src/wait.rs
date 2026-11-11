//! Waiting for a job (spec 2.9): the in-process completion registry that
//! replaces `QueueEvents` + `job.waitUntilFinished`, and the helpers of
//! `packages/worker-config/src/lib/job-wait.ts`.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{broadcast, watch};

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
        match self {
            WaitError::Timeout => f.write_str("Job wait timed out before finishing"),
            WaitError::Failed(reason) => f.write_str(reason),
            WaitError::CompletionStateUnknown(cause) => write!(
                f,
                "Unable to determine whether the queued job has finished retrying: {cause}"
            ),
            WaitError::Shutdown => f.write_str("Job wait ended by shutdown"),
        }
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
    events: broadcast::Sender<JobEvent>,
    closed: watch::Sender<bool>,
}

/// Capacity of the event channel (spec 2.9). A lagged receiver re-reads
/// the job row when it waits through a store.
const EVENT_CAPACITY: usize = 1024;

static GLOBAL: Mutex<Option<Arc<CompletionRegistry>>> = Mutex::new(None);

impl CompletionRegistry {
    /// A new, separate registry (tests).
    pub fn new() -> Arc<CompletionRegistry> {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (closed, _) = watch::channel(false);
        Arc::new(CompletionRegistry { events, closed })
    }

    /// The shared registry of this process (`getQueueEvents`): the same
    /// `Arc` until [`CompletionRegistry::close_global`].
    pub fn global() -> Arc<CompletionRegistry> {
        let mut slot = GLOBAL.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slot.get_or_insert_with(CompletionRegistry::new))
    }

    /// `closeAllQueueEvents`: closes the shared registry (pending waits end
    /// with `WaitError::Shutdown`); the next `global()` is a fresh one.
    pub async fn close_global() {
        let open = GLOBAL.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(registry) = open {
            registry.close();
        }
    }

    /// Ends every pending wait with `WaitError::Shutdown`.
    pub fn close(&self) {
        self.closed.send_replace(true);
    }

    /// Whether [`CompletionRegistry::close`] was called.
    pub fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// Publishes one event (called by the store after commit).
    pub fn publish(&self, event: JobEvent) {
        // No receiver = nobody waits; not an error.
        let _ = self.events.send(event);
    }

    /// A receiver of every event published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<JobEvent> {
        self.events.subscribe()
    }

    /// Waits on `rx` (subscribed before the caller read any state) for the
    /// first finish event of `(queue, job_id)`. `on_lag` is asked for the
    /// current outcome when events were dropped (`None` = keep waiting).
    pub(crate) async fn wait_on<'a, F>(
        &self,
        mut rx: broadcast::Receiver<JobEvent>,
        queue: QueueName,
        job_id: &str,
        timeout_ms: u64,
        on_lag: F,
    ) -> Result<Value, WaitError>
    where
        F: Fn() -> BoxFuture<'a, Option<Result<Value, WaitError>>>,
    {
        let mut closed = self.closed.subscribe();
        if *closed.borrow_and_update() {
            return Err(WaitError::Shutdown);
        }
        let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => return Err(WaitError::Timeout),
                changed = closed.changed() => {
                    if changed.is_err() || *closed.borrow() {
                        return Err(WaitError::Shutdown);
                    }
                }
                event = rx.recv() => match event {
                    Ok(event) => {
                        if let Some(outcome) = finish_outcome(&event, queue, job_id) {
                            return outcome;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(outcome) = on_lag().await {
                            return outcome;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(WaitError::Shutdown),
                },
            }
        }
    }
}

/// The wait outcome an event gives for `(queue, job_id)`, if it is a finish
/// event of that job. Every failed attempt rejects (BullMQ `failed`).
pub(crate) fn finish_outcome(
    event: &JobEvent,
    queue: QueueName,
    job_id: &str,
) -> Option<Result<Value, WaitError>> {
    match event {
        JobEvent::Completed {
            queue: q,
            job_id: id,
            return_value,
        } if *q == queue && id == job_id => Some(Ok(return_value.clone())),
        JobEvent::Failed {
            queue: q,
            job_id: id,
            reason,
            ..
        } if *q == queue && id == job_id => Some(Err(WaitError::Failed(reason.clone()))),
        _ => None,
    }
}

impl JobWaiter for CompletionRegistry {
    fn wait_until_finished<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>> {
        // Subscribe before the future is first polled: an event published
        // right after this call is not lost.
        let rx = self.subscribe();
        Box::pin(async move {
            self.wait_on(rx, queue, job_id, timeout_ms, || Box::pin(async { None }))
                .await
        })
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
    let Some(queue) = job.queue else {
        return Err(WaitError::Failed(NOT_WAITABLE.to_owned()));
    };
    let deadline = clock
        .now_ms()
        .saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX));
    let mut last_error: Option<WaitError> = None;
    loop {
        let remaining = deadline.saturating_sub(clock.now_ms());
        if remaining <= 0 {
            return Err(last_error.unwrap_or(WaitError::Timeout));
        }
        let remaining = u64::try_from(remaining).unwrap_or(0);
        let error = match waiter
            .wait_until_finished(queue, &job.job_id, remaining)
            .await
        {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if job.job_id.is_empty() {
            return Err(error);
        }
        let latest = match inspector.get_job(queue, &job.job_id).await {
            Ok(Some(latest)) => latest,
            Ok(None) => {
                return Err(WaitError::CompletionStateUnknown(format!(
                    "Unable to load queued job {}",
                    job.job_id
                )));
            }
            Err(lookup) => return Err(WaitError::CompletionStateUnknown(lookup.to_string())),
        };
        let attempts = latest.opts.attempts.or(job.attempts).unwrap_or(1);
        if latest.attempts_made >= attempts {
            return Err(error);
        }
        last_error = Some(error);
    }
}

/// The error of a wait on a target without a queue.
const NOT_WAITABLE: &str = "Queue did not return a waitable job";

/// `INTEGRATION_JOB_WAIT_TIMEOUT_MS`.
pub const INTEGRATION_JOB_WAIT_TIMEOUT_MS: u64 = 10_000;

/// `waitForJobResult` (strict): the result, or the wait error unchanged.
pub async fn wait_for_job_result(
    waiter: &dyn JobWaiter,
    job: &WaitTarget,
    timeout_ms: u64,
) -> Result<Value, WaitError> {
    let Some(queue) = job.queue else {
        return Err(WaitError::Failed(NOT_WAITABLE.to_owned()));
    };
    waiter
        .wait_until_finished(queue, &job.job_id, timeout_ms)
        .await
}

/// `waitForIntegrationJobCompletion` (best effort, default 10 000 ms):
/// never fails; a failed wait is logged.
pub async fn wait_for_integration_job_completion(
    waiter: &dyn JobWaiter,
    job: &WaitTarget,
    timeout_ms: Option<u64>,
) {
    let queue = job.queue.unwrap_or(QueueName::Integration);
    let timeout = timeout_ms.unwrap_or(INTEGRATION_JOB_WAIT_TIMEOUT_MS);
    if let Err(err) = waiter
        .wait_until_finished(queue, &job.job_id, timeout)
        .await
    {
        // Best-effort ordering only: never returned.
        tracing::debug!(err = %err, job_id = %job.job_id, "Integration job wait ended without a result");
    }
}
