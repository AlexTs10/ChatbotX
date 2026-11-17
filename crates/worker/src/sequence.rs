//! The sequence scheduler of `apps/worker/src/sequence-scheduler` on the
//! SQLite schedule: the consumer services (`DispatchProcessorService`,
//! `StepExecutorService`, `RetrySchedulerService`), the `sendSequenceFlow`
//! integration job (`integration/handlers/sequence-flow.ts`) and the start
//! of the producer, consumer and reconcile loops.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_db::rusqlite::{OptionalExtension, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::runtime::{Runtime, WorkerSpec};
use chatbotx_jobs::sequence::advance::{AdvanceParams, CurrentStep, advance_enrollment};
use chatbotx_jobs::sequence::consumer::{
    DispatchRow, SEQUENCE_CONSUMER_CONCURRENCY, ScheduleConnector, SequenceConsumer,
    SequenceConsumerDeps, SequenceConsumerServices, StepRow, StepValidation,
    start_sequence_consumer,
};
use chatbotx_jobs::sequence::producer::{
    ProducerBackend, SchedulerWorker, SchedulerWorkerDeps, SchedulerWorkerOptions,
    SqlPendingDispatchLookup,
};
use chatbotx_jobs::sequence::reconcile::{
    ReconcileDeps, ReconcileJob, ReconcileOptions, SqlReconcileRepo,
};
use chatbotx_jobs::sequence::repo::SequenceCtx;
use chatbotx_jobs::sequence::retry::{RetrySchedulerService, revert_dispatch_to_pending};
use chatbotx_jobs::sequence::schedule::DispatchSchedule;
use chatbotx_jobs::sequence::sqlite_schedule::SqliteDispatchSchedule;
use chatbotx_jobs::shutdown::Shutdown;
use chatbotx_jobs::{
    BoxError, BoxFuture, Clock, Enqueuer, JobCtx, JobError, JobsError, QueueName, Retention,
    SystemClock,
};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::{Deps, db_fail, str_of};

/// `isDispatchReady` tolerance: a dispatch due within 1 s runs now.
pub const DISPATCH_READY_TOLERANCE_MS: i64 = 1000;

/// `DispatchProcessorService.isDispatchReady`.
pub fn is_dispatch_ready(run_at_ms: i64, now_ms: i64) -> bool {
    run_at_ms <= now_ms + DISPATCH_READY_TOLERANCE_MS
}

/// `DispatchProcessorService.validateDispatch`: only a pending dispatch runs.
pub fn validate_dispatch(status: Option<&str>) -> bool {
    status == Some("pending")
}

/// `StepExecutorService.validateStep`: missing, then inactive, then no flow.
pub fn validate_step(step: Option<&StepRow>) -> StepValidation {
    let reason = match step {
        None => Some("step_not_found"),
        Some(step) if !step.is_active => Some("step_inactive"),
        Some(step) if step.flow_id.is_none() => Some("flow_not_configured"),
        Some(_) => None,
    };
    StepValidation {
        valid: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

/// A `SequenceDispatch` row with its status and `completedAt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDispatch {
    pub row: DispatchRow,
    pub status: String,
    pub completed_at: Option<i64>,
}

fn id(text: &str) -> Result<i64, DbError> {
    text.trim()
        .parse()
        .map_err(|_| DbError::other(format!("invalid id {text:?}")))
}

/// `sequenceDispatchRepository.findWithRelations` / `findRunning`: the
/// dispatch of the workspace in `status`.
pub async fn find_dispatch(
    db: &Db,
    dispatch_id: &str,
    status: &'static str,
    workspace_id: &str,
) -> Result<Option<StoredDispatch>, DbError> {
    let (dispatch_id, workspace_id) = (id(dispatch_id)?, id(workspace_id)?);
    db.read(move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "id", "attempt", "bucket", "contactId", "contactInboxId", "enrollmentId",
                          "sequenceId", "stepId", "workspaceId", "runAtMs", "status", "completedAt"
                   FROM "SequenceDispatch" WHERE "id" = ?1 AND "status" = ?2 AND "workspaceId" = ?3"#,
                params![dispatch_id, status, workspace_id],
                |r| {
                    Ok(StoredDispatch {
                        row: DispatchRow {
                            id: r.get::<_, i64>(0)?.to_string(),
                            attempt: r.get(1)?,
                            bucket: r.get(2)?,
                            contact_id: r.get::<_, i64>(3)?.to_string(),
                            contact_inbox_id: r.get::<_, i64>(4)?.to_string(),
                            enrollment_id: r.get::<_, i64>(5)?.to_string(),
                            sequence_id: r.get::<_, i64>(6)?.to_string(),
                            step_id: r.get::<_, i64>(7)?.to_string(),
                            workspace_id: r.get::<_, i64>(8)?.to_string(),
                            run_at_ms: r.get(9)?,
                        },
                        status: r.get(10)?,
                        completed_at: r.get(11)?,
                    })
                },
            )
            .optional()?)
    })
    .await
}

