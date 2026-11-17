//! The producer side (spec 2.3): the `Enqueuer` trait that replaces
//! `xQueue.add` / `addBulk`, and the error type of the crate.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::options::{AddResult, JobOptions, NewJob};
use crate::queue::QueueName;

/// A boxed `Send` future (no `futures` / `async-trait` crates).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Errors of the jobs layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobsError {
    /// `Custom Id cannot contain :` and other option checks (spec 2.3 step 1).
    InvalidOptions(String),
    /// The store (database) failed or is closed. The text is the cause.
    Store(String),
    /// The queue cannot be used now (the TypeScript "queue is unavailable").
    Unavailable(String),
    /// Shutdown began; no new work is accepted.
    Shutdown(String),
    /// A configuration value is not valid.
    Config(String),
    /// Any other error, with its message.
    Other(String),
}

impl fmt::Display for JobsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobsError::InvalidOptions(m)
            | JobsError::Store(m)
            | JobsError::Unavailable(m)
            | JobsError::Shutdown(m)
            | JobsError::Config(m)
            | JobsError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for JobsError {}

/// Adds jobs to a queue. `JobStore` is the SQLite implementation; tests use
/// a recording fake.
pub trait Enqueuer: Send + Sync {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>>;

    /// All jobs in one write transaction (BullMQ `addBulk` is one MULTI).
    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>>;

    /// `getPrioritizedCount() + getDelayedCount()` of `queue`: the jobs still to
    /// run. A producer without a store (a test fake) reports none.
    fn backlog_count(&self, _queue: QueueName) -> BoxFuture<'_, Result<u64, JobsError>> {
        Box::pin(async { Ok(0) })
    }

    /// `queue.remove(jobId)`: `Ok(false)` for an active (locked) or missing
    /// job. A producer without a store (a test fake) removes nothing.
    fn remove_job<'a>(
        &'a self,
        _queue: QueueName,
        _job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async { Ok(false) })
    }
}

impl From<chatbotx_db::DbError> for JobsError {
    fn from(error: chatbotx_db::DbError) -> JobsError {
        JobsError::Store(error.to_string())
    }
}

impl<T: Enqueuer + ?Sized> Enqueuer for std::sync::Arc<T> {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        (**self).add(queue, name, data, opts)
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        (**self).add_bulk(queue, jobs)
    }

    fn backlog_count(&self, queue: QueueName) -> BoxFuture<'_, Result<u64, JobsError>> {
        (**self).backlog_count(queue)
    }

    fn remove_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        (**self).remove_job(queue, job_id)
    }
}
