//! Port of `packages/worker-config/__tests__/inbound-webhook-request-job.test.ts` (spec 5.1).
//! The bullmq `Queue.add` mock is the `FakeEnqueuer`.

mod support;

use chatbotx_jobs::queues::default_jobs::DefaultJobAction;
use chatbotx_jobs::queues::inbound_webhook::{
    InboundWebhookRequestData, enqueue_process_inbound_webhook_request,
    process_inbound_webhook_request_job_id,
};
use chatbotx_jobs::{QueueName, Retention};
use serde_json::json;
use support::FakeEnqueuer;

fn data() -> InboundWebhookRequestData {
    InboundWebhookRequestData {
        workspace_id: "ws-1".to_owned(),
        inbound_webhook_id: "w-1".to_owned(),
        request_id: "9007199254740993".to_owned(),
    }
}

/// Port of: packages/worker-config/__tests__/inbound-webhook-request-job.test.ts:43 "processInboundWebhookRequestJobId > is deterministic per request and BullMQ-safe (no colon)"
#[test]
fn is_deterministic_per_request_and_bullmq_safe_no_colon() {
    let id = process_inbound_webhook_request_job_id("123");
    assert_eq!(id, "inbound-webhook-request-123");
    assert!(!id.contains(':'));
    assert_ne!(process_inbound_webhook_request_job_id("124"), id);
}

/// Port of: packages/worker-config/__tests__/inbound-webhook-request-job.test.ts:53 "enqueueProcessInboundWebhookRequest > enqueues under the deterministic id so a duplicate enqueue is a no-op"
#[tokio::test]
async fn enqueues_under_the_deterministic_id_so_a_duplicate_enqueue() {
    let fake = FakeEnqueuer::new();
    enqueue_process_inbound_webhook_request(fake.as_ref(), data())
        .await
        .unwrap();

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    let action = DefaultJobAction::ProcessInboundWebhookRequest.as_str();
    assert_eq!(call.queue, QueueName::Default);
    assert_eq!(call.name, action);
    assert_eq!(
        call.data,
        json!({
            "type": action,
            "data": {
                "workspaceId": "ws-1",
                "inboundWebhookId": "w-1",
                "requestId": "9007199254740993",
            },
        })
    );
    assert_eq!(
        call.opts.job_id.as_deref(),
        Some("inbound-webhook-request-9007199254740993")
    );
}

/// Port of: packages/worker-config/__tests__/inbound-webhook-request-job.test.ts:65 "enqueueProcessInboundWebhookRequest > does not keep finished jobs, so a re-dispatch of a still-unfinished request is never blocked by a completed one"
#[tokio::test]
async fn does_not_keep_finished_jobs_so_a_re_dispatch_of_a_still() {
    let fake = FakeEnqueuer::new();
    enqueue_process_inbound_webhook_request(fake.as_ref(), data())
        .await
        .unwrap();

    assert_eq!(
        fake.calls()[0].opts.remove_on_complete,
        Some(Retention::Remove)
    );
}

/// Port of: packages/worker-config/__tests__/inbound-webhook-request-job.test.ts:76 "enqueueProcessInboundWebhookRequest > does not keep failed jobs either, so a re-dispatch can always enqueue the request again"
#[tokio::test]
async fn does_not_keep_failed_jobs_either_so_a_re_dispatch_can_always() {
    let fake = FakeEnqueuer::new();
    enqueue_process_inbound_webhook_request(fake.as_ref(), data())
        .await
        .unwrap();

    assert_eq!(fake.calls()[0].opts.remove_on_fail, Some(Retention::Remove));
}

/// Port of: packages/worker-config/__tests__/inbound-webhook-request-job.test.ts:87 "enqueueProcessInboundWebhookRequest > resolves to nothing, so callers do not depend on the queue implementation"
#[tokio::test]
async fn resolves_to_nothing_so_callers_do_not_depend_on_the_queue() {
    let fake = FakeEnqueuer::new();
    let result: Result<(), _> =
        enqueue_process_inbound_webhook_request(fake.as_ref(), data()).await;
    assert_eq!(result, Ok(()));
}
