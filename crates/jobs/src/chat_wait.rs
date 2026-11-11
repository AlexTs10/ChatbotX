//! Waiting for chat jobs from flow steps (`apps/worker/src/integration/utils/message.ts`):
//! `waitForChatJobCompletion` and `sendMessageAndWait`.

use serde_json::json;

use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::JobOptions;
use crate::queue::QueueName;
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
    let Some(job) = job else {
        return;
    };
    let queue = job.queue.unwrap_or(QueueName::Chat);
    let Err(err) = waiter
        .wait_until_finished(queue, &job.job_id, CHAT_JOB_WAIT_TIMEOUT_MS)
        .await
    else {
        return;
    };
    // A stalled chat worker shows here as a burst of these errors (the
    // backlog signal). Swallowed so the flow still advances in order.
    match (
        context.conversation_id.as_deref(),
        context.step_id.as_deref(),
    ) {
        (Some(conversation_id), Some(step_id)) => {
            tracing::error!(conversation_id, step_id, err = %err, "{CHAT_WAIT_FAILED}");
        }
        (Some(conversation_id), None) => {
            tracing::error!(conversation_id, err = %err, "{CHAT_WAIT_FAILED}");
        }
        (None, Some(step_id)) => {
            tracing::error!(step_id, err = %err, "{CHAT_WAIT_FAILED}");
        }
        (None, None) => tracing::error!(err = %err, "{CHAT_WAIT_FAILED}"),
    }
}

const CHAT_WAIT_FAILED: &str = "Chat job did not complete in time or failed; continuing flow";

/// `ChatJobAction.sendChatMessage`.
pub const SEND_CHAT_MESSAGE_ACTION: &str = "sendChatMessage";

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
    conversations.ensure_conversation(conversation_id).await?;
    let data = if is_image_url(text) {
        json!({ "conversationId": conversation_id, "url": text })
    } else {
        json!({ "conversationId": conversation_id, "text": text })
    };
    let added = enqueuer
        .add(
            QueueName::Chat,
            SEND_CHAT_MESSAGE_ACTION,
            json!({ "type": SEND_CHAT_MESSAGE_ACTION, "data": data }),
            JobOptions::default(),
        )
        .await?;
    let target = WaitTarget {
        queue: Some(QueueName::Chat),
        job_id: added.job_id,
        ..WaitTarget::default()
    };
    let context = ChatWaitContext {
        conversation_id: Some(conversation_id.to_owned()),
        step_id: None,
    };
    wait_for_chat_job_completion(waiter, Some(&target), &context).await;
    Ok(())
}

/// `isImageUrl`: an http(s) URL whose path ends in an image extension.
fn is_image_url(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    let path = lower.split(['?', '#']).next().unwrap_or("");
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"]
        .iter()
        .any(|ext| path.ends_with(ext))
}
