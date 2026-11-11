//! Port of `packages/worker-config/__tests__/enqueue-integration-job.test.ts` (spec 5.1). The two bullmq
//! `Queue.add` mocks (integration / low) are one `FakeEnqueuer` that
//! records the queue of every add.

mod support;

use chatbotx_jobs::queues::integration::{
    BROADCAST_SEND_PRIORITY, IntegrationJob, IntegrationJobAction, SEQUENCE_SEND_PRIORITY,
    enqueue_integration_job, flow_continuation_job_options,
};
use chatbotx_jobs::{Backoff, JobOptions, QueueName, Retention};
use serde_json::{Value, json};
use support::FakeEnqueuer;

fn job(action: IntegrationJobAction, data: Value) -> IntegrationJob {
    IntegrationJob { action, data }
}

/// `{ attempts: 5, backoff: { type: "exponential", delay: 30_000 } }`
fn ads_retry() -> JobOptions {
    JobOptions {
        attempts: Some(5),
        backoff: Some(Backoff::Exponential(30_000)),
        ..JobOptions::default()
    }
}

/// `statusJob(continuesFlow)`
fn status_job(continues_flow: Option<bool>) -> IntegrationJob {
    let mut data = json!({
        "integrationType": "whatsapp",
        "integrationIdentifier": "phone-1",
        "payload": { "messageId": "wamid.1", "status": "delivered", "timestamp": "1" },
    });
    if let Some(flag) = continues_flow {
        data["continuesFlow"] = json!(flag);
    }
    job(IntegrationJobAction::MessageStatus, data)
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:47 "jobOptionsByAction > gives the 3 non-CAPI ads actions attempts:5 / 30s exponential backoff, no priority"
#[tokio::test]
async fn gives_the_3_non_capi_ads_actions_attempts_5_30s_exponential() {
    for action in [
        IntegrationJobAction::EvaluateTemplateSent,
        IntegrationJobAction::EvaluateConversionTrigger,
        IntegrationJobAction::SyncRetargetAudience,
    ] {
        let fake = FakeEnqueuer::new();
        enqueue_integration_job(fake.as_ref(), job(action, json!({})), None)
            .await
            .unwrap();

        let calls = fake.calls_for(QueueName::Integration);
        assert_eq!(calls[0].opts, ads_retry(), "{action:?}");
    }
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:64 "jobOptionsByAction > gives sendConversionEvent the ads retry plus an explicit priority"
#[tokio::test]
async fn gives_sendconversionevent_the_ads_retry_plus_an_explicit() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::SendConversionEvent,
            json!({ "adsConversionEventId": "event-1", "workspaceId": "ws-1" }),
        ),
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        fake.calls_for(QueueName::Integration)[0].opts,
        JobOptions {
            priority: Some(1),
            ..ads_retry()
        }
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:78 "jobOptionsByAction > leaves an existing integration action (no ads entry) with no extra opts"
#[tokio::test]
async fn leaves_an_existing_integration_action_no_ads_entry_with_no() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::AgentMarkAsRead,
            json!({ "conversationId": "conv-1" }),
        ),
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        fake.calls_for(QueueName::Integration)[0].opts,
        JobOptions::default()
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:90 "enqueueIntegrationJob merge order > applies the action default when the caller passes no opts"
#[tokio::test]
async fn applies_the_action_default_when_the_caller_passes_no_opts() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::EvaluateTemplateSent,
            json!({
                "workspaceId": "ws-1",
                "integrationWhatsappId": "iw-1",
                "contactInboxId": "ci-1",
                "templateId": "template-1",
            }),
        ),
        None,
    )
    .await
    .unwrap();

    let calls = fake.calls_for(QueueName::Integration);
    assert_eq!(calls.len(), 1);
    let name = IntegrationJobAction::EvaluateTemplateSent.as_str();
    assert_eq!(calls[0].name, name);
    assert_eq!(calls[0].data["type"], json!(name));
    assert_eq!(calls[0].opts, ads_retry());
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:110 "enqueueIntegrationJob merge order > producer opts (jobId, retention) survive and override overlapping action defaults"
#[tokio::test]
async fn producer_opts_jobid_retention_survive_and_override() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::SendConversionEvent,
            json!({ "adsConversionEventId": "event-1", "workspaceId": "ws-1" }),
        ),
        Some(JobOptions {
            job_id: Some("ads-conversion-send-event-1".to_owned()),
            attempts: Some(99),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        fake.calls_for(QueueName::Integration)[0].opts,
        JobOptions {
            // Producer wins on the overlapping key.
            attempts: Some(99),
            // Non-overlapping action default survives.
            backoff: Some(Backoff::Exponential(30_000)),
            priority: Some(1),
            // Producer-only opts pass through untouched.
            job_id: Some("ads-conversion-send-event-1".to_owned()),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        }
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:138 "enqueueIntegrationJob merge order > an existing integration producer's jobId is unaffected (no action defaults to merge)"
#[tokio::test]
async fn an_existing_integration_producer_s_jobid_is_unaffected_no() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::AgentMarkAsRead,
            json!({ "conversationId": "conv-1" }),
        ),
        Some(JobOptions {
            job_id: Some("agent-mark-as-read-1".to_owned()),
            ..JobOptions::default()
        }),
    )
    .await
    .unwrap();

    let calls = fake.calls_for(QueueName::Integration);
    let name = IntegrationJobAction::AgentMarkAsRead.as_str();
    assert_eq!(calls[0].name, name);
    assert_eq!(calls[0].data["type"], json!(name));
    assert_eq!(
        calls[0].opts,
        JobOptions {
            job_id: Some("agent-mark-as-read-1".to_owned()),
            ..JobOptions::default()
        }
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:167 "low-queue routing > routes a plain message status to the low queue"
#[tokio::test]
async fn routes_a_plain_message_status_to_the_low_queue() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        status_job(None),
        Some(JobOptions {
            job_id: Some("wa-status-1".to_owned()),
            ..JobOptions::default()
        }),
    )
    .await
    .unwrap();

    assert_eq!(fake.count(QueueName::Integration), 0);
    let low = fake.calls_for(QueueName::Low);
    assert_eq!(low.len(), 1);
    let name = IntegrationJobAction::MessageStatus.as_str();
    assert_eq!(low[0].name, name);
    assert_eq!(low[0].data["type"], json!(name));
    assert_eq!(
        low[0].opts,
        JobOptions {
            job_id: Some("wa-status-1".to_owned()),
            ..JobOptions::default()
        }
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:178 "low-queue routing > routes a status with continuesFlow:false to the low queue"
#[tokio::test]
async fn routes_a_status_with_continuesflow_false_to_the_low_queue() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(fake.as_ref(), status_job(Some(false)), None)
        .await
        .unwrap();

    assert_eq!(fake.count(QueueName::Low), 1);
    assert_eq!(fake.count(QueueName::Integration), 0);
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:185 "low-queue routing > keeps a status that continues a flow on the integration queue"
#[tokio::test]
async fn keeps_a_status_that_continues_a_flow_on_the_integration() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(fake.as_ref(), status_job(Some(true)), None)
        .await
        .unwrap();

    assert_eq!(fake.count(QueueName::Integration), 1);
    assert_eq!(fake.count(QueueName::Low), 0);
}

