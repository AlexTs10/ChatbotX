//! Port of `apps/worker/__tests__/sequence-worker-consumer.test.ts`
//! (spec 5.3 "sequence-worker-consumer"). The mocked services are
//! `FakeConsumerServices`; `SchedulerClient` is `FakeDispatchSchedule`;
//! the integration queue is `AuditEnqueuer` (reads the task audit actor).

#[path = "support/sequence_worker.rs"]
mod sequence_worker;
mod support;

use std::sync::Arc;

use chatbotx_jobs::sequence::consumer::{
    AuditActor, DispatchRow, SequenceConsumer, SequenceConsumerDeps, start_sequence_consumer,
};
use chatbotx_jobs::sequence::schedule::ScheduleError;
use chatbotx_jobs::{JobError, QueueName, Retention};
use sequence_worker::{
    AuditEnqueuer, FakeBackend, FakeConsumerServices, FakeDispatchSchedule, ScheduleCall,
};
use serde_json::json;
use support::{RecordingLog, TestClock};
use tracing::Level;

const NOW: i64 = 1_700_000_000_000;

struct Harness {
    schedule: Arc<FakeDispatchSchedule>,
    backend: Arc<FakeBackend>,
    services: Arc<FakeConsumerServices>,
    integration: Arc<AuditEnqueuer>,
}

fn harness(dispatch: Option<DispatchRow>) -> Harness {
    let schedule = FakeDispatchSchedule::new();
    Harness {
        backend: FakeBackend::new(Arc::clone(&schedule), None),
        schedule,
        services: FakeConsumerServices::new(dispatch),
        integration: Arc::new(AuditEnqueuer::default()),
    }
}

fn deps(h: &Harness, shutdown: bool) -> SequenceConsumerDeps {
    SequenceConsumerDeps {
        connector: Arc::clone(&h.backend) as _,
        services: Arc::clone(&h.services) as _,
        integration: Arc::clone(&h.integration) as _,
        clock: TestClock::new(NOW),
        shutdown_requested: Arc::new(move || shutdown),
    }
}

/// `startConsumer()`: the worker must be created.
async fn start(h: &Harness) -> Arc<SequenceConsumer> {
    start_sequence_consumer(deps(h, false))
        .await
        .expect("start consumer")
        .expect("worker created")
}

fn current_payload() -> serde_json::Value {
    json!({
        "bucket": 3,
        "claimedAt": NOW,
        "dispatchId": "dispatch-1",
        "workspaceId": "workspace-1",
    })
}

