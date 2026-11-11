//! The sequence consumer (`apps/worker/src/sequence-scheduler/worker-consumer.ts`,
//! spec 3.1-3.2): the handler of the `sequenceScheduler` queue
//! (concurrency 100). Per job: blocked-workspace guard → dispatch lock
//! (30 s) → fetch the pending dispatch (missing → remove from the schedule)
//! → validate → not ready → re-schedule → lock the dispatch row → enqueue
//! the `integration` `sendSequenceFlow` job (`seq-<id>-<attempt>`).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::clock::Clock;
use crate::ctx::{BoxError, JobError};
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::Retention;
use crate::queue::QueueName;
use crate::sequence::schedule::DispatchSchedule;

/// Worker concurrency (`MAX_PROCESS`).
pub const SEQUENCE_CONSUMER_CONCURRENCY: u32 = 100;
/// Dispatch lock TTL of the consumer, in seconds.
pub const CONSUMER_LOCK_TTL_SECS: u64 = 30;
/// `SEQUENCE_SCHEDULE_PAYLOAD_TYPE` (flow-config).
pub const SEQUENCE_SCHEDULE_PAYLOAD_TYPE: &str = "sequenceSchedule";
/// The audit actor source set around the integration enqueue.
pub const EXECUTE_STEP_AUDIT_SOURCE: &str = "sequence-scheduler:executeStep";

/// The job data published by the producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceJobData {
    pub bucket: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<i64>,
    pub dispatch_id: String,
    pub workspace_id: String,
}

/// The job payload: the current shape, or the legacy wrapper
/// `{ key, value: "<JSON of SequenceJobData>" }` kept for the one-release
/// cutover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SequenceJobPayload {
    Current(SequenceJobData),
    Legacy { key: String, value: String },
}

/// A pending `SequenceDispatch` row as the consumer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRow {
    pub id: String,
    pub attempt: u32,
    pub bucket: u32,
    pub contact_id: String,
    pub contact_inbox_id: String,
    pub enrollment_id: String,
    pub sequence_id: String,
    pub step_id: String,
    pub workspace_id: String,
}

/// A sequence step as the consumer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRow {
    pub id: String,
    pub sequence_id: String,
}

/// `validateStep` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepValidation {
    pub valid: bool,
    pub reason: Option<String>,
}

/// A lock held by another owner, raised by any lock inside the handler
/// (`LockAcquisitionError`). Only an error for the dispatch's own key is
/// lock contention; any other key propagates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisitionError {
    pub key: String,
    pub message: String,
}

impl fmt::Display for LockAcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for LockAcquisitionError {}

/// The business services the handler calls (`DispatchProcessorService`,
/// `StepExecutorService`, `RetrySchedulerService`, `isBlockedWorkspace`,
/// `revertDispatchToPending`, `advanceEnrollment`).
pub trait SequenceConsumerServices: Send + Sync {
    fn is_blocked_workspace<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, BoxError>>;
    fn fetch_dispatch<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<DispatchRow>, BoxError>>;
    fn validate_dispatch<'a>(
        &'a self,
        dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>>;
    fn is_dispatch_ready(&self, dispatch: &DispatchRow, now_ms: i64) -> bool;
    fn lock_dispatch<'a>(
        &'a self,
        dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>>;
    fn fetch_step<'a>(
        &'a self,
        step_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<StepRow>, BoxError>>;
    fn validate_step<'a>(
        &'a self,
        step: &'a StepRow,
    ) -> BoxFuture<'a, Result<StepValidation, BoxError>>;
    fn mark_dispatch_canceled<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>>;
    fn revert_dispatch_to_pending<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>>;
}

/// `sequenceConnections.useExisting()` + `SchedulerClient`.
pub trait ScheduleConnector: Send + Sync {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>>;
}

/// Collaborators of the consumer.
#[derive(Clone)]
pub struct SequenceConsumerDeps {
    pub connector: Arc<dyn ScheduleConnector>,
    pub services: Arc<dyn SequenceConsumerServices>,
    /// The `integration` queue.
    pub integration: Arc<dyn Enqueuer>,
    pub clock: Arc<dyn Clock>,
    /// `isShutdownRequested()`.
    pub shutdown_requested: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// The worker options of the consumer (BullMQ `Worker` options).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerWorkerSpec {
    pub queue: QueueName,
    pub concurrency: u32,
    /// `{ count: 0 }`: terminal jobs are removed so the producer can enqueue
    /// `sequence-<id>` again for a retried dispatch.
    pub remove_on_complete: Retention,
    pub remove_on_fail: Retention,
}

/// The audit actor of the current task (`getAuditActor()`), set by the
/// consumer around the integration enqueue (`withAuditContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditActor {
    pub source: String,
    pub workspace_id: String,
}

/// The audit actor of the running task, if any.
pub fn current_audit_actor() -> Option<AuditActor> {
    todo!()
}

/// A started consumer (the BullMQ `Worker`).
pub struct SequenceConsumer {
    _private: (),
}

/// Starts the consumer: connects the schedule, then creates the worker
/// unless shutdown was requested meanwhile (then logs a warn
/// `Shutdown requested ...` and returns `Ok(None)`).
pub async fn start_sequence_consumer(
    deps: SequenceConsumerDeps,
) -> Result<Option<Arc<SequenceConsumer>>, JobsError> {
    todo!()
}

impl SequenceConsumer {
    /// The worker options it was created with.
    pub fn spec(&self) -> &ConsumerWorkerSpec {
        todo!()
    }

    /// The job processor. A malformed legacy payload is logged
    /// (`Failed to parse ...`, `err`) and discarded (`Ok`); lock contention
    /// on the dispatch's own key is logged at debug
    /// (`... another worker owns the lock`) and is `Ok`; any other error is
    /// logged (`... propagating for BullMQ retry`) and returned as
    /// `JobError::Retryable`.
    pub async fn process(&self, data: Value) -> Result<(), JobError> {
        todo!()
    }
}
