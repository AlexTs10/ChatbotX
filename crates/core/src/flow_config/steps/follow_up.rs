//! Port of `packages/flow-config/src/steps/follow-up.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const FOLLOW_UP_MAX_DELAY_DAYS: usize = 366;

schema!(pub fn follow_up_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("followUp")).describe("Step type discriminator: \"followUp\".")),
("duration", z::number().coerce().int().gte(1 as f64).describe("Delay length in `unit`s (integer >= 1). The total delay cannot exceed 366 days.")),
("unit", wait_step_delay_units().describe("Unit for `duration`."))]).super_refine(follow_up_step_schema_super_refine));

default_fn!(pub fn follow_up_step_default_fn = json!({"id": create_id(), "stepType": "followUp", "duration": 1, "unit": "hours"}));

// HAND-PORT computeFollowUpTriggerAt:
// step=>addMilliseconds(Date.now(),step.duration*delayUnitToMs(step.unit))

// (data,ctx)=>{if(data.duration*delayUnitToMs(data.unit)>FOLLOW_UP_MAX_DELAY_MS){ctx.addIssue({code:"custom",path:["duration"],message:`Follow-up delay cannot exceed ${FOLLOW_UP_MAX_DELAY_DAYS} days`})}}
fn follow_up_step_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn compute_follow_up_trigger_at(
    step: &Value,
    clock: &dyn crate::Clock,
) -> chrono::DateTime<chrono::Utc> {
    todo!("P3.4")
}