fn retryable_message(result: Result<(), JobError>) -> String {
    match result {
        Err(JobError::Retryable(e)) => e.to_string(),
        other => panic!("expected JobError::Retryable, got {other:?}"),
    }
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:176 "sequence scheduler consumer > does not create a Worker when shutdown began while Redis connected"
#[tokio::test]
async fn does_not_create_a_worker_when_shutdown_began_while_redis() {
    let log = RecordingLog::install();
    let h = harness(None);

    let consumer = start_sequence_consumer(deps(&h, true))
        .await
        .expect("start returns");

    assert!(h.backend.connects() >= 1);
    assert!(
        log.find_containing(Level::WARN, "Shutdown requested")
            .is_some(),
        "{:?}",
        log.entries()
    );
    assert!(consumer.is_none());
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:197 "sequence scheduler consumer > processes the legacy wrapper payload during the one-release cutover"
#[tokio::test]
async fn processes_the_legacy_wrapper_payload_during_the_one_release() {
    let h = harness(None);
    let consumer = start(&h).await;

    let value = json!({
        "bucket": 3,
        "dispatchId": "dispatch-1",
        "workspaceId": "workspace-1",
    })
    .to_string();
    consumer
        .process(json!({ "key": "dispatch-1", "value": value }))
        .await
        .expect("processed");

    assert_eq!(
        h.schedule.try_lock_calls(),
        vec![(3, "dispatch-1".to_owned(), 30)]
    );
    assert!(h.schedule.has(&ScheduleCall::RemoveFromSchedule {
        bucket: 3,
        id: "dispatch-1".into()
    }));
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:220 "sequence scheduler consumer > removes terminal jobs so the scheduler can enqueue a retried dispatch"
#[tokio::test]
async fn removes_terminal_jobs_so_the_scheduler_can_enqueue_a() {
    let h = harness(None);
    let consumer = start(&h).await;

    let spec = consumer.spec();
    assert_eq!(spec.queue, QueueName::SequenceScheduler);
    assert_eq!(spec.remove_on_complete, Retention::KeepCount(0));
    assert_eq!(spec.remove_on_fail, Retention::KeepCount(0));
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:231 "sequence scheduler consumer > sets the dispatch workspace audit context before enqueuing a sequence step"
#[tokio::test]
async fn sets_the_dispatch_workspace_audit_context_before_enqueuing() {
    let h = harness(Some(DispatchRow {
        id: "dispatch-1".into(),
        attempt: 0,
        bucket: 3,
        contact_id: "contact-1".into(),
        contact_inbox_id: "contact-inbox-1".into(),
        enrollment_id: "enrollment-1".into(),
        sequence_id: "sequence-1".into(),
        step_id: "step-1".into(),
        workspace_id: "workspace-1".into(),
    }));
    let consumer = start(&h).await;

    consumer
        .process(current_payload())
        .await
        .expect("processed");

    let calls = h.integration.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let (call, actor) = &calls[0];
    assert_eq!(call.queue, QueueName::Integration);
    let actor: &AuditActor = actor.as_ref().expect("audit actor set during add");
    assert_eq!(actor.source, "sequence-scheduler:executeStep");
    assert_eq!(actor.workspace_id, "workspace-1");
    // A scheduled sequence send yields to live traffic and flow continuations.
    assert_eq!(call.data["data"]["metadata"]["type"], "sequenceSchedule");
    assert_eq!(call.opts.priority, Some(5));
    assert_eq!(call.opts.job_id.as_deref(), Some("seq-dispatch-1-0"));
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:274 "sequence scheduler consumer > discards malformed legacy payloads without retrying"
#[tokio::test]
async fn discards_malformed_legacy_payloads_without_retrying() {
    let log = RecordingLog::install();
    let h = harness(None);
    let consumer = start(&h).await;

    let result = consumer
        .process(json!({ "key": "dispatch-1", "value": "{invalid" }))
        .await;

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(h.services.fetches(), 0);
    let entry = log
        .find_containing(Level::ERROR, "Failed to parse")
        .expect("parse failure logged");
    assert!(entry.field("err").is_some(), "{entry:?}");
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:288 "sequence scheduler consumer > skips lock contention without failing the BullMQ job"
#[tokio::test]
async fn skips_lock_contention_without_failing_the_bullmq_job() {
    let log = RecordingLog::install();
    let h = harness(None);
    h.schedule.fail_lock_once(ScheduleError::LockAcquisition {
        key: "seq:dispatch:{3}:lock:dispatch-1".into(),
    });
    let consumer = start(&h).await;

    let result = consumer.process(current_payload()).await;

    assert!(result.is_ok(), "{result:?}");
    let entry = log
        .find_containing(Level::DEBUG, "another worker owns the lock")
        .expect("lock contention logged at debug");
    assert!(entry.field("err").is_some(), "{entry:?}");
    assert_eq!(
        h.schedule.try_lock_calls(),
        vec![(3, "dispatch-1".to_owned(), 30)]
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:319 "sequence scheduler consumer > propagates non-lock failures so BullMQ retries the job"
#[tokio::test]
async fn propagates_non_lock_failures_so_bullmq_retries_the_job() {
    let log = RecordingLog::install();
    let h = harness(None);
    h.schedule
        .fail_lock_once(ScheduleError::Store("database unavailable".into()));
    let consumer = start(&h).await;

    let message = retryable_message(consumer.process(current_payload()).await);

    assert!(message.contains("database unavailable"), "{message}");
    let entry = log
        .find_containing(Level::ERROR, "propagating for BullMQ retry")
        .expect("failure logged");
    assert!(entry.field("err").is_some(), "{entry:?}");
}

/// Port of: apps/worker/__tests__/sequence-worker-consumer.test.ts:341 "sequence scheduler consumer > rethrows a lock error for a nested lock with a different key"
#[tokio::test]
async fn rethrows_a_lock_error_for_a_nested_lock_with_a_different_key() {
    let h = harness(None);
    // A lock error raised inside the dispatch lock, for another key.
    *h.services.fetch_error.lock().unwrap() = Some((
        "seq:dispatch:{3}:lock:nested".to_owned(),
        "nested lock held".to_owned(),
    ));
    let consumer = start(&h).await;

    let message = retryable_message(consumer.process(current_payload()).await);

    assert!(message.contains("nested lock held"), "{message}");
}
