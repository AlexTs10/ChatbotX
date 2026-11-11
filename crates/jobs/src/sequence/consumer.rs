//! The sequence consumer (`apps/worker/src/sequence-scheduler/worker-consumer.ts`,
//! spec 3.1-3.2): the handler of the `sequenceScheduler` queue
//! (concurrency 100). Per job: blocked-workspace guard → dispatch lock
//! (30 s) → fetch the pending dispatch (missing → remove from the schedule)
//! → validate → not ready → re-schedule → lock the dispatch row → enqueue
//! the `integration` `sendSequenceFlow` job (`seq-<id>-<attempt>`).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clock::Clock;
use crate::ctx::{BoxError, JobError};

use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::{Backoff, JobOptions, Retention};
use crate::queue::QueueName;
use crate::queues::integration::flow_continuation_job_options;
use crate::sequence::schedule::{DispatchSchedule, ScheduleError, dispatch_lock_key};

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
        f.write_str(&self.message)
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
    /// `Number(dispatch.runAtMs)` for the re-schedule of a dispatch that is
    /// not due yet (`None` = re-schedule at now).
    fn fetch_dispatch_run_at(&self, dispatch: &DispatchRow) -> Option<i64> {
        let _ = dispatch;
        None
    }
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

tokio::task_local! {
    static AUDIT_ACTOR: AuditActor;
}

/// The audit actor of the running task, if any.
pub fn current_audit_actor() -> Option<AuditActor> {
    AUDIT_ACTOR.try_with(Clone::clone).ok()
}

/// Runs `future` with `actor` as the task audit actor (`withAuditContext`).
pub async fn with_audit_actor<F: std::future::Future>(actor: AuditActor, future: F) -> F::Output {
    AUDIT_ACTOR.scope(actor, future).await
}

/// A started consumer (the BullMQ `Worker`).
pub struct SequenceConsumer {
    deps: SequenceConsumerDeps,
    schedule: Arc<dyn DispatchSchedule>,
    spec: ConsumerWorkerSpec,
}

/// Starts the consumer: connects the schedule, then creates the worker
/// unless shutdown was requested meanwhile (then logs a warn
/// `Shutdown requested ...` and returns `Ok(None)`).
pub async fn start_sequence_consumer(
    deps: SequenceConsumerDeps,
) -> Result<Option<Arc<SequenceConsumer>>, JobsError> {
    let schedule = deps.connector.connect_schedule().await?;
    // Shutdown may have drained the closers while the schedule connected.
    if (deps.shutdown_requested)() {
        tracing::warn!("Shutdown requested during startup; not starting dispatch consumer");
        return Ok(None);
    }
    Ok(Some(Arc::new(SequenceConsumer {
        deps,
        schedule,
        spec: ConsumerWorkerSpec {
            queue: QueueName::SequenceScheduler,
            concurrency: SEQUENCE_CONSUMER_CONCURRENCY,
            remove_on_complete: Retention::KeepCount(0),
            remove_on_fail: Retention::KeepCount(0),
        },
    })))
}

