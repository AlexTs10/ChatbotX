//! `integration/handlers/coupon.ts`: `setUpCoupon` / `markCouponUsed`. A
//! refusal (`{ ok: false, reason }`) is logged and the flow goes on; only a
//! thrown error routes to the error state.

use chatbotx_business::coupon;
use chatbotx_jobs::JobError;

use super::{StepCtx, step_error};
use crate::flow::StepStatus;

/// The refusal codes of `coupon::issue` / `coupon::mark_used` (the
/// original's `{ ok: false }` results).
const REFUSALS: &[&str] = &["couponIssueUnavailable", "couponNotIssued"];

pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    let issue = match step_type {
        "setUpCoupon" => true,
        "markCouponUsed" => false,
        _ => return Ok(None),
    };
    let db = &ctx.run.deps.db;
    let (ws, contact) = (ctx.workspace_id(), ctx.contact_id());
    // A non-numeric topic id matches no topic: the original's
    // `topicUnavailable` refusal.
    let topic = ctx.str("topicId").and_then(|t| t.parse::<i64>().ok());
    let result = match topic {
        None => Err(None),
        Some(topic) if issue => coupon::issue(db, ws, topic, contact)
            .await
            .map(|_| ())
            .map_err(Some),
        Some(topic) => coupon::mark_used(db, ws, topic, contact)
            .await
            .map(|_| ())
            .map_err(Some),
    };
    let what = if issue { "issue" } else { "mark-used" };
    Ok(Some(match result {
        Ok(()) => StepStatus::Success,
        Err(error)
            if error
                .as_ref()
                .is_none_or(|e| e.code().is_some_and(|c| REFUSALS.contains(&c))) =>
        {
            tracing::warn!(
                workspace_id = ws,
                topic_id = ctx.str("topicId").unwrap_or(""),
                contact_id = contact,
                conversation_id = ctx.run.conversation.id,
                action = step_type,
                "Coupon {what} skipped"
            );
            StepStatus::Success
        }
        Err(error) => step_error(
            ctx,
            &format!(
                "Coupon {what} failed: {}",
                error.map(|e| e.message()).unwrap_or_default()
            ),
        ),
    }))
}
