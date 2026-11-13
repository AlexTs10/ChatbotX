//! Port of `packages/events/__tests__/webhook-context.test.ts` (P3.6-X1):
//! the webhook execution context is scoped to its callback.

use chatbotx_business::events::context::{
    WebhookExecutionContext, is_webhook_context, run_with_webhook_execution_context,
    with_webhook_execution_context,
};

// webhook execution context > scopes webhook context to the callback
#[tokio::test]
async fn scopes_webhook_context_to_the_callback() {
    // The `vi.waitFor` block of the original (the lazy module load) is the
    // async form here.
    assert!(
        with_webhook_execution_context(WebhookExecutionContext::webhook(), async {
            is_webhook_context()
        })
        .await
    );
    assert!(run_with_webhook_execution_context(
        WebhookExecutionContext::webhook(),
        is_webhook_context
    ));
    assert!(!run_with_webhook_execution_context(
        WebhookExecutionContext::default(),
        is_webhook_context
    ));
    assert!(!is_webhook_context());
}
