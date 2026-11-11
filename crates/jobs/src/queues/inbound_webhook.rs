//! Inbound webhook request outbox job (`queues/default`:
//! `enqueueProcessInboundWebhookRequest`).

use serde::{Deserialize, Serialize};

use crate::enqueue::{Enqueuer, JobsError};
use crate::options::{JobOptions, Retention};
use crate::queue::QueueName;
use crate::queues::default_jobs::DefaultJobAction;

/// The job payload `data` (ids stay strings: they can exceed 2^53).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundWebhookRequestData {
    pub workspace_id: String,
    pub inbound_webhook_id: String,
    pub request_id: String,
}

/// `inbound-webhook-request-<requestId>`.
pub fn process_inbound_webhook_request_job_id(request_id: &str) -> String {
    format!("inbound-webhook-request-{request_id}")
}

/// Adds `processInboundWebhookRequest` with `{ type, data }` on the
/// `default` queue under the deterministic id, `removeOnComplete` and
/// `removeOnFail` = `Remove`.
pub async fn enqueue_process_inbound_webhook_request(
    enqueuer: &dyn Enqueuer,
    data: InboundWebhookRequestData,
) -> Result<(), JobsError> {
    let action = DefaultJobAction::ProcessInboundWebhookRequest.as_str();
    let job_id = process_inbound_webhook_request_job_id(&data.request_id);
    let payload = serde_json::json!({
        "type": action,
        "data": serde_json::to_value(&data).map_err(|e| JobsError::Other(e.to_string()))?,
    });
    enqueuer
        .add(
            QueueName::Default,
            action,
            payload,
            JobOptions {
                job_id: Some(job_id),
                remove_on_complete: Some(Retention::Remove),
                // The request row is the outbox: a kept failed job would block
                // the re-dispatch of the unfinished request.
                remove_on_fail: Some(Retention::Remove),
                ..JobOptions::default()
            },
        )
        .await
        .map(|_| ())
}
