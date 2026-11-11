//! Port of `apps/worker/__tests__/create-queue-worker.test.ts` (spec 5.3
//! "create-queue-worker"): `WorkerSpec` defaults and `Runtime::spawn_worker`
//! with its `failed` / `error` listeners and its shutdown closer.

#[path = "support/shutdown.rs"]
mod shutdown_support;
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chatbotx_jobs::runtime::{FailedHook, FailedJob, Handler, Runtime, WorkerSpec};
use chatbotx_jobs::shutdown::{Shutdown, ShutdownReason};
use chatbotx_jobs::{BoxError, BoxFuture, JobStore, JobsError, QueueName, Retention};
use serde_json::Value;
use shutdown_support::{ExitRecorder, err, shutdown_with, within};
use support::{RecordingLog, TestClock, migrated_db};
use tracing::Level;

fn noop_handler() -> Handler {
    Arc::new(|_job| Box::pin(async { Ok(Value::Null) }))
}

async fn runtime(shutdown: &Arc<Shutdown>) -> Runtime {
    let clock = TestClock::new(0);
    let store = JobStore::open(migrated_db().await, clock.clone())
        .await
        .expect("job store");
    Runtime::new(Arc::new(store), Arc::clone(shutdown), clock)
}

fn failed(queue: QueueName, job_id: &str) -> FailedJob {
    FailedJob {
        queue,
        job_id: job_id.to_owned(),
        reason: "boom".to_owned(),
        final_attempt: true,
    }
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:62 "createQueueWorker > merges the queue connection and defaults, with caller options winning"
#[test]
fn merges_the_queue_connection_and_defaults_with_caller_options() {
    let spec = WorkerSpec::new(QueueName::Chat).concurrency(30);

    assert_eq!(spec.queue, QueueName::Chat);
    assert_eq!(spec.concurrency, 30);
    assert_eq!(spec.remove_on_complete, Retention::KeepCount(1000));
    assert_eq!(spec.remove_on_fail, Retention::KeepCount(5000));
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:74 "createQueueWorker > attaches error and failed listeners"
#[test]
fn attaches_error_and_failed_listeners() {
    let mut listeners = WorkerSpec::new(QueueName::Chat).listeners();
    listeners.sort_unstable();

    assert_eq!(listeners, vec!["error", "failed"]);
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:80 "createQueueWorker > logs a failed job with its queue and id"
#[tokio::test]
async fn logs_a_failed_job_with_its_queue_and_id() {
    let records = RecordingLog::install();
    let shutdown = shutdown_with(&ExitRecorder::new());
    let runtime = runtime(&shutdown).await;
    let worker = runtime
        .spawn_worker(WorkerSpec::new(QueueName::Chat), noop_handler())
        .expect("worker");

    within(worker.report_failed(failed(QueueName::Chat, "7")))
        .await
        .expect("failed listener");

    let error = records.find("Job has failed").expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("queue"), Some("chat"));
    assert_eq!(error.field("job_id"), Some("7"));
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:94 "createQueueWorker > a custom onFailed replaces the default logger"
#[tokio::test]
async fn a_custom_onfailed_replaces_the_default_logger() {
    let records = RecordingLog::install();
    let shutdown = shutdown_with(&ExitRecorder::new());
    let runtime = runtime(&shutdown).await;
    let calls = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&calls);
    let hook: FailedHook = Arc::new(move |_job| {
        seen.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) }) as BoxFuture<'static, Result<(), BoxError>>
    });
    let worker = runtime
        .spawn_worker(
            WorkerSpec::new(QueueName::Chat).on_failed(hook),
            noop_handler(),
        )
        .expect("worker");

    within(worker.report_failed(failed(QueueName::Chat, "7")))
        .await
        .expect("failed listener");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(records.at(Level::ERROR).is_empty());
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:110 "createQueueWorker > a rejecting onFailed is logged and never becomes an unhandled rejection"
#[tokio::test]
async fn a_rejecting_onfailed_is_logged_and_never_becomes_an() {
    let records = RecordingLog::install();
    let shutdown = shutdown_with(&ExitRecorder::new());
    let runtime = runtime(&shutdown).await;
    let hook: FailedHook = Arc::new(|_job| {
        Box::pin(async { Err(err("db down")) }) as BoxFuture<'static, Result<(), BoxError>>
    });
    let worker = runtime
        .spawn_worker(
            WorkerSpec::new(QueueName::Heavy).on_failed(hook),
            noop_handler(),
        )
        .expect("worker");

    within(worker.report_failed(failed(QueueName::Heavy, "9")))
        .await
        .expect("the hook error is handled inside the listener task");

    let error = records
        .find("Failed-job handler threw")
        .expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("queue"), Some("heavy"));
    assert_eq!(error.field("job_id"), Some("9"));
    // No unhandled rejection: the worker keeps running, no crash path.
    assert!(!shutdown.is_requested());
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:133 "createQueueWorker > does not construct a worker once shutdown has begun"
#[tokio::test]
async fn does_not_construct_a_worker_once_shutdown_has_begun() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let runtime = runtime(&shutdown).await;
    within(shutdown.request(ShutdownReason::Sigterm, None)).await;

    let result = runtime.spawn_worker(WorkerSpec::new(QueueName::Chat), noop_handler());

    let Err(error) = result else {
        panic!("spawn_worker must fail after shutdown began");
    };
    assert!(matches!(error, JobsError::Shutdown(_)), "{error:?}");
    assert!(
        error.to_string().contains("shutdown already requested"),
        "{error}"
    );
    assert!(shutdown.closer_names().is_empty());
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:143 "createQueueWorker > registers a closer named after the queue that closes the worker"
#[tokio::test]
async fn registers_a_closer_named_after_the_queue_that_closes_the() {
    let exit = ExitRecorder::new();
    let shutdown = shutdown_with(&exit);
    let runtime = runtime(&shutdown).await;
    let worker = runtime
        .spawn_worker(WorkerSpec::new(QueueName::Heavy), noop_handler())
        .expect("worker");

    assert_eq!(shutdown.closer_names(), vec!["heavy".to_owned()]);
    within(shutdown.request(ShutdownReason::Sigterm, None)).await;
    assert_eq!(worker.close_count(), 1);
}

/// Port of: apps/worker/__tests__/create-queue-worker.test.ts:151 "createQueueWorker > an error event is logged and does not trigger shutdown"
#[tokio::test]
async fn an_error_event_is_logged_and_does_not_trigger_shutdown() {
    let records = RecordingLog::install();
    let shutdown = shutdown_with(&ExitRecorder::new());
    let runtime = runtime(&shutdown).await;
    let worker = runtime
        .spawn_worker(WorkerSpec::new(QueueName::Chat), noop_handler())
        .expect("worker");

    worker.report_error(&JobsError::Store("ECONNRESET".to_owned()));

    let error = records.find("Queue worker error").expect("error logged");
    assert_eq!(error.level, Level::ERROR);
    assert_eq!(error.field("queue"), Some("chat"));
    assert!(!shutdown.is_requested());
}
