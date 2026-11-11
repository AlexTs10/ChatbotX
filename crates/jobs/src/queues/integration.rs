//! The `integration` queue (`queues/integration/index.ts`): job actions,
//! per-action default options, the low-queue routing of bookkeeping jobs,
//! and the flow-continuation priority.

use serde_json::Value;

use crate::enqueue::{Enqueuer, JobsError};
use crate::options::{AddResult, JobOptions};

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
        todo!()
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
    todo!()
}

/// `isLowRoutedIntegrationJob`: message status (unless `continuesFlow`),
/// contact mark-as-read and echo bookkeeping go to the `low` queue.
pub fn is_low_routed_integration_job(job: &IntegrationJob) -> bool {
    todo!()
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
    todo!()
}

/// `flowContinuationJobOptions(metadata)`: the scheduled-send priority for
/// broadcast / sequence runs (`metadata.type`), else no options.
pub fn flow_continuation_job_options(metadata: Option<&Value>) -> JobOptions {
    todo!()
}
