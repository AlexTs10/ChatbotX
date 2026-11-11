//! `advance-enrollment.ts`: move an enrollment past the step just sent.

use crate::sequence::repo::{SequenceCtx, SequenceError};

/// The step that was just sent (`{ id, order }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentStep {
    pub id: String,
    pub order: i64,
}

/// `advanceEnrollment` input (the scheduler is `ctx.schedule`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceParams {
    pub enrollment_id: String,
    pub workspace_id: String,
    pub sequence_id: String,
    pub contact_id: String,
    pub current_step: CurrentStep,
    pub sent_at_ms: i64,
}

/// `advanceEnrollment(params)`:
/// - missing enrollment: `Enrollment <id> not found`;
/// - status not `active`, or `lastStepId == current_step.id`: no change;
/// - no next active step: one UPDATE (no transaction) to `completed`,
///   `completedAt = sentAt`, `nextStepId/nextRunAt = NULL`,
///   `currentStep = order + 1`;
/// - else one transaction: UPDATE (`lastStepId`, `nextStepId`, `nextRunAt`,
///   `currentStep`) and one dispatch per contact inbox (`ctx.dispatches`,
///   run time from `ctx.next_run`); after the commit each dispatch is added
///   to `ctx.schedule`.
pub async fn advance_enrollment(
    ctx: &SequenceCtx,
    params: AdvanceParams,
) -> Result<(), SequenceError> {
    todo!()
}
