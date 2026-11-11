//! Port of `apps/worker/__tests__/sequence-worker-producer.test.ts`
//! (spec 5.3 "sequence-worker-producer"). The `SchedulerClient` mock is
//! `FakeDispatchSchedule`; the `sequenceScheduler` queue mock is
//! `FakeEnqueuer` / `GatedEnqueuer`; timers are tokio paused time.

#[path = "support/sequence_worker.rs"]
mod sequence_worker;
mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chatbotx_jobs::sequence::buckets::assigned_buckets;
use chatbotx_jobs::sequence::producer::{
    BucketStats, ClaimedDispatch, SchedulerHealth, SchedulerWorker, SchedulerWorkerDeps,
    SchedulerWorkerOptions,
};
use chatbotx_jobs::sequence::schedule::{ScheduleError, ScheduleKind};
use chatbotx_jobs::sequence::tick::{EnvValue, resolve_tick_interval_ms};
use chatbotx_jobs::{Enqueuer, JobOptions, JobsError, QueueName};
use sequence_worker::{
    FakeBackend, FakeDispatchSchedule, FakePendingLookup, GatedEnqueuer, ScheduleCall, env,
};
use support::{FakeEnqueuer, RecordingLog, TestClock};
use tracing::Level;

const NOW: i64 = 1_700_000_000_000;

fn deps(
    backend: Arc<FakeBackend>,
    pending: Arc<FakePendingLookup>,
    shutdown: bool,
) -> SchedulerWorkerDeps {
    SchedulerWorkerDeps {
        backend,
        pending,
        clock: TestClock::new(NOW),
        shutdown_requested: Arc::new(move || shutdown),
    }
}

/// `new SchedulerWorker({ buckets, tickIntervalMs, ... })` with a backend
/// that hands out `schedule` and `queue`.
fn worker(
    options: SchedulerWorkerOptions,
    schedule: &Arc<FakeDispatchSchedule>,
    queue: Option<Arc<dyn Enqueuer>>,
    pending: Arc<FakePendingLookup>,
    shutdown: bool,
) -> (Arc<SchedulerWorker>, Arc<FakeBackend>) {
    let backend = FakeBackend::new(Arc::clone(schedule), queue);
    let w = SchedulerWorker::new(
        options,
        &BTreeMap::new(),
        deps(Arc::clone(&backend), pending, shutdown),
    );
    (w, backend)
}

/// `makeReadyWorker(options)`: buckets [0], tick 60 000, running, with the
/// schedule and queue attached.
fn ready_worker(
    options: SchedulerWorkerOptions,
    schedule: &Arc<FakeDispatchSchedule>,
    queue: Arc<dyn Enqueuer>,
    pending: Arc<FakePendingLookup>,
) -> Arc<SchedulerWorker> {
    let options = SchedulerWorkerOptions {
        buckets: options.buckets.or(Some(vec![0])),
        tick_interval_ms: options.tick_interval_ms.or(Some(60_000)),
        ..options
    };
    let backend = FakeBackend::new(Arc::clone(schedule), Some(Arc::clone(&queue)));
    SchedulerWorker::ready(
        options,
        &BTreeMap::new(),
        deps(backend, pending, false),
        Arc::clone(schedule) as _,
        queue,
    )
}

fn opts(buckets: &[u32], tick_interval_ms: u64) -> SchedulerWorkerOptions {
    SchedulerWorkerOptions {
        buckets: Some(buckets.to_vec()),
        tick_interval_ms: Some(tick_interval_ms),
        ..SchedulerWorkerOptions::default()
    }
}

