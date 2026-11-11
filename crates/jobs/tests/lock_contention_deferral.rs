//! Port of `apps/worker/__tests__/lock-contention-deferral.test.ts`
//! (`deferOnLockContention`, spec 1.4 / S7). `moveToDelayed` + `DelayedError`
//! = `JobError::Delayed { run_at_ms }`; the worker token = `JobCtx::lease_token`.

mod support;

use std::fmt;

use chatbotx_jobs::deferral::{
    LOCK_CONTENTION_POLICY, LockAcquisitionError, defer_on_lock_contention, deferrals_so_far,
    lock_contention_delay_ms,
};
use chatbotx_jobs::{BoxError, JobCtx, JobError};
use support::{RecordingLog, TestClock};
use tracing::Level;

fn make_job(attempts_started: u32, attempts_made: u32) -> JobCtx {
    JobCtx {
        job_id: "job-1".to_owned(),
        name: "incomingMessage".to_owned(),
        attempts_started,
        attempts_made,
        lease_token: Some("token".to_owned()),
        ..JobCtx::default()
    }
}

fn lock_error() -> BoxError {
    Box::new(LockAcquisitionError {
        message: "Failed to acquire lock".to_owned(),
    })
}

/// A plain error with an identity, so "the same error" can be checked.
#[derive(Debug)]
struct Boom {
    id: u32,
}

impl fmt::Display for Boom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("boom")
    }
}

impl std::error::Error for Boom {}

/// A different error type that wraps a lock error (the TypeScript
/// "foreign Error named LockAcquisitionError").
#[derive(Debug)]
struct Foreign {
    source: LockAcquisitionError,
}

impl fmt::Display for Foreign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Failed to acquire lock")
    }
}

