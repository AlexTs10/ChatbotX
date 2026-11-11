//! Port of `apps/worker/__tests__/wait-for-chat-job-completion.test.ts`
//! (`waitForChatJobCompletion`, `sendMessageAndWait`). `job.waitUntilFinished`
//! is a recording `JobWaiter`; `chatQueue.add` is `FakeEnqueuer`.

mod support;

#[path = "support/deferral.rs"]
mod deferral;

use std::collections::BTreeSet;

use chatbotx_jobs::chat_wait::{
    ChatWaitContext, ConversationLookup, send_message_and_wait, wait_for_chat_job_completion,
};
use chatbotx_jobs::{BoxFuture, JobsError, QueueName, WaitError, WaitTarget};
use deferral::FakeWaiter;
use serde_json::json;
use support::{FakeEnqueuer, RecordingLog};
use tracing::Level;

const WAIT_FAILED: &str = "Chat job did not complete in time or failed; continuing flow";

/// `findOrFail` resolves `{ id: "conv-1" }`.
struct FoundConversation;

impl ConversationLookup for FoundConversation {
    fn ensure_conversation<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<(), JobsError>> {
        Box::pin(async { Ok(()) })
    }
}

fn chat_job() -> WaitTarget {
    WaitTarget {
        queue: Some(QueueName::Chat),
        job_id: "chat-job-1".to_owned(),
        ..WaitTarget::default()
    }
}

fn field_names(entry: &support::LogEntry) -> BTreeSet<String> {
    entry.fields.keys().cloned().collect()
}

/// Port of: apps/worker/__tests__/wait-for-chat-job-completion.test.ts:39 "waitForChatJobCompletion > awaits job.waitUntilFinished with a bounded timeout"
#[tokio::test]
async fn awaits_job_waituntilfinished_with_a_bounded_timeout() {
    let waiter = FakeWaiter::resolving(json!("done"));
    let job = chat_job();

    wait_for_chat_job_completion(&waiter, Some(&job), &ChatWaitContext::default()).await;

    let calls = waiter.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].timeout_ms > 0);
    assert_eq!(calls[0].queue, QueueName::Chat);
}

/// Port of: apps/worker/__tests__/wait-for-chat-job-completion.test.ts:52 "waitForChatJobCompletion > is a no-op for a non-job value"
#[tokio::test]
async fn is_a_no_op_for_a_non_job_value() {
    let waiter = FakeWaiter::resolving(json!("done"));

    wait_for_chat_job_completion(&waiter, None, &ChatWaitContext::default()).await;

    assert!(waiter.calls().is_empty());
}

/// Port of: apps/worker/__tests__/wait-for-chat-job-completion.test.ts:60 "waitForChatJobCompletion > swallows and logs a failed or timed-out wait"
#[tokio::test]
async fn swallows_and_logs_a_failed_or_timed_out_wait() {
    let log = RecordingLog::install();
    let waiter = FakeWaiter::rejecting(WaitError::Failed("send failed".to_owned()));
    let job = chat_job();
    let context = ChatWaitContext {
        conversation_id: Some("conv-1".to_owned()),
        step_id: Some("step-1".to_owned()),
    };

    wait_for_chat_job_completion(&waiter, Some(&job), &context).await;

    let errors = log.at(Level::ERROR);
    assert_eq!(errors.len(), 1);
    let entry = &errors[0];
    assert_eq!(entry.message, WAIT_FAILED);
    assert_eq!(
        field_names(entry),
        ["conversation_id", "err", "step_id"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(entry.field("conversation_id"), Some("conv-1"));
    assert_eq!(entry.field("step_id"), Some("step-1"));
    assert!(
        entry
            .field("err")
            .is_some_and(|e| e.contains("send failed"))
    );
}

/// Port of: apps/worker/__tests__/wait-for-chat-job-completion.test.ts:83 "sendMessageAndWait > does not throw when the chat job wait rejects (avoids job retry / double send)"
#[tokio::test]
async fn does_not_throw_when_the_chat_job_wait_rejects_avoids_job() {
    let log = RecordingLog::install();
    let enqueuer = FakeEnqueuer::new();
    let waiter = FakeWaiter::rejecting(WaitError::Failed("wait timed out".to_owned()));

    let result =
        send_message_and_wait(&*enqueuer, &waiter, &FoundConversation, "conv-1", "hello").await;

    assert_eq!(result, Ok(()));
    let entry = log.find(WAIT_FAILED).expect("error log");
    assert_eq!(entry.level, Level::ERROR);
    assert_eq!(
        field_names(&entry),
        ["conversation_id", "err"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(entry.field("conversation_id"), Some("conv-1"));
    assert!(
        entry
            .field("err")
            .is_some_and(|e| e.contains("wait timed out"))
    );
}

/// Port of: apps/worker/__tests__/wait-for-chat-job-completion.test.ts:97 "sendMessageAndWait > awaits the enqueued job with a bounded timeout"
#[tokio::test]
async fn awaits_the_enqueued_job_with_a_bounded_timeout() {
    let enqueuer = FakeEnqueuer::new();
    let waiter = FakeWaiter::resolving(json!("done"));

    send_message_and_wait(&*enqueuer, &waiter, &FoundConversation, "conv-1", "hello")
        .await
        .expect("sendMessageAndWait");

    let calls = waiter.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].timeout_ms > 0);
    assert_eq!(enqueuer.count(QueueName::Chat), 1);
}
