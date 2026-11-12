//! Port of `packages/flow-config/src/steps/wait.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn wait_step_delay_types = z::enum_(&["duration", "date", "random"]));

schema!(pub fn wait_step_delay_units = z::enum_(&["seconds", "minutes", "hours", "days"]));

schema!(pub fn wait_step_date_types = z::enum_(&["specific", "dynamic"]));

schema!(pub fn wait_step_offset_operators = z::enum_(&["add", "subtract"]));

pub const ENQUEUE_DELAY_MS: usize = 300000;

// HAND-PORT delayUnitToMs:
// unit=>UNIT_MS[unit]

schema!(pub fn wait_step_schema = z::intersection(z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("wait")).describe("Step type discriminator: \"wait\"."))]), z::discriminated_union("delayType", vec![z::object(vec![("delayType", z::literal(json!("duration")).describe("Wait a fixed duration.")),
("duration", z::number().coerce().int().gte(1 as f64).lte(999999 as f64).describe("Wait length in `unit`s (integer 1-999999).")),
("unit", wait_step_delay_units().describe("Unit for `duration`.")),
("interval", z::boolean().describe("If true, only resume inside the daily window `startTime`-`endTime` (both then required); otherwise the wait ends exactly after `duration`.")),
("startTime", z::string().iso_time().nullable().describe("Daily window start, ISO time (HH:mm:ss). Required when `interval` is true, else null.")),
("endTime", z::string().iso_time().nullable().describe("Daily window end, ISO time (HH:mm:ss). Required when `interval` is true, else null."))]),
z::object(vec![("delayType", z::literal(json!("date")).describe("Wait until a date.")),
("dateType", wait_step_date_types().describe("`specific` waits until `datetime`; `dynamic` waits until the date stored in the custom field `outputFieldId`.")),
("datetime", z::string().iso_datetime(false, false, None).optional().describe("ISO 8601 datetime to wait until. Required when `dateType` is `specific`.")),
("outputFieldId", z::string().trim().min(1).optional().describe("Id of the date custom field holding the target date. Required when `dateType` is `dynamic`.")),
("offset", z::boolean().default(json!(false)).describe("Apply `offsetValue` `offsetUnit` before/after the dynamic date (uses `offsetOperator`).")),
("offsetOperator", wait_step_offset_operators().optional().describe("`add` waits past the date; `subtract` ends before it.")),
("offsetValue", z::number().coerce().int().gte(1 as f64).lte(999999 as f64).optional().describe("Offset amount in `offsetUnit`s (integer 1-999999).")),
("offsetUnit", wait_step_delay_units().optional().describe("Unit for `offsetValue`."))]),
z::object(vec![("delayType", z::literal(json!("random")).describe("Wait a random duration between `min` and `max`.")),
("min", z::number().coerce().int().gte(1 as f64).lte(999999 as f64).describe("Shortest wait in `unit`s (integer 1-999999).")),
("max", z::number().coerce().int().gte(1 as f64).lte(999999 as f64).describe("Longest wait in `unit`s; must be >= `min`.")),
("unit", wait_step_delay_units().describe("Unit for `min` and `max`."))])])).super_refine(wait_step_schema_super_refine));

default_fn!(pub fn wait_step_default_fn = json!({"id": create_id(), "stepType": "wait", "delayType": "duration", "duration": 1, "unit": "hours", "interval": false, "startTime": null, "endTime": null}));

default_fn!(pub fn delay_type_duration_default_fn = json!({"duration": 1, "unit": "hours", "interval": false, "startTime": null, "endTime": null}));

// HAND-PORT buildJobId:
// (rowId,triggerAt)=>`smart-delay-${rowId}-${triggerAt.getTime()}`

// (data,ctx)=>{if(data.delayType===waitStepDelayTypes.enum.random&&data.min>data.max){ctx.addIssue({code:"custom",path:["max"],message:"Max must be \u2265 Min"})}if(data.delayType===waitStepDelayTypes.enum.date){if(data.dateType===waitStepDateTypes.enum.specific&&!data.datetime){ctx.addIssue({code:"custom",path:["datetime"],message:"Required"})}if(data.dateType===waitStepDateTypes.enum.dynamic&&!data.outputFieldId){ctx.addIssue({code:"custom",path:["outputFieldId"],message:"Required"})}}if(data.delayType===waitStepDelayTypes.enum.duration&&data.interval){if(!data.startTime){ctx.addIssue({code:"custom",path:["startTime"],message:"Required when interval is enabled"})}if(!data.endTime){ctx.addIssue({code:"custom",path:["endTime"],message:"Required when interval is enabled"})}}}
fn wait_step_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn build_job_id(row_id: &str, trigger_at: chrono::DateTime<chrono::Utc>) -> String {
    todo!("P3.4")
}

pub fn delay_unit_to_ms(unit: &str) -> i64 {
    todo!("P3.4")
}
