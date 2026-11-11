//! The consumer side of one job: what a handler sees (`JobCtx`) and how it
//! ends (`JobError`), spec 2.6 and `apps/worker/src/lib/job-attempts.ts`.

use serde_json::Value;

/// A boxed error with its message (`Error.message`) as `Display`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// One claimed job, as a handler sees it (BullMQ `Job`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobCtx {
    /// Row id (snowflake).
    pub id: i64,
    /// Custom job id or the row id as text (`job.id`).
    pub job_id: String,
    /// Job name (the action).
    pub name: String,
    pub data: Value,
    /// `job.opts.attempts`; `None` = not configured (counts as 1).
    pub attempts: Option<u32>,
    /// `job.attemptsMade`: failed attempts so far.
    pub attempts_made: u32,
    /// `job.attemptsStarted`: activations so far, this one included.
    pub attempts_started: u32,
    pub priority: u32,
    /// The lease token of the worker that holds the job (BullMQ `token`).
    /// `None` = no lease: the job cannot be parked with `JobError::Delayed`.
    pub lease_token: Option<String>,
}

impl JobCtx {
    /// [`is_final_attempt`] for this job (call from inside a handler).
    pub fn is_final_attempt(&self) -> bool {
        todo!()
    }

    /// [`has_exhausted_attempts`] for this job (call from a `failed` hook).
    pub fn has_exhausted_attempts(&self) -> bool {
        todo!()
    }
}

/// `isFinalAttempt`: `attempts_made + 1 >= attempts.unwrap_or(1)`.
pub fn is_final_attempt(attempts_made: u32, attempts: Option<u32>) -> bool {
    todo!()
}

/// `hasExhaustedAttempts`: `attempts_made >= attempts.unwrap_or(1)`.
pub fn has_exhausted_attempts(attempts_made: u32, attempts: Option<u32>) -> bool {
    todo!()
}

/// How a handler ends with an error (spec 2.6).
#[derive(Debug)]
pub enum JobError {
    /// Normal failure: retried while attempts remain.
    Retryable(BoxError),
    /// `UnrecoverableError`: final failure at once.
    Unrecoverable(BoxError),
    /// `moveToDelayed(run_at_ms)` + `DelayedError`: parked, not a failure.
    Delayed { run_at_ms: i64 },
}