/// The lock-contention check of `isLockAcquisitionError(error, key)`: a
/// lock error for exactly `key` (the dispatch's own lock).
fn is_own_lock_error(err: &(dyn std::error::Error + 'static), key: &str) -> bool {
    if let Some(ScheduleError::LockAcquisition { key: k }) = err.downcast_ref::<ScheduleError>() {
        return k == key;
    }
    if let Some(e) = err.downcast_ref::<LockAcquisitionError>() {
        return e.key == key;
    }
    false
}

impl SequenceConsumer {
    /// The worker options it was created with.
    pub fn spec(&self) -> &ConsumerWorkerSpec {
        &self.spec
    }

    /// The job processor. A malformed legacy payload is logged
    /// (`Failed to parse ...`, `err`) and discarded (`Ok`); lock contention
    /// on the dispatch's own key is logged at debug
    /// (`... another worker owns the lock`) and is `Ok`; any other error is
    /// logged (`... propagating for BullMQ retry`) and returned as
    /// `JobError::Retryable`.
    pub async fn process(&self, data: Value) -> Result<(), JobError> {
        let payload = match serde_json::from_value::<SequenceJobPayload>(data.clone()) {
            Ok(SequenceJobPayload::Current(payload)) => payload,
            Ok(SequenceJobPayload::Legacy { value, .. }) => {
                // TODO(remove after the next release): legacy wrapper jobs.
                match serde_json::from_str::<SequenceJobData>(&value) {
                    Ok(payload) => payload,
                    Err(err) => {
                        tracing::error!(
                            err = %err,
                            value = %value,
                            "Failed to parse sequence dispatch message; discarding"
                        );
                        return Ok(());
                    }
                }
            }
            Err(err) => {
                tracing::warn!(err = %err, payload = %data, "Skipping sequence dispatch message without workspaceId");
                return Ok(());
            }
        };
        if payload.workspace_id.is_empty() {
            tracing::warn!(payload = ?payload, "Skipping sequence dispatch message without workspaceId");
            return Ok(());
        }
        let key = dispatch_lock_key(payload.bucket, &payload.dispatch_id);
        match self.process_dispatch(&payload).await {
            Ok(()) => Ok(()),
            Err(err) if is_own_lock_error(err.as_ref(), &key) => {
                tracing::debug!(
                    err = %err,
                    payload = ?payload,
                    "Dispatch processing skipped because another worker owns the lock"
                );
                Ok(())
            }
            Err(err) => {
                tracing::error!(
                    err = %err,
                    payload = ?payload,
                    "Error processing dispatch; propagating for BullMQ retry"
                );
                Err(JobError::Retryable(err))
            }
        }
    }

    async fn process_dispatch(&self, payload: &SequenceJobData) -> Result<(), BoxError> {
        let services = &self.deps.services;
        if services.is_blocked_workspace(&payload.workspace_id).await? {
            return Ok(());
        }
        let _lock = self
            .schedule
            .try_lock(payload.bucket, &payload.dispatch_id, CONSUMER_LOCK_TTL_SECS)
            .await?;
        let Some(dispatch) = services
            .fetch_dispatch(&payload.dispatch_id, &payload.workspace_id)
            .await?
        else {
            self.schedule
                .remove_from_schedule(payload.bucket, &payload.dispatch_id)
                .await?;
            return Ok(());
        };
        if !services.validate_dispatch(&dispatch).await? {
            return Ok(());
        }
        if !services.is_dispatch_ready(&dispatch, self.deps.clock.now_ms()) {
            // Not due yet: back to the schedule at its run time.
            let run_at = services
                .fetch_dispatch_run_at(&dispatch)
                .unwrap_or_else(|| self.deps.clock.now_ms());
            self.schedule
                .add_to_schedule(dispatch.bucket, &payload.dispatch_id, run_at)
                .await?;
            return Ok(());
        }
        if !services.lock_dispatch(&dispatch).await? {
            return Ok(());
        }
        let actor = AuditActor {
            source: EXECUTE_STEP_AUDIT_SOURCE.to_owned(),
            workspace_id: dispatch.workspace_id.clone(),
        };
        with_audit_actor(actor, self.execute_step(&dispatch)).await;
        Ok(())
    }

    /// `executeStep`: validates the step, then enqueues `sendSequenceFlow`
    /// on `integration`. Any failure reverts the dispatch to pending.
    async fn execute_step(&self, dispatch: &DispatchRow) {
        if let Err(err) = self.try_execute_step(dispatch).await {
            tracing::error!(
                err = %err,
                dispatch_id = %dispatch.id,
                "Failed to enqueue sendSequenceFlow; reverting dispatch"
            );
            if let Err(err) = self
                .deps
                .services
                .revert_dispatch_to_pending(&dispatch.id, &dispatch.workspace_id)
                .await
            {
                tracing::error!(err = %err, dispatch_id = %dispatch.id, "Failed to revert dispatch to pending");
            }
        }
    }

    async fn try_execute_step(&self, dispatch: &DispatchRow) -> Result<(), BoxError> {
        let services = &self.deps.services;
        let step = services.fetch_step(&dispatch.step_id).await?;
        let validation = match &step {
            Some(step) => services.validate_step(step).await?,
            None => StepValidation {
                valid: false,
                reason: Some("Step not found".to_owned()),
            },
        };
        if !validation.valid {
            let reason = validation.reason.unwrap_or_default();
            services
                .mark_dispatch_canceled(&dispatch.id, &dispatch.workspace_id, &reason)
                .await?;
            // The enrollment advance past an invalid step belongs to the
            // business services (Phase 3 `advanceEnrollment` caller).
            self.schedule
                .remove_from_schedule(dispatch.bucket, &dispatch.id)
                .await?;
            return Ok(());
        }
        let metadata = json!({
            "type": SEQUENCE_SCHEDULE_PAYLOAD_TYPE,
            "sequenceStepId": step.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
            "sequenceId": step.as_ref().map(|s| s.sequence_id.clone()).unwrap_or_default(),
            "dispatchId": dispatch.id,
            "contactInboxId": dispatch.contact_inbox_id,
        });
        let opts = JobOptions {
            job_id: Some(format!("seq-{}-{}", dispatch.id, dispatch.attempt)),
            attempts: Some(3),
            backoff: Some(Backoff::Exponential(5000)),
            remove_on_complete: Some(Retention::Remove),
            ..flow_continuation_job_options(Some(&metadata))
        };
        let data = json!({
            "type": SEND_SEQUENCE_FLOW_ACTION,
            "data": {
                "dispatchId": dispatch.id,
                "workspaceId": dispatch.workspace_id,
                "stepId": dispatch.step_id,
                "contactId": dispatch.contact_id,
                "contactInboxId": dispatch.contact_inbox_id,
                "enrollmentId": dispatch.enrollment_id,
                "sequenceId": dispatch.sequence_id,
                "bucket": dispatch.bucket,
                "metadata": metadata,
            },
        });
        self.deps
            .integration
            .add(
                QueueName::Integration,
                SEND_SEQUENCE_FLOW_ACTION,
                data,
                opts,
            )
            .await?;
        tracing::info!(
            workspace_id = %dispatch.workspace_id,
            source = EXECUTE_STEP_AUDIT_SOURCE,
            "Sequence step sent to contact"
        );
        Ok(())
    }
}

/// `IntegrationJobAction.sendSequenceFlow`.
pub const SEND_SEQUENCE_FLOW_ACTION: &str = "sendSequenceFlow";