/// `sequenceDispatchRepository.claim`: pending → running under this owner;
/// the `status = 'pending'` predicate is the guard against a second worker.
pub async fn claim_dispatch(
    db: &Db,
    dispatch_id: &str,
    workspace_id: &str,
    lock_owner: String,
    now_ms: i64,
) -> Result<bool, DbError> {
    let (dispatch_id, workspace_id) = (id(dispatch_id)?, id(workspace_id)?);
    db.write(move |tx| {
        Ok(tx.execute(
            r#"UPDATE "SequenceDispatch" SET "status" = 'running', "lockedAt" = ?3, "lockOwner" = ?4, "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'pending'"#,
            params![dispatch_id, workspace_id, now_ms, lock_owner],
        )? > 0)
    })
    .await
}

/// `sequenceDispatchRepository.findStepWithFlow`: the step and whether its flow exists.
pub async fn fetch_step(db: &Db, step_id: &str) -> Result<Option<StepRow>, DbError> {
    let step_id = id(step_id)?;
    db.read(move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT s."id", s."sequenceId", s."order", s."isActive", f."id"
                   FROM "SequenceStep" s LEFT JOIN "Flow" f ON f."id" = s."flowId"
                   WHERE s."id" = ?1"#,
                params![step_id],
                |r| {
                    Ok(StepRow {
                        id: r.get::<_, i64>(0)?.to_string(),
                        sequence_id: r.get::<_, i64>(1)?.to_string(),
                        order: r.get(2)?,
                        is_active: r.get(3)?,
                        flow_id: r.get(4)?,
                    })
                },
            )
            .optional()?)
    })
    .await
}

/// `sequenceDispatchUtils.markCompleted` (running rows only).
pub async fn mark_dispatch_completed(
    db: &Db,
    dispatch_id: &str,
    workspace_id: &str,
    sent_at: i64,
    now_ms: i64,
) -> Result<(), DbError> {
    let (dispatch_id, workspace_id) = (id(dispatch_id)?, id(workspace_id)?);
    db.write(move |tx| {
        tx.execute(
            r#"UPDATE "SequenceDispatch" SET "status" = 'completed', "completedAt" = ?3, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'running'"#,
            params![dispatch_id, workspace_id, sent_at, now_ms],
        )?;
        Ok(())
    })
    .await
}

/// `process.env.HOSTNAME || "unknown"`.
fn lock_owner() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The production consumer services.
pub struct ConsumerServices {
    pub db: Arc<Db>,
    pub schedule: Arc<dyn DispatchSchedule>,
    pub clock: Arc<dyn Clock>,
}

impl SequenceConsumerServices for ConsumerServices {
    fn is_blocked_workspace<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        Box::pin(async move {
            let ws = id(workspace_id)?;
            Ok(chatbotx_business::broadcast::send::is_blocked_workspace(&self.db, ws).await?)
        })
    }

    /// `fetchDispatch`: a failed query is logged and reads as missing.
    fn fetch_dispatch<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<DispatchRow>, BoxError>> {
        Box::pin(async move {
            match find_dispatch(&self.db, dispatch_id, "pending", workspace_id).await {
                Ok(found) => Ok(found.map(|d| d.row)),
                Err(err) => {
                    tracing::error!(err = %err, "Error fetchDispatch query failed");
                    Ok(None)
                }
            }
        })
    }

    /// The fetch reads pending rows only, so a found dispatch is valid.
    fn validate_dispatch<'a>(
        &'a self,
        _dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        Box::pin(async { Ok(validate_dispatch(Some("pending"))) })
    }

    fn is_dispatch_ready(&self, dispatch: &DispatchRow, now_ms: i64) -> bool {
        is_dispatch_ready(dispatch.run_at_ms, now_ms)
    }

    fn fetch_dispatch_run_at(&self, dispatch: &DispatchRow) -> Option<i64> {
        Some(dispatch.run_at_ms)
    }

    fn lock_dispatch<'a>(
        &'a self,
        dispatch: &'a DispatchRow,
    ) -> BoxFuture<'a, Result<bool, BoxError>> {
        Box::pin(async move {
            Ok(claim_dispatch(
                &self.db,
                &dispatch.id,
                &dispatch.workspace_id,
                lock_owner(),
                self.clock.now_ms(),
            )
            .await?)
        })
    }

    fn fetch_step<'a>(
        &'a self,
        step_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<StepRow>, BoxError>> {
        Box::pin(async move { Ok(fetch_step(&self.db, step_id).await?) })
    }

    fn validate_step<'a>(
        &'a self,
        step: &'a StepRow,
    ) -> BoxFuture<'a, Result<StepValidation, BoxError>> {
        let validation = validate_step(Some(step));
        Box::pin(async move { Ok(validation) })
    }

    fn mark_dispatch_canceled<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            RetrySchedulerService::new(self.db.clone(), self.clock.clone())
                .mark_dispatch_canceled(dispatch_id, workspace_id, reason)
                .await?;
            Ok(())
        })
    }

    fn revert_dispatch_to_pending<'a>(
        &'a self,
        dispatch_id: &'a str,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            revert_dispatch_to_pending(&self.db, self.clock.as_ref(), dispatch_id, workspace_id)
                .await?;
            Ok(())
        })
    }

    fn advance_past_step<'a>(
        &'a self,
        dispatch: &'a DispatchRow,
        step: &'a StepRow,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let ctx = SequenceCtx::new(self.db.clone(), self.schedule.clone(), self.clock.clone());
            advance(&ctx, dispatch, step, self.clock.now_ms()).await
        })
    }
}

