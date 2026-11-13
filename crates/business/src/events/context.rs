//! `webhook/context.ts`: the webhook execution context (an
//! `AsyncLocalStorage` in the original, a tokio task-local here). Channel
//! webhook handlers run inside it; trigger jobs then carry
//! `channelOriginated: true` and webhook events are allowed.

use std::future::Future;

/// `ExecutionContext`. `source: Some("webhook")` = channel-originated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebhookExecutionContext {
    pub source: Option<String>,
}

impl WebhookExecutionContext {
    /// `{ source: "webhook" }`.
    pub fn webhook() -> WebhookExecutionContext {
        WebhookExecutionContext {
            source: Some("webhook".to_string()),
        }
    }
}

tokio::task_local! {
    static WEBHOOK_CONTEXT: WebhookExecutionContext;
}

/// `runWithWebhookExecutionContext` for a synchronous callback.
pub fn run_with_webhook_execution_context<T>(
    context: WebhookExecutionContext,
    callback: impl FnOnce() -> T,
) -> T {
    let _ = (context, callback);
    todo!("P3.6-X1")
}

/// `runWithWebhookExecutionContext` for a future.
pub async fn with_webhook_execution_context<F: Future>(
    context: WebhookExecutionContext,
    future: F,
) -> F::Output {
    WEBHOOK_CONTEXT.scope(context, future).await
}

/// `isWebhookContext`.
pub fn is_webhook_context() -> bool {
    todo!("P3.6-X1")
}

/// `webhookChannelOrigin`: `Some("channel")` inside a webhook context.
pub fn webhook_channel_origin() -> Option<&'static str> {
    is_webhook_context().then_some("channel")
}
