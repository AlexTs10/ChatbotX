//! Port of `packages/worker-config/__tests__/job-wait.test.ts` (spec 5.1). `waitUntilFinished` and
//! `getJob` mocks are scripted fakes; the `Date.now` spy is a `TestClock`.

mod support;
#[path = "support/worker_config.rs"]
mod worker_config;

use std::sync::Arc;

use chatbotx_jobs::queues::heavy::{HeavyJobAction, heavy_completion_wait_timeout_ms};
use chatbotx_jobs::wait::wait_for_completion_with_retries;
use chatbotx_jobs::{JobsError, QueueName, WaitError, WaitTarget};
use serde_json::json;
use support::TestClock;
use worker_config::{FakeInspector, FakeWaiter, attempts_record};

fn job(attempts_made: u32) -> WaitTarget {
    WaitTarget {
        queue: Some(QueueName::Heavy),
        job_id: "job-1".to_owned(),
        attempts_made,
        attempts: Some(2),
    }
}

/// Port of: packages/worker-config/__tests__/job-wait.test.ts:9 "waitForJobCompletionWithRetries > waits through failed attempts until the job completes"
#[tokio::test]
async fn waits_through_failed_attempts_until_the_job_completes() {
    let waiter = FakeWaiter::new(vec![
        Err(WaitError::Failed("temporary provider failure".to_owned())),
        Ok(json!({ "status": "success" })),
    ]);
    let inspector = FakeInspector::new(Ok(Some(attempts_record(1, 2))));
    let clock = TestClock::new(0);

    let result =
        wait_for_completion_with_retries(&waiter, &inspector, clock.as_ref(), &job(0), 1000).await;

    assert_eq!(result, Ok(json!({ "status": "success" })));
    assert_eq!(waiter.calls().len(), 2);
    assert_eq!(inspector.get_job_calls(), 1);
}

/// Port of: packages/worker-config/__tests__/job-wait.test.ts:32 "waitForJobCompletionWithRetries > throws after the final attempt fails"
#[tokio::test]
async fn throws_after_the_final_attempt_fails() {
    let error = WaitError::Failed("permanent provider failure".to_owned());
    let waiter = FakeWaiter::new(vec![Err(error.clone())]);
    let inspector = FakeInspector::new(Ok(Some(attempts_record(2, 2))));
    let clock = TestClock::new(0);

    let result =
        wait_for_completion_with_retries(&waiter, &inspector, clock.as_ref(), &job(1), 1000).await;

    assert_eq!(result, Err(error));
    assert_eq!(waiter.calls().len(), 1);
}

/// Port of: packages/worker-config/__tests__/job-wait.test.ts:52 "waitForJobCompletionWithRetries > keeps enough wait budget for a slow attempt and its retry backoff"
#[tokio::test]
async fn keeps_enough_wait_budget_for_a_slow_attempt_and_its_retry() {
    let clock = TestClock::new(0);
    let hook_clock = Arc::clone(&clock);
    let waiter = FakeWaiter::new(vec![
        Err(WaitError::Failed("temporary provider failure".to_owned())),
        Ok(json!({ "status": "success" })),
    ])
    .with_hook(move |call| {
        if call == 0 {
            hook_clock.set(120_000);
        }
    });
    let inspector = FakeInspector::new(Ok(Some(attempts_record(1, 2))));
    let timeout_ms = heavy_completion_wait_timeout_ms(HeavyJobAction::AiGenerateImage, 120_000);

    let result =
        wait_for_completion_with_retries(&waiter, &inspector, clock.as_ref(), &job(0), timeout_ms)
            .await;

    assert_eq!(result, Ok(json!({ "status": "success" })));
    assert_eq!(timeout_ms, 330_000);
    let timeouts: Vec<u64> = waiter.calls().iter().map(|c| c.timeout_ms).collect();
    assert_eq!(timeouts, vec![330_000, 210_000]);
}

/// Port of: packages/worker-config/__tests__/job-wait.test.ts:90 "waitForJobCompletionWithRetries > throws a retryable state error when it cannot inspect the queued job"
#[tokio::test]
async fn throws_a_retryable_state_error_when_it_cannot_inspect_the() {
    let waiter = FakeWaiter::new(vec![Err(WaitError::Failed(
        "temporary provider failure".to_owned(),
    ))]);
    let inspector = FakeInspector::new(Err(JobsError::Store("Redis unavailable".to_owned())));
    let clock = TestClock::new(0);

    let result =
        wait_for_completion_with_retries(&waiter, &inspector, clock.as_ref(), &job(0), 1000).await;

    assert!(
        matches!(result, Err(WaitError::CompletionStateUnknown(_))),
        "{result:?}"
    );
}