/// `advanceEnrollment` after `step` was sent (or skipped) at `sent_at`.
async fn advance(
    ctx: &SequenceCtx,
    dispatch: &DispatchRow,
    step: &StepRow,
    sent_at_ms: i64,
) -> Result<(), BoxError> {
    advance_enrollment(
        ctx,
        AdvanceParams {
            enrollment_id: dispatch.enrollment_id.clone(),
            workspace_id: dispatch.workspace_id.clone(),
            sequence_id: dispatch.sequence_id.clone(),
            contact_id: dispatch.contact_id.clone(),
            current_step: CurrentStep {
                id: step.id.clone(),
                order: step.order,
            },
            sent_at_ms,
        },
    )
    .await?;
    Ok(())
}

/// The schedule and the `sequenceScheduler` queue for the producer and consumer.
struct Backend {
    schedule: Arc<dyn DispatchSchedule>,
    queue: Arc<dyn Enqueuer>,
}

impl ProducerBackend for Backend {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>> {
        let schedule = self.schedule.clone();
        Box::pin(async move { Ok(schedule) })
    }

    fn sequence_queue(&self) -> BoxFuture<'_, Result<Option<Arc<dyn Enqueuer>>, JobsError>> {
        let queue = self.queue.clone();
        Box::pin(async move { Ok(Some(queue)) })
    }
}

impl ScheduleConnector for Backend {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>> {
        ProducerBackend::connect_schedule(self)
    }
}

/// `parseInt(env || default)`: unset, unparsable or 0 is `None` (the default).
fn env_number<T: std::str::FromStr + PartialEq + Default>(name: &str) -> Option<T> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<T>().ok())
        .filter(|v| *v != T::default())
}