impl std::error::Error for Foreign {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn random() -> f64 {
    0.37
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:40 "deferralsSoFar > started=1 made=0 → 0"
#[test]
fn started_made_1_0() {
    assert_eq!(deferrals_so_far(1, 0), 0);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:40 "deferralsSoFar > started=3 made=0 → 2"
#[test]
fn started_made_3_0() {
    assert_eq!(deferrals_so_far(3, 0), 2);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:40 "deferralsSoFar > started=2 made=1 → 0"
#[test]
fn started_made_2_1() {
    assert_eq!(deferrals_so_far(2, 1), 0);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:40 "deferralsSoFar > started=4 made=1 → 2"
#[test]
fn started_made_4_1() {
    assert_eq!(deferrals_so_far(4, 1), 2);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:40 "deferralsSoFar > started=0 made=0 → 0"
#[test]
fn started_made_0_0() {
    assert_eq!(deferrals_so_far(0, 0), 0);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:58 "lockContentionDelayMs > doubles per deferral with equal jitter and caps at maxDelayMs"
#[test]
fn doubles_per_deferral_with_equal_jitter_and_caps_at_maxdelayms() {
    let base = LOCK_CONTENTION_POLICY.base_delay_ms;
    let max = LOCK_CONTENTION_POLICY.max_delay_ms;
    assert_eq!(base, 2000);
    assert_eq!(max, 30_000);
    assert_eq!(lock_contention_delay_ms(0, &|| 0.0), base / 2);
    assert_eq!(lock_contention_delay_ms(0, &|| 1.0), base);
    assert_eq!(lock_contention_delay_ms(1, &|| 1.0), base * 2);
    assert_eq!(lock_contention_delay_ms(2, &|| 0.5), base * 4 * 3 / 4);
    assert_eq!(lock_contention_delay_ms(10, &|| 1.0), max);
    assert_eq!(lock_contention_delay_ms(10, &|| 0.0), max / 2);
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:70 "deferOnLockContention > returns the processor result untouched"
#[tokio::test]
async fn returns_the_processor_result_untouched() {
    let job = make_job(1, 0);
    let clock = TestClock::new(1_000_000);

    let result =
        defer_on_lock_contention(&job, &*clock, &random, async { Ok::<_, BoxError>("done") }).await;

    assert!(matches!(result, Ok("done")));
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:80 "deferOnLockContention > rethrows any other error without touching the job"
#[tokio::test]
async fn rethrows_any_other_error_without_touching_the_job() {
    let log = RecordingLog::install();
    let job = make_job(1, 0);
    let clock = TestClock::new(1_000_000);

    let result = defer_on_lock_contention(&job, &*clock, &random, async {
        Err::<(), BoxError>(Box::new(Boom { id: 42 }))
    })
    .await;

    match result {
        Err(JobError::Retryable(e)) => {
            assert_eq!(e.downcast_ref::<Boom>().map(|b| b.id), Some(42));
        }
        other => panic!("expected the same error back, got {other:?}"),
    }
    assert!(log.at(Level::WARN).is_empty());
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:92 "deferOnLockContention > parks the job in the delayed set and signals BullMQ with DelayedError"
#[tokio::test]
async fn parks_the_job_in_the_delayed_set_and_signals_bullmq_with() {
    let log = RecordingLog::install();
    let job = make_job(1, 0);
    let clock = TestClock::new(1_000_000);

    let result =
        defer_on_lock_contention(&job, &*clock, &random, async { Err::<(), _>(lock_error()) })
            .await;

    let base = LOCK_CONTENTION_POLICY.base_delay_ms as i64;
    match result {
        Err(JobError::Delayed { run_at_ms }) => {
            assert!(run_at_ms >= 1_000_000 + base / 2, "run_at {run_at_ms}");
            assert!(run_at_ms <= 1_000_000 + base, "run_at {run_at_ms}");
        }
        other => panic!("expected Delayed, got {other:?}"),
    }
    let warns = log.at(Level::WARN);
    assert_eq!(warns.len(), 1);
    let warn = &warns[0];
    assert_eq!(warn.field("job_id"), Some("job-1"));
    assert_eq!(warn.field("deferral"), Some("0"));
    assert!(
        warn.field("err")
            .is_some_and(|e| e.contains("Failed to acquire lock"))
    );
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:116 "deferOnLockContention > recognises the error by name when it is not the same class instance"
#[tokio::test]
async fn recognises_the_error_by_name_when_it_is_not_the_same_class() {
    let job = make_job(1, 0);
    let clock = TestClock::new(1_000_000);
    let foreign: BoxError = Box::new(Foreign {
        source: LockAcquisitionError {
            message: "Failed to acquire lock".to_owned(),
        },
    });

    let result =
        defer_on_lock_contention(&job, &*clock, &random, async { Err::<(), _>(foreign) }).await;

    assert!(
        matches!(result, Err(JobError::Delayed { .. })),
        "{result:?}"
    );
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:126 "deferOnLockContention > cannot defer without the worker token, so the error propagates"
#[tokio::test]
async fn cannot_defer_without_the_worker_token_so_the_error_propagates() {
    let job = JobCtx {
        lease_token: None,
        ..make_job(1, 0)
    };
    let clock = TestClock::new(1_000_000);

    let result =
        defer_on_lock_contention(&job, &*clock, &random, async { Err::<(), _>(lock_error()) })
            .await;

    match result {
        Err(JobError::Retryable(e)) => assert!(e.downcast_ref::<LockAcquisitionError>().is_some()),
        other => panic!("expected the lock error back, got {other:?}"),
    }
}

/// Port of: apps/worker/__tests__/lock-contention-deferral.test.ts:137 "deferOnLockContention > lets the error fail the attempt once the deferral budget is spent"
#[tokio::test]
async fn lets_the_error_fail_the_attempt_once_the_deferral_budget_is() {
    let job = make_job(LOCK_CONTENTION_POLICY.max_deferrals + 1, 0);
    let clock = TestClock::new(1_000_000);

    let result =
        defer_on_lock_contention(&job, &*clock, &random, async { Err::<(), _>(lock_error()) })
            .await;

    match result {
        Err(JobError::Retryable(e)) => assert!(e.downcast_ref::<LockAcquisitionError>().is_some()),
        other => panic!("expected the lock error back, got {other:?}"),
    }
}
