//! The SQLite job store (spec 2.2-2.11): the `jobs`, `job_dedup`,
//! `job_dedup_next`, `job_schedulers` and `job_meta` tables.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_db::{Db, WriteTx};
use serde_json::Value;

use crate::clock::Clock;
use crate::ctx::JobCtx;
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::{AddResult, JobOptions, NewJob};
use crate::queue::QueueName;

/// BullMQ `job.getState()` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JobState {
    Waiting,
    /// A waiting job with `priority > 0`.
    Prioritized,
    Delayed,
    Active,
    Completed,
    Failed,
    /// No such job.
    Unknown,
}

impl JobState {
    /// `"waiting"`, `"prioritized"`, `"delayed"`, `"active"`, `"completed"`, `"failed"`, `"unknown"`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// One job row as `getJob` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRecord {
    pub queue: QueueName,
    pub job_id: String,
    pub name: String,
    pub data: Value,
    pub state: JobState,
    /// Stored priority (`job.priority`; changed by `change_priority`).
    pub priority: u32,
    pub attempts_made: u32,
    pub attempts_started: u32,
    /// The options given at enqueue (`job.opts`, never changed later).
    pub opts: JobOptions,
    pub return_value: Option<Value>,
    pub failed_reason: Option<String>,
}

/// The inspection API (spec 2.11, S14) as a trait, so callers can be tested
/// with a fake that fails.
pub trait JobInspector: Send + Sync {
    /// `queue.getJob(id)`.
    fn get_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>>;

    /// `job.getState()`; a missing job is `JobState::Unknown`.
    fn get_state<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>>;

    /// `job.changePriority({ priority })`: only a waiting/prioritized job;
    /// `Ok(false)` when the job is in another state (not an error).
    fn change_priority<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>>;

    /// `queue.remove(id)`: `Ok(false)` for an active (locked) or missing job.
    fn remove<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>>;
}

/// The SQLite job store on a `chatbotx-db` handle.
pub struct JobStore {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
}

impl JobStore {
    /// Opens the store on `db` and creates the jobs tables if they are
    /// missing (the `jobs` migration).
    pub async fn open(db: Arc<Db>, clock: Arc<dyn Clock>) -> Result<JobStore, JobsError> {
        todo!()
    }

    /// The database handle.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// `add` inside a caller-owned write transaction (transactional outbox,
    /// spec 2.3). Signals are sent by the caller after commit.
    pub fn add_in(
        &self,
        tx: &mut WriteTx<'_>,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> Result<AddResult, JobsError> {
        todo!()
    }

    /// `getJobCounts(...states)`.
    pub async fn get_job_counts(
        &self,
        queue: QueueName,
        states: &[JobState],
    ) -> Result<BTreeMap<JobState, u64>, JobsError> {
        todo!()
    }

    /// Promotes due delayed jobs (`run_at_ms <= now`), returns how many.
    pub async fn promote_due(&self) -> Result<u64, JobsError> {
        todo!()
    }

    /// Claims up to `limit` waiting jobs of `queue` in BullMQ order
    /// (`ORDER BY priority, ready_seq`) for `lease_owner` (spec 2.4).
    pub async fn claim(
        &self,
        queue: QueueName,
        lease_owner: &str,
        limit: u32,
    ) -> Result<Vec<JobCtx>, JobsError> {
        todo!()
    }

    /// Marks a claimed job completed with its return value (spec 2.6).
    pub async fn complete(
        &self,
        queue: QueueName,
        job_id: &str,
        return_value: Value,
    ) -> Result<(), JobsError> {
        todo!()
    }

    /// Records one failed attempt (retry or final failure, spec 2.6).
    pub async fn fail(
        &self,
        queue: QueueName,
        job_id: &str,
        reason: &str,
        unrecoverable: bool,
    ) -> Result<(), JobsError> {
        todo!()
    }
}

impl Enqueuer for JobStore {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        todo!()
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        todo!()
    }
}

impl JobInspector for JobStore {
    fn get_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>> {
        todo!()
    }

    fn get_state<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>> {
        todo!()
    }

    fn change_priority<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        todo!()
    }

    fn remove<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        todo!()
    }
}