async fn assert_routed_to_low(action: IntegrationJobAction, data: Value) {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(fake.as_ref(), job(action, data), None)
        .await
        .unwrap();

    assert_eq!(fake.count(QueueName::Low), 1);
    assert_eq!(fake.count(QueueName::Integration), 0);
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:192 "low-queue routing > routes contactMarkAsRead to the low queue"
#[tokio::test]
async fn routes_to_the_low_queue_contact_mark_as_read() {
    assert_routed_to_low(
        IntegrationJobAction::ContactMarkAsRead,
        json!({
            "integrationType": "messenger",
            "integrationIdentifier": "page-1",
            "sourceConversationId": "psid-1",
            "payload": {},
        }),
    )
    .await;
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:192 "low-queue routing > routes incomingMessageEcho to the low queue"
#[tokio::test]
async fn routes_to_the_low_queue_incoming_message_echo() {
    assert_routed_to_low(
        IntegrationJobAction::IncomingMessageEcho,
        json!({
            "integrationType": "zalo",
            "integrationIdentifier": "oa-1",
            "payload": {},
        }),
    )
    .await;
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:217 "low-queue routing > leaves customer messages on the integration queue"
#[tokio::test]
async fn leaves_customer_messages_on_the_integration_queue() {
    let fake = FakeEnqueuer::new();
    enqueue_integration_job(
        fake.as_ref(),
        job(
            IntegrationJobAction::IncomingMessage,
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": "page-1",
                "payload": {},
            }),
        ),
        None,
    )
    .await
    .unwrap();

    assert_eq!(fake.count(QueueName::Integration), 1);
    assert_eq!(fake.count(QueueName::Low), 0);
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:233 "flowContinuationJobOptions > a continuation of a live flow gets no priority, so it stays in FIFO order with customer messages"
#[test]
fn a_continuation_of_a_live_flow_gets_no_priority_so_it_stays() {
    assert_eq!(flow_continuation_job_options(None), JobOptions::default());
    assert_eq!(
        flow_continuation_job_options(Some(&json!({ "type": "updateStatus" }))),
        JobOptions::default()
    );
}

/// Port of: packages/worker-config/__tests__/enqueue-integration-job.test.ts:242 "flowContinuationJobOptions > broadcast and sequence runs keep the scheduled-send priority on every step"
#[test]
fn broadcast_and_sequence_runs_keep_the_scheduled_send_priority() {
    assert_eq!(
        flow_continuation_job_options(Some(&json!({ "type": "broadcast" }))),
        JobOptions {
            priority: Some(BROADCAST_SEND_PRIORITY),
            ..JobOptions::default()
        }
    );
    assert_eq!(
        flow_continuation_job_options(Some(&json!({ "type": "sequenceSchedule" }))),
        JobOptions {
            priority: Some(SEQUENCE_SEND_PRIORITY),
            ..JobOptions::default()
        }
    );
}
