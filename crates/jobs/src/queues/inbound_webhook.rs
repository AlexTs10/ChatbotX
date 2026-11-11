//! Inbound webhook request outbox job (`queues/default`:
//! `enqueueProcessInboundWebhookRequest`).

use serde::{Deserialize, Serialize};

use crate::enqueue::{Enqueuer, JobsError};

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
    todo!()
}

/// Adds `processInboundWebhookRequest` with `{ type, data }` on the
/// `default` queue under the deterministic id, `removeOnComplete` and
/// `removeOnFail` = `Remove`.
pub async fn enqueue_process_inbound_webhook_request(
    enqueuer: &dyn Enqueuer,
    data: InboundWebhookRequestData,
) -> Result<(), JobsError> {
    todo!()
}
