//! The `integration` queue (`queues/integration/index.ts`): job actions,
//! per-action default options, the low-queue routing of bookkeeping jobs,
//! and the flow-continuation priority.

use serde_json::Value;

use crate::enqueue::{Enqueuer, JobsError};
use crate::options::{AddResult, Backoff, JobOptions};
use crate::queue::QueueName;

/// `BROADCAST_SEND_PRIORITY`.
pub const BROADCAST_SEND_PRIORITY: u32 = 5;
/// `SEQUENCE_SEND_PRIORITY`.
pub const SEQUENCE_SEND_PRIORITY: u32 = 5;

/// `IntegrationJobAction` (the members the ported tests use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegrationJobAction {
    EvaluateTemplateSent,
    EvaluateConversionTrigger,
    SyncRetargetAudience,
    SendConversionEvent,
    AgentMarkAsRead,
    MessageStatus,
    ContactMarkAsRead,
    IncomingMessageEcho,
    IncomingMessage,
}

impl IntegrationJobAction {
    /// `"evaluateTemplateSent"`, `"messageStatus"`, ...
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationJobAction::EvaluateTemplateSent => "evaluateTemplateSent",
            IntegrationJobAction::EvaluateConversionTrigger => "evaluateConversionTrigger",
            IntegrationJobAction::SyncRetargetAudience => "syncRetargetAudience",
            IntegrationJobAction::SendConversionEvent => "sendConversionEvent",
            IntegrationJobAction::AgentMarkAsRead => "agentMarkAsRead",
            IntegrationJobAction::MessageStatus => "messageStatus",
            IntegrationJobAction::ContactMarkAsRead => "contactMarkAsRead",
            IntegrationJobAction::IncomingMessageEcho => "incomingMessageEcho",
            IntegrationJobAction::IncomingMessage => "incomingMessage",
        }
    }
}

/// One integration job: `{ type, data }`.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationJob {
    pub action: IntegrationJobAction,
    pub data: Value,
}

/// `jobOptionsByAction[action]` (empty for actions without an entry).
pub fn job_options_by_action(action: IntegrationJobAction) -> JobOptions {
    match action {
        IntegrationJobAction::EvaluateTemplateSent
        | IntegrationJobAction::EvaluateConversionTrigger
        | IntegrationJobAction::SyncRetargetAudience => ads_conversion_retry_options(),
        IntegrationJobAction::SendConversionEvent => JobOptions {
            priority: Some(CAPI_EVENT_PRIORITY),
            ..ads_conversion_retry_options()
        },
        IntegrationJobAction::AgentMarkAsRead
        | IntegrationJobAction::MessageStatus
        | IntegrationJobAction::ContactMarkAsRead
        | IntegrationJobAction::IncomingMessageEcho
        | IntegrationJobAction::IncomingMessage => JobOptions::default(),
    }
}

/// `CAPI_EVENT_PRIORITY`.
pub const CAPI_EVENT_PRIORITY: u32 = 1;

/// `adsConversionRetryOptions`: 5 attempts, exponential 30 000 ms.
pub fn ads_conversion_retry_options() -> JobOptions {
    JobOptions {
        attempts: Some(5),
        backoff: Some(Backoff::Exponential(30_000)),
        ..JobOptions::default()
    }
}

/// `isLowRoutedIntegrationJob`: message status (unless `continuesFlow`),
/// contact mark-as-read and echo bookkeeping go to the `low` queue.
pub fn is_low_routed_integration_job(job: &IntegrationJob) -> bool {
    match job.action {
        // A status that may continue a flow stays with live traffic.
        IntegrationJobAction::MessageStatus => {
            job.data.get("continuesFlow").and_then(Value::as_bool) != Some(true)
        }
        IntegrationJobAction::ContactMarkAsRead | IntegrationJobAction::IncomingMessageEcho => true,
        _ => false,
    }
}

/// `enqueueIntegrationJob(job, opts)`: adds `job.action` with
/// `{ type, data }` on `integration` (or `low`, see
/// [`is_low_routed_integration_job`]) with
/// `JobOptions::merge(job_options_by_action(action), opts)`.
pub async fn enqueue_integration_job(
    enqueuer: &dyn Enqueuer,
    job: IntegrationJob,
    opts: Option<JobOptions>,
) -> Result<AddResult, JobsError> {
    let options = JobOptions::merge(
        &job_options_by_action(job.action),
        &opts.unwrap_or_default(),
    );
    let queue = if is_low_routed_integration_job(&job) {
        QueueName::Low
    } else {
        QueueName::Integration
    };
    let name = job.action.as_str();
    let payload = serde_json::json!({ "type": name, "data": job.data });
    enqueuer.add(queue, name, payload, options).await
}

/// `flowContinuationJobOptions(metadata)`: the scheduled-send priority for
/// broadcast / sequence runs (`metadata.type`), else no options.
pub fn flow_continuation_job_options(metadata: Option<&Value>) -> JobOptions {
    let priority = match metadata.and_then(|m| m.get("type")).and_then(Value::as_str) {
        Some("broadcast") => Some(BROADCAST_SEND_PRIORITY),
        Some("sequenceSchedule") => Some(SEQUENCE_SEND_PRIORITY),
        _ => None,
    };
    JobOptions {
        priority,
        ..JobOptions::default()
    }
}
