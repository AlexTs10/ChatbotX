//! Port of `packages/flow-config/src/steps/follow-up.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const FOLLOW_UP_MAX_DELAY_DAYS: usize = 366;

schema!(pub fn follow_up_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("followUp")).describe("Step type discriminator: \"followUp\".")),
("duration", z::number().coerce().int().gte(1_f64).describe("Delay length in `unit`s (integer >= 1). The total delay cannot exceed 366 days.")),
("unit", wait_step_delay_units().describe("Unit for `duration`."))]).super_refine(follow_up_step_schema_super_refine));

default_fn!(pub fn follow_up_step_default_fn = json!({"id": create_id(), "stepType": "followUp", "duration": 1, "unit": "hours"}));

const FOLLOW_UP_MAX_DELAY_MS: f64 = FOLLOW_UP_MAX_DELAY_DAYS as f64 * 86_400_000.0;

fn follow_up_step_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    let duration = data["duration"].as_f64().unwrap_or(f64::NAN);
    if duration * delay_unit_to_ms(str_field(data, "unit")) as f64 > FOLLOW_UP_MAX_DELAY_MS {
        ctx.issue(
            path!["duration"],
            format!("Follow-up delay cannot exceed {FOLLOW_UP_MAX_DELAY_DAYS} days"),
        );
    }
}

/// `computeFollowUpTriggerAt`: now + duration × unit.
pub fn compute_follow_up_trigger_at(
    step: &Value,
    clock: &dyn crate::Clock,
) -> chrono::DateTime<chrono::Utc> {
    let duration = step["duration"].as_f64().unwrap_or(0.0);
    let ms = clock.now_ms() + (duration * delay_unit_to_ms(str_field(step, "unit")) as f64) as i64;
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default()
}