/// `sequence-scheduler/worker.ts`, `worker-producer.ts` and
/// `worker-consumer.ts`: starts the reconcile loops, the producer and the
/// `sequenceScheduler` worker, each stopped by the shutdown coordinator.
pub fn start(runtime: &Runtime, deps: &Deps, shutdown: &Arc<Shutdown>) -> Result<(), JobsError> {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let schedule: Arc<dyn DispatchSchedule> =
        Arc::new(SqliteDispatchSchedule::new(deps.db.clone(), None));
    let backend = Arc::new(Backend {
        schedule: schedule.clone(),
        queue: deps.jobs.clone(),
    });
    let requested = {
        let shutdown = shutdown.clone();
        Arc::new(move || shutdown.is_requested()) as Arc<dyn Fn() -> bool + Send + Sync>
    };

    // The consumer first, so a claimed dispatch always has a worker.
    let consumer_deps = SequenceConsumerDeps {
        connector: backend.clone(),
        services: Arc::new(ConsumerServices {
            db: deps.db.clone(),
            schedule: schedule.clone(),
            clock: clock.clone(),
        }),
        integration: deps.jobs.clone(),
        clock: clock.clone(),
        shutdown_requested: requested.clone(),
    };
    let consumer: Arc<OnceCell<Option<Arc<SequenceConsumer>>>> = Arc::new(OnceCell::new());
    runtime.spawn_worker(
        WorkerSpec::new(QueueName::SequenceScheduler)
            .concurrency(SEQUENCE_CONSUMER_CONCURRENCY)
            .retention(Retention::KeepCount(0), Retention::KeepCount(0)),
        Arc::new(
            move |job: JobCtx| -> BoxFuture<'static, Result<Value, JobError>> {
                let (consumer, consumer_deps) = (consumer.clone(), consumer_deps.clone());
                Box::pin(async move {
                    let started = consumer
                        .get_or_try_init(|| start_sequence_consumer(consumer_deps))
                        .await
                        .map_err(db_fail)?;
                    match started {
                        Some(consumer) => consumer.process(job.data).await.map(|()| Value::Null),
                        None => Err(JobError::retryable("Dispatch consumer is not started")),
                    }
                })
            },
        ),
    )?;

    let env: BTreeMap<String, String> = std::env::vars().collect();
    let producer = SchedulerWorker::new(
        SchedulerWorkerOptions::default(),
        &env,
        SchedulerWorkerDeps {
            backend,
            pending: Arc::new(SqlPendingDispatchLookup {
                db: deps.db.clone(),
            }),
            clock: clock.clone(),
            shutdown_requested: requested,
        },
    );
    {
        let producer = producer.clone();
        tokio::spawn(async move {
            if let Err(err) = producer.start().await {
                tracing::error!(err = %err, "Failed to start the sequence scheduler producer");
            }
        });
    }
    shutdown.on_shutdown(
        "sequence-scheduler-producer",
        Box::new(move || {
            Box::pin(async move {
                producer.stop().await;
                Ok(())
            })
        }),
    );

    let reconcile = Arc::new(ReconcileJob::new(
        ReconcileOptions {
            interval_ms: env_number("BOOTSTRAP_INTERVAL_MS"),
            cleanup_interval_ms: env_number("CLEANUP_INTERVAL_MS"),
            retention_interval_ms: env_number("SEQUENCE_DISPATCH_RETENTION_INTERVAL_MS"),
            retention_batch_size: env_number("SEQUENCE_DISPATCH_RETENTION_BATCH_SIZE"),
            retention_ttl_days: env_number("SEQUENCE_DISPATCH_RETENTION_TTL_DAYS"),
            buckets: None,
        },
        ReconcileDeps {
            schedule,
            repo: Arc::new(SqlReconcileRepo {
                db: deps.db.clone(),
            }),
            db: Some(deps.db.clone()),
            clock,
        },
    ));
    tokio::spawn(reconcile.run(shutdown.cancel_token()));
    Ok(())
}

// ---- sendSequenceFlow ----------------------------------------------------

/// `IntegrationJobSendSequenceFlow["data"]`.
struct SendSequenceFlow<'a> {
    data: &'a Value,
    dispatch_id: &'a str,
    workspace_id: &'a str,
    bucket: u32,
}

/// `handleSendSequenceFlow`: on a failure of the final attempt the dispatch is
/// marked failed and leaves the schedule; the error is always returned.
pub async fn handle_send_sequence_flow(deps: &Deps, job: &JobCtx) -> Result<(), JobError> {
    let data = &job.data["data"];
    let input = SendSequenceFlow {
        data,
        dispatch_id: str_of(data, "dispatchId").unwrap_or_default(),
        workspace_id: str_of(data, "workspaceId").unwrap_or_default(),
        bucket: data["bucket"].as_u64().unwrap_or_default() as u32,
    };
    let schedule: Arc<dyn DispatchSchedule> =
        Arc::new(SqliteDispatchSchedule::new(deps.db.clone(), None));
    let Err(err) = run_send_sequence_flow(deps, &schedule, job, &input).await else {
        return Ok(());
    };
    let final_attempt = job.is_final_attempt();
    tracing::error!(
        err = %err,
        dispatch_id = input.dispatch_id,
        job_id = %job.job_id,
        attempt = job.attempts_made + 1,
        is_final_attempt = final_attempt,
        "sendSequenceFlow handler failed"
    );
    if final_attempt {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        if let Err(cleanup) = RetrySchedulerService::new(deps.db.clone(), clock)
            .mark_dispatch_failed(input.dispatch_id, input.workspace_id, &err.to_string())
            .await
        {
            tracing::error!(err = %cleanup, dispatch_id = input.dispatch_id, job_id = %job.job_id, "markDispatchFailed failed in terminal cleanup");
        }
        if let Err(cleanup) = schedule
            .remove_from_schedule(input.bucket, input.dispatch_id)
            .await
        {
            tracing::error!(err = %cleanup, dispatch_id = input.dispatch_id, job_id = %job.job_id, "removeFromSchedule failed in terminal cleanup");
        }
    }
    Err(err)
}

