//! Waiting for chat jobs from flow steps (`apps/worker/src/integration/utils/message.ts`):
//! `waitForChatJobCompletion` and `sendMessageAndWait`.

use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::wait::{JobWaiter, WaitTarget};

/// `CHAT_JOB_WAIT_TIMEOUT_MS` default (kept below the 600 000 ms lease).
pub const CHAT_JOB_WAIT_TIMEOUT_MS: u64 = 120_000;

/// Log context of a wait (`{ conversationId, stepId }`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatWaitContext {
    pub conversation_id: Option<String>,
    pub step_id: Option<String>,
}

/// `waitForChatJobCompletion(job, context)`: waits on the `chat` queue job
/// with a bounded timeout ([`CHAT_JOB_WAIT_TIMEOUT_MS`]). `None` (not a job)
/// returns at once without a wait. A failed or timed-out wait is logged at
/// error level ("Chat job did not complete in time or failed; continuing
/// flow", fields = the context fields set + `err`) and never returned.
pub async fn wait_for_chat_job_completion(
    waiter: &dyn JobWaiter,
    job: Option<&WaitTarget>,
    context: &ChatWaitContext,
) {
    todo!()
}

/// `findOrFail(conversationModel, ...)`: the conversation must exist.
pub trait ConversationLookup: Send + Sync {
    fn ensure_conversation<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<(), JobsError>>;
}

/// `sendMessageAndWait(conversationId, text)`: enqueues `sendChatMessage`
/// on `chat` and waits for it like [`wait_for_chat_job_completion`] (context
/// `{ conversationId }`). A failed wait never fails the caller (no job retry,
/// no double send).
pub async fn send_message_and_wait(
    enqueuer: &dyn Enqueuer,
    waiter: &dyn JobWaiter,
    conversations: &dyn ConversationLookup,
    conversation_id: &str,
    text: &str,
) -> Result<(), JobsError> {
    todo!()
}