fn bulk_names(queue: &FakeEnqueuer) -> Vec<Vec<String>> {
    queue
        .bulks()
        .into_iter()
        .map(|(_, jobs)| jobs.into_iter().map(|j| j.name).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// resolveTickIntervalMs
// ---------------------------------------------------------------------------

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:191 "resolveTickIntervalMs > uses a valid env value"
#[test]
fn uses_a_valid_env_value() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Number(2500.0))),
        2500
    );
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("2500"))),
        2500
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:196 "resolveTickIntervalMs > lets an explicit config value win, even below the env minimum"
#[test]
fn lets_an_explicit_config_value_win_even_below_the_env_minimum() {
    assert_eq!(
        resolve_tick_interval_ms(Some(5), Some(EnvValue::Number(2500.0))),
        5
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for undefined"
#[test]
fn falls_back_to_the_default_for_undefined() {
    assert_eq!(resolve_tick_interval_ms(None, None), 5000);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for empty string"
#[test]
fn falls_back_to_the_default_for_empty_string() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text(""))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for zero"
#[test]
fn falls_back_to_the_default_for_zero() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("0"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for non-numeric"
#[test]
fn falls_back_to_the_default_for_non_numeric() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("abc"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for below the minimum"
#[test]
fn falls_back_to_the_default_for_below_the_minimum() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("50"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for above the maximum"
#[test]
fn falls_back_to_the_default_for_above_the_maximum() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("20000"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for above the setTimeout limit"
#[test]
fn falls_back_to_the_default_for_above_the_settimeout_limit() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("99999999999"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for negative"
#[test]
fn falls_back_to_the_default_for_negative() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("-1000"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:202 "resolveTickIntervalMs > falls back to the default for fractional"
#[test]
fn falls_back_to_the_default_for_fractional() {
    assert_eq!(
        resolve_tick_interval_ms(None, Some(EnvValue::Text("1000.5"))),
        5000
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:216 "resolveTickIntervalMs > treats a zero config value as unset"
#[test]
fn treats_a_zero_config_value_as_unset() {
    assert_eq!(
        resolve_tick_interval_ms(Some(0), Some(EnvValue::Number(2500.0))),
        2500
    );
}

// ---------------------------------------------------------------------------
// SchedulerWorker tick interval
// ---------------------------------------------------------------------------

fn worker_with_env(
    options: SchedulerWorkerOptions,
    env_map: BTreeMap<String, String>,
) -> Arc<SchedulerWorker> {
    let schedule = FakeDispatchSchedule::new();
    let backend = FakeBackend::new(schedule, Some(FakeEnqueuer::new() as _));
    SchedulerWorker::new(
        options,
        &env_map,
        deps(backend, FakePendingLookup::with(&[]), false),
    )
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:225 "SchedulerWorker tick interval > reads the interval from env when none is passed"
#[test]
fn reads_the_interval_from_env_when_none_is_passed() {
    let w = worker_with_env(
        SchedulerWorkerOptions {
            buckets: Some(vec![0]),
            ..SchedulerWorkerOptions::default()
        },
        env(&[("SCHEDULER_TICK_INTERVAL_MS", "3000")]),
    );
    assert_eq!(w.config().tick_interval_ms, 3000);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:233 "SchedulerWorker tick interval > falls back to the default when env is missing"
#[test]
fn falls_back_to_the_default_when_env_is_missing() {
    let w = worker_with_env(
        SchedulerWorkerOptions {
            buckets: Some(vec![0]),
            ..SchedulerWorkerOptions::default()
        },
        env(&[]),
    );
    assert_eq!(w.config().tick_interval_ms, 5000);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:241 "SchedulerWorker tick interval > lets the constructor option override env"
#[test]
fn lets_the_constructor_option_override_env() {
    let w = worker_with_env(
        opts(&[0], 7),
        env(&[("SCHEDULER_TICK_INTERVAL_MS", "3000")]),
    );
    assert_eq!(w.config().tick_interval_ms, 7);
}

// ---------------------------------------------------------------------------
// SchedulerWorker lifecycle
// ---------------------------------------------------------------------------

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:252 "SchedulerWorker lifecycle > initializes the typed singleton queue once"
#[tokio::test(start_paused = true)]
async fn initializes_the_typed_singleton_queue_once() {
    let schedule = FakeDispatchSchedule::new();
    let (w, backend) = worker(
        opts(&[0], 60_000),
        &schedule,
        Some(FakeEnqueuer::new() as _),
        FakePendingLookup::with(&[]),
        false,
    );

    w.start().await.expect("first start");
    w.start().await.expect("second start");

    assert_eq!(backend.connects(), 1);
    assert_eq!(backend.queue_lookups(), 1);
    w.stop().await;
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:263 "SchedulerWorker lifecycle > fails startup when the sequence scheduler queue is unavailable"
#[tokio::test(start_paused = true)]
async fn fails_startup_when_the_sequence_scheduler_queue_is() {
    let schedule = FakeDispatchSchedule::new();
    let (w, _backend) = worker(
        opts(&[0], 60_000),
        &schedule,
        None,
        FakePendingLookup::with(&[]),
        false,
    );

    let err = w.start().await.expect_err("start must fail");
    assert!(
        err.to_string()
            .contains("Sequence scheduler queue is unavailable"),
        "{err:?}"
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:272 "SchedulerWorker lifecycle > starts no bucket tick when shutdown began while Redis was connecting"
#[tokio::test(start_paused = true)]
async fn starts_no_bucket_tick_when_shutdown_began_while_redis_was() {
    let schedule = FakeDispatchSchedule::new();
    let (w, _backend) = worker(
        opts(&[0, 1], 5),
        &schedule,
        Some(FakeEnqueuer::new() as _),
        FakePendingLookup::with(&[]),
        true,
    );

    w.start().await.expect("start");

    assert!(!w.is_running());
    assert!(w.timer_buckets().is_empty());
    assert_eq!(schedule.get_due_calls(), 0);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:284 "SchedulerWorker lifecycle > clears bucket timers exactly once when stopped"
#[tokio::test(start_paused = true)]
async fn clears_bucket_timers_exactly_once_when_stopped() {
    let schedule = FakeDispatchSchedule::new();
    let (w, _backend) = worker(
        opts(&[0, 1], 60_000),
        &schedule,
        Some(FakeEnqueuer::new() as _),
        FakePendingLookup::with(&[]),
        false,
    );

    w.start().await.expect("start");
    w.stop().await;
    w.stop().await;

    assert!(w.timer_buckets().is_empty());
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:297 "SchedulerWorker lifecycle > keeps one timer per bucket across repeated ticks"
#[tokio::test(start_paused = true)]
async fn keeps_one_timer_per_bucket_across_repeated_ticks() {
    let schedule = FakeDispatchSchedule::new();
    let (w, _backend) = worker(
        opts(&[0, 1, 2], 5),
        &schedule,
        Some(FakeEnqueuer::new() as _),
        FakePendingLookup::with(&[]),
        false,
    );

    w.start().await.expect("start");
    tokio::time::sleep(Duration::from_millis(120)).await;

    let timers = w.timer_buckets();
    assert_eq!(timers, vec![0, 1, 2]);
    assert_eq!(timers.len(), 3);
    w.stop().await;
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:313 "SchedulerWorker lifecycle > stop waits for a running tick to finish publishing its claimed dispatches"
#[tokio::test(start_paused = true)]
async fn stop_waits_for_a_running_tick_to_finish_publishing_its() {
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["d1"]);
    let queue = GatedEnqueuer::new();
    let (w, _backend) = worker(
        opts(&[0], 60_000),
        &schedule,
        Some(Arc::clone(&queue) as _),
        FakePendingLookup::with(&[("d1", "ws-1")]),
        false,
    );

    w.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(5), queue.started.notified())
        .await
        .expect("addBulk called");
    assert_eq!(queue.inner.bulks().len(), 1);

    let stopping = {
        let w = Arc::clone(&w);
        tokio::spawn(async move { w.stop().await })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!stopping.is_finished());

    queue.release();
    stopping.await.expect("stop task");
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:339 "SchedulerWorker lifecycle > does not schedule another bucket tick after stop"
#[tokio::test(start_paused = true)]
async fn does_not_schedule_another_bucket_tick_after_stop() {
    let schedule = FakeDispatchSchedule::new();
    let (w, _backend) = worker(
        opts(&[0], 5),
        &schedule,
        Some(FakeEnqueuer::new() as _),
        FakePendingLookup::with(&[]),
        false,
    );

    w.start().await.expect("start");
    tokio::time::sleep(Duration::from_millis(30)).await;
    w.stop().await;
    schedule.clear_calls();

    tokio::time::sleep(Duration::from_millis(60)).await;

    assert_eq!(schedule.get_due_calls(), 0);
}

// ---------------------------------------------------------------------------
// SchedulerWorker.processBucket
// ---------------------------------------------------------------------------

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:355 "SchedulerWorker.processBucket > returns before locking when neither zset has due entries"
#[tokio::test(start_paused = true)]
async fn returns_before_locking_when_neither_zset_has_due_entries() {
    let schedule = FakeDispatchSchedule::new();
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[]),
    );

    w.process_bucket(0).await;

    assert!(schedule.try_lock_calls().is_empty());
    assert!(queue.bulks().is_empty());
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:364 "SchedulerWorker.processBucket > claims due schedule and retry entries, then publishes them together"
#[tokio::test(start_paused = true)]
async fn claims_due_schedule_and_retry_entries_then_publishes_them() {
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["scheduled"]);
    schedule.push_due(ScheduleKind::Retry, &["retry"]);
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("scheduled", "workspace-1"), ("retry", "workspace-2")]),
    );

    w.process_bucket(0).await;

    assert!(schedule.has(&ScheduleCall::RemoveFromSchedule {
        bucket: 0,
        id: "scheduled".into()
    }));
    assert!(schedule.has(&ScheduleCall::RemoveFromRetry {
        bucket: 0,
        id: "retry".into()
    }));
    assert_eq!(
        bulk_names(&queue),
        vec![vec!["scheduled".to_owned(), "retry".to_owned()]]
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:385 "SchedulerWorker.processBucket > publishes only entries whose dispatch lock was acquired"
#[tokio::test(start_paused = true)]
async fn publishes_only_entries_whose_dispatch_lock_was_acquired() {
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["claimed", "locked"]);
    schedule.fail_lock_for("locked", ScheduleError::Store("lock held".into()));
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("claimed", "workspace-1")]),
    );

    w.process_bucket(0).await;

    assert!(schedule.has(&ScheduleCall::RemoveFromSchedule {
        bucket: 0,
        id: "claimed".into()
    }));
    assert!(!schedule.has(&ScheduleCall::RemoveFromSchedule {
        bucket: 0,
        id: "locked".into()
    }));
    assert_eq!(bulk_names(&queue), vec![vec!["claimed".to_owned()]]);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:413 "SchedulerWorker.processBucket > uses scheduler lock TTL as seconds when claiming"
#[tokio::test(start_paused = true)]
async fn uses_scheduler_lock_ttl_as_seconds_when_claiming() {
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["dispatch-1"]);
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions {
            lock_ttl_ms: Some(60_000),
            ..SchedulerWorkerOptions::default()
        },
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("dispatch-1", "workspace-1")]),
    );

    w.process_bucket(0).await;

    assert_eq!(
        schedule.try_lock_calls(),
        vec![(0, "dispatch-1".to_owned(), 60)]
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:431 "SchedulerWorker.processBucket > re-inserts claimed dispatches when BullMQ publishing fails"
#[tokio::test(start_paused = true)]
async fn re_inserts_claimed_dispatches_when_bullmq_publishing_fails() {
    let log = RecordingLog::install();
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["scheduled"]);
    schedule.push_due(ScheduleKind::Retry, &["retry"]);
    let queue = FakeEnqueuer::failing(JobsError::Unavailable("queue unavailable".into()));
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("scheduled", "workspace-1"), ("retry", "workspace-1")]),
    );

    w.process_bucket(0).await;

    let batches: Vec<_> = schedule
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            ScheduleCall::BatchAdd(entries) => Some(entries),
            _ => None,
        })
        .collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].bucket, 0);
    assert_eq!(batches[0][0].dispatch_id, "scheduled");
    assert!(schedule.calls().iter().any(|c| matches!(
        c,
        ScheduleCall::AddToRetry { bucket: 0, id, .. } if id == "retry"
    )));
    let entry = log
        .find_containing(Level::ERROR, "Failed to publish")
        .expect("publish failure logged");
    assert_eq!(entry.field("bucket"), Some("0"));
    assert_eq!(entry.field("count"), Some("2"));
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:455 "SchedulerWorker.processBucket > reports only failed reinsertion entries"
#[tokio::test(start_paused = true)]
async fn reports_only_failed_reinsertion_entries() {
    let log = RecordingLog::install();
    let schedule = FakeDispatchSchedule::new();
    schedule.push_due(ScheduleKind::Schedule, &["scheduled"]);
    schedule.push_due(ScheduleKind::Retry, &["retry"]);
    schedule.fail_next(
        "batch_add_to_schedule",
        ScheduleError::Store("schedule unavailable".into()),
    );
    let queue = FakeEnqueuer::failing(JobsError::Unavailable("queue unavailable".into()));
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("scheduled", "workspace-1"), ("retry", "workspace-1")]),
    );

    w.process_bucket(0).await;

    let last = log
        .at(Level::ERROR)
        .pop()
        .expect("an error was logged last");
    assert!(last.message.contains("Failed to re-insert"), "{last:?}");
    assert!(
        last.field("err")
            .is_some_and(|e| e.contains("schedule unavailable")),
        "{last:?}"
    );
    assert_eq!(last.field("bucket"), Some("0"));
    // `dispatch_ids` is recorded with `?` (Debug of the id list).
    assert_eq!(last.field("dispatch_ids"), Some(r#"["scheduled"]"#));
}

// ---------------------------------------------------------------------------
// SchedulerWorker.publishDispatches
// ---------------------------------------------------------------------------

fn claimed(ids: &[&str]) -> Vec<ClaimedDispatch> {
    ids.iter()
        .map(|id| ClaimedDispatch {
            dispatch_id: id.to_string(),
            source: ScheduleKind::Schedule,
        })
        .collect()
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:483 "SchedulerWorker.publishDispatches > prefixes numeric dispatch IDs for BullMQ while preserving dispatch identity"
#[tokio::test(start_paused = true)]
async fn prefixes_numeric_dispatch_ids_for_bullmq_while_preserving() {
    let dispatch_id = "18446744073709551615";
    let schedule = FakeDispatchSchedule::new();
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[(dispatch_id, "workspace-1")]),
    );

    w.publish_dispatches(7, &claimed(&[dispatch_id])).await;

    let bulks = queue.bulks();
    assert_eq!(bulks.len(), 1);
    let (queue_name, jobs) = &bulks[0];
    assert_eq!(*queue_name, QueueName::SequenceScheduler);
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.name, dispatch_id);
    assert_eq!(
        support::sorted_keys(&job.data),
        vec!["bucket", "claimedAt", "dispatchId", "workspaceId"]
    );
    assert_eq!(job.data["bucket"], 7);
    assert!(job.data["claimedAt"].is_number());
    assert_eq!(job.data["dispatchId"], dispatch_id);
    assert_eq!(job.data["workspaceId"], "workspace-1");
    assert_eq!(
        job.opts,
        JobOptions {
            job_id: Some(format!("sequence-{dispatch_id}")),
            ..JobOptions::default()
        }
    );
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:506 "SchedulerWorker.publishDispatches > filters claimed IDs that no longer have a pending row"
#[tokio::test(start_paused = true)]
async fn filters_claimed_ids_that_no_longer_have_a_pending_row() {
    let schedule = FakeDispatchSchedule::new();
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[("dispatch-1", "workspace-1")]),
    );

    w.publish_dispatches(0, &claimed(&["dispatch-1", "missing"]))
        .await;

    assert_eq!(bulk_names(&queue), vec![vec!["dispatch-1".to_owned()]]);
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:522 "SchedulerWorker.publishDispatches > does not enqueue when no claimed ID remains pending"
#[tokio::test(start_paused = true)]
async fn does_not_enqueue_when_no_claimed_id_remains_pending() {
    let schedule = FakeDispatchSchedule::new();
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[]),
    );

    w.publish_dispatches(0, &claimed(&["missing"])).await;

    assert!(queue.is_empty());
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:530 "SchedulerWorker.publishDispatches > batches all pending dispatches into one queue operation"
#[tokio::test(start_paused = true)]
async fn batches_all_pending_dispatches_into_one_queue_operation() {
    let schedule = FakeDispatchSchedule::new();
    let queue = FakeEnqueuer::new();
    let w = ready_worker(
        SchedulerWorkerOptions::default(),
        &schedule,
        Arc::clone(&queue) as _,
        FakePendingLookup::with(&[
            ("dispatch-1", "workspace-1"),
            ("dispatch-2", "workspace-2"),
            ("dispatch-3", "workspace-3"),
        ]),
    );

    w.publish_dispatches(10, &claimed(&["dispatch-1", "dispatch-2", "dispatch-3"]))
        .await;

    let bulks = queue.bulks();
    assert_eq!(bulks.len(), 1);
    assert_eq!(bulks[0].1.len(), 3);
    assert!(queue.calls().is_empty());
}