async fn run_send_sequence_flow(
    deps: &Deps,
    schedule: &Arc<dyn DispatchSchedule>,
    job: &JobCtx,
    input: &SendSequenceFlow<'_>,
) -> Result<(), JobError> {
    let Some(dispatch) = find_dispatch(&deps.db, input.dispatch_id, "running", input.workspace_id)
        .await
        .map_err(db_fail)?
    else {
        return Ok(());
    };
    // The job data names the enrollment, sequence and contact (as the original).
    let row = DispatchRow {
        enrollment_id: str_of(input.data, "enrollmentId")
            .map_or(dispatch.row.enrollment_id.clone(), str::to_owned),
        sequence_id: str_of(input.data, "sequenceId")
            .map_or(dispatch.row.sequence_id.clone(), str::to_owned),
        contact_id: str_of(input.data, "contactId")
            .map_or(dispatch.row.contact_id.clone(), str::to_owned),
        ..dispatch.row.clone()
    };
    let step_id = str_of(input.data, "stepId").unwrap_or(&dispatch.row.step_id);
    let step = fetch_step(&deps.db, step_id).await.map_err(db_fail)?;
    let validation = validate_step(step.as_ref());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let ctx = SequenceCtx::new(deps.db.clone(), schedule.clone(), clock.clone());
    let (true, Some(step), Some(flow_id)) = (
        validation.valid,
        &step,
        step.as_ref().and_then(|s| s.flow_id),
    ) else {
        RetrySchedulerService::new(deps.db.clone(), clock.clone())
            .mark_dispatch_canceled(
                input.dispatch_id,
                input.workspace_id,
                validation.reason.as_deref().unwrap_or_default(),
            )
            .await
            .map_err(db_fail)?;
        if let Some(step) = &step {
            advance(&ctx, &row, step, clock.now_ms())
                .await
                .map_err(db_fail)?;
        }
        schedule
            .remove_from_schedule(input.bucket, input.dispatch_id)
            .await
            .map_err(db_fail)?;
        return Ok(());
    };
    let sent_at = match dispatch.completed_at {
        Some(completed_at) => completed_at,
        None => {
            send_flow_direct(deps, flow_id, &row, input.data, job).await?;
            let sent_at = clock.now_ms();
            mark_dispatch_completed(
                &deps.db,
                input.dispatch_id,
                input.workspace_id,
                sent_at,
                sent_at,
            )
            .await
            .map_err(db_fail)?;
            sent_at
        }
    };
    advance(&ctx, &row, step, sent_at).await.map_err(db_fail)?;
    schedule
        .remove_from_schedule(input.bucket, input.dispatch_id)
        .await
        .map_err(db_fail)?;
    Ok(())
}

/// `sendFlowDirect`: the flow on every contact inbox of the contact, in its
/// workspace conversation.
async fn send_flow_direct(
    deps: &Deps,
    flow_id: i64,
    dispatch: &DispatchRow,
    data: &Value,
    job: &JobCtx,
) -> Result<(), JobError> {
    let (contact_id, workspace_id) = (
        id(&dispatch.contact_id).map_err(db_fail)?,
        id(&dispatch.workspace_id).map_err(db_fail)?,
    );
    let (conversation, contact_inboxes) = deps
        .db
        .read(move |conn| {
            let conversation: Option<i64> = conn
                .query_row(
                    r#"SELECT "id" FROM "Conversation" WHERE "contactId" = ?1 AND "workspaceId" = ?2 LIMIT 1"#,
                    params![contact_id, workspace_id],
                    |r| r.get(0),
                )
                .optional()?;
            let mut stmt = conn.prepare(
                r#"SELECT ci."id" FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
                   WHERE ci."contactId" = ?1 AND i."workspaceId" = ?2 ORDER BY ci."id""#,
            )?;
            let inboxes = stmt
                .query_map(params![contact_id, workspace_id], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((conversation, inboxes))
        })
        .await
        .map_err(db_fail)?;
    let conversation = conversation.ok_or_else(|| {
        JobError::retryable(format!("Conversation not found for contact {contact_id}"))
    })?;
    // `flowExecutionKey: job.id` (`seq-<dispatch>-<attempt>`, fixed across retries).
    let key = job.job_id.clone();
    // ponytail: one inbox after another (the original runs them in parallel).
    for contact_inbox in contact_inboxes {
        let node = json!({
            "flowId": flow_id,
            "metadata": data.get("metadata").cloned().unwrap_or(Value::Null),
            "conversationId": conversation,
            "contactInboxId": contact_inbox,
        });
        crate::flow::run_flow_node(deps, &node, key.clone()).await?;
    }
    Ok(())
}
