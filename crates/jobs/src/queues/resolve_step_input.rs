//! `buildResolveStepInputJob` (`packages/worker-config/src/queues/integration`):
//! the `resolveStepInput` integration job of a step-input tap.

use serde::{Deserialize, Serialize};

use crate::options::NewJob;

/// `IntegrationJobAction.resolveStepInput`.
pub const RESOLVE_STEP_INPUT_ACTION: &str = "resolveStepInput";

/// Why the step input resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveStepInputReason {
    UserSkipped,
}

/// The payload of a `resolveStepInput` job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveStepInput {
    pub conversation_id: String,
    pub contact_inbox_id: String,
    /// Snowflake as text (> 2^53).
    pub challenge_id: String,
    pub reason: ResolveStepInputReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// `Date` at the queue boundary: ISO-8601 with ms and `Z`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_created_at: Option<String>,
}

/// `buildResolveStepInputJob(input)`: name `resolveStepInput`, data
/// `{ type, data: input }`, and simple deduplication
/// `step-input-<conversationId>-<challengeId>` (no fixed job id).
pub fn build_resolve_step_input_job(input: &ResolveStepInput) -> NewJob {
    todo!()
}
