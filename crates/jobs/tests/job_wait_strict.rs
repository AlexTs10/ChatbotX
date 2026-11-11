//! Port of `packages/worker-config/__tests__/job-wait-strict.test.ts` (spec 5.1). The `job.waitUntilFinished`
//! mock is a scripted `JobWaiter` fake.

#[path = "support/worker_config.rs"]
mod worker_config;

use chatbotx_jobs::wait::wait_for_job_result;
use chatbotx_jobs::{QueueName, WaitError, WaitTarget};
use serde_json::json;
use worker_config::FakeWaiter;

fn target() -> WaitTarget {
    WaitTarget {
        queue: Some(QueueName::Integration),
        job_id: "job-1".to_owned(),
        attempts_made: 0,
        attempts: None,
    }
}

/// Port of: packages/worker-config/__tests__/job-wait-strict.test.ts:7 "waitForJobResult (strict variant) > resolves with the job's result"
#[tokio::test]
async fn resolves_with_the_job_s_result() {
    let waiter = FakeWaiter::new(vec![Ok(json!("sofia status gateway wa-1\nNOREG"))]);

    let result = wait_for_job_result(&waiter, &target(), 15_000).await;

    assert_eq!(result, Ok(json!("sofia status gateway wa-1\nNOREG")));
    let calls = waiter.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].timeout_ms, 15_000);
}

/// Port of: packages/worker-config/__tests__/job-wait-strict.test.ts:17 "waitForJobResult (strict variant) > rejects when the wait times out (unlike the best-effort helper, never swallowed)"
#[tokio::test]
async fn rejects_when_the_wait_times_out_unlike_the_best_effort() {
    let waiter = FakeWaiter::new(vec![Err(WaitError::Timeout)]);

    let result = wait_for_job_result(&waiter, &target(), 15_000).await;

    assert_eq!(result, Err(WaitError::Timeout));
}

/// Port of: packages/worker-config/__tests__/job-wait-strict.test.ts:29 "waitForJobResult (strict variant) > rejects when the underlying job fails"
#[tokio::test]
async fn rejects_when_the_underlying_job_fails() {
    let waiter = FakeWaiter::new(vec![Err(WaitError::Failed(
        "-ERR gateway wa-1 not found".to_owned(),
    ))]);

    let result = wait_for_job_result(&waiter, &target(), 15_000).await;

    assert_eq!(
        result,
        Err(WaitError::Failed("-ERR gateway wa-1 not found".to_owned()))
    );
}