// ---------------------------------------------------------------------------
// getAssignedBuckets
// ---------------------------------------------------------------------------

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:551 "getAssignedBuckets > uses all 256 buckets when no assignment is configured"
#[test]
fn uses_all_256_buckets_when_no_assignment_is_configured() {
    assert_eq!(assigned_buckets(None), (0..256).collect::<Vec<u32>>());
}

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:557 "getAssignedBuckets > parses explicit lists and inclusive ranges"
#[test]
fn parses_explicit_lists_and_inclusive_ranges() {
    assert_eq!(assigned_buckets(Some("0,5,200")), vec![0, 5, 200]);
    assert_eq!(assigned_buckets(Some("10-12")), vec![10, 11, 12]);
}

// ---------------------------------------------------------------------------
// SchedulerWorker.getHealth
// ---------------------------------------------------------------------------

/// Port of: apps/worker/__tests__/sequence-worker-producer.test.ts:567 "SchedulerWorker.getHealth > returns per-bucket schedule and retry counts"
#[tokio::test(start_paused = true)]
async fn returns_per_bucket_schedule_and_retry_counts() {
    let schedule = FakeDispatchSchedule::new();
    schedule.set_count(ScheduleKind::Schedule, 5);
    schedule.set_count(ScheduleKind::Retry, 2);
    let w = ready_worker(
        SchedulerWorkerOptions {
            buckets: Some(vec![3, 7]),
            ..SchedulerWorkerOptions::default()
        },
        &schedule,
        FakeEnqueuer::new() as _,
        FakePendingLookup::with(&[]),
    );

    let health = w.health().await.expect("health");

    let stats = BucketStats {
        schedule: 5,
        retry: 2,
    };
    assert_eq!(
        health,
        SchedulerHealth {
            running: true,
            buckets: vec![3, 7],
            stats: BTreeMap::from([(3, stats), (7, stats)]),
        }
    );
    assert_eq!(schedule.count_calls(ScheduleKind::Schedule), 2);
    assert_eq!(schedule.count_calls(ScheduleKind::Retry), 2);
}
