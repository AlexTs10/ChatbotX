//! Port of `packages/flow-config/src/steps/wait.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn wait_step_delay_types = z::enum_(&["duration", "date", "random"]));

schema!(pub fn wait_step_delay_units = z::enum_(&["seconds", "minutes", "hours", "days"]));

schema!(pub fn wait_step_date_types = z::enum_(&["specific", "dynamic"]));

schema!(pub fn wait_step_offset_operators = z::enum_(&["add", "subtract"]));

pub const ENQUEUE_DELAY_MS: usize = 300000;

schema!(pub fn wait_step_schema = z::intersection(z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("wait")).describe("Step type discriminator: \"wait\"."))]), z::discriminated_union("delayType", vec![z::object(vec![("delayType", z::literal(json!("duration")).describe("Wait a fixed duration.")),
("duration", z::number().coerce().int().gte(1_f64).lte(999999_f64).describe("Wait length in `unit`s (integer 1-999999).")),
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
("offsetValue", z::number().coerce().int().gte(1_f64).lte(999999_f64).optional().describe("Offset amount in `offsetUnit`s (integer 1-999999).")),
("offsetUnit", wait_step_delay_units().optional().describe("Unit for `offsetValue`."))]),
z::object(vec![("delayType", z::literal(json!("random")).describe("Wait a random duration between `min` and `max`.")),
("min", z::number().coerce().int().gte(1_f64).lte(999999_f64).describe("Shortest wait in `unit`s (integer 1-999999).")),
("max", z::number().coerce().int().gte(1_f64).lte(999999_f64).describe("Longest wait in `unit`s; must be >= `min`.")),
("unit", wait_step_delay_units().describe("Unit for `min` and `max`."))])])).super_refine(wait_step_schema_super_refine));

default_fn!(pub fn wait_step_default_fn = json!({"id": create_id(), "stepType": "wait", "delayType": "duration", "duration": 1, "unit": "hours", "interval": false, "startTime": null, "endTime": null}));

default_fn!(pub fn delay_type_duration_default_fn = json!({"duration": 1, "unit": "hours", "interval": false, "startTime": null, "endTime": null}));

fn wait_step_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    if data["delayType"] == "random" && js_number(data.get("min")) > js_number(data.get("max")) {
        ctx.issue(path!["max"], "Max must be \u{2265} Min");
    }
    if data["delayType"] == "date" {
        if data["dateType"] == "specific" && !truthy(data.get("datetime")) {
            ctx.issue(path!["datetime"], "Required");
        }
        if data["dateType"] == "dynamic" && !truthy(data.get("outputFieldId")) {
            ctx.issue(path!["outputFieldId"], "Required");
        }
    }
    if data["delayType"] == "duration" && truthy(data.get("interval")) {
        if !truthy(data.get("startTime")) {
            ctx.issue(path!["startTime"], "Required when interval is enabled");
        }
        if !truthy(data.get("endTime")) {
            ctx.issue(path!["endTime"], "Required when interval is enabled");
        }
    }
}

/// `delayUnitToMs(unit)`.
pub fn delay_unit_to_ms(unit: &str) -> i64 {
    match unit {
        "seconds" => 1000,
        "minutes" => 60_000,
        "hours" => 3_600_000,
        "days" => 86_400_000,
        _ => 0,
    }
}

/// `buildJobId(rowId, triggerAt)`.
pub fn build_job_id(row_id: &str, trigger_at: chrono::DateTime<chrono::Utc>) -> String {
    format!("smart-delay-{row_id}-{}", trigger_at.timestamp_millis())
}

/// `computeTriggerAt(step, getCustomFieldValue)`. The servers run in UTC,
/// so date-fns local-time math is UTC math. `random` is `Math.random()`;
/// `custom_field_value` is the awaited `getCustomFieldValue(outputFieldId)`
/// (`None` when there is no getter or no value).
pub fn compute_trigger_at(
    step: &Value,
    now_ms: i64,
    random: f64,
    custom_field_value: Option<&str>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Duration, Timelike};
    let to_date = |ms: i64| chrono::DateTime::from_timestamp_millis(ms);
    let time_to_seconds = |time: &str| -> i64 {
        let mut parts = time.split(':').map(crate::js::to_number);
        let h = parts.next().unwrap_or(0.0);
        let m = parts.next().unwrap_or(0.0);
        let s = parts.next().unwrap_or(0.0);
        (h * 3600.0 + m * 60.0 + s) as i64
    };
    let set_time_of_day = |date: chrono::DateTime<chrono::Utc>, total: i64| {
        let midnight = date
            .with_hour(0)?
            .with_minute(0)?
            .with_second(0)?
            .with_nanosecond(0)?;
        Some(midnight + Duration::seconds(total))
    };
    let time_of_day = |d: chrono::DateTime<chrono::Utc>| i64::from(d.num_seconds_from_midnight());
    let delay_type = str_field(step, "delayType");
    if delay_type == "duration" {
        let unit_ms = delay_unit_to_ms(str_field(step, "unit"));
        let base = to_date(now_ms + (js_number(step.get("duration")) * unit_ms as f64) as i64)?;
        let (start, end) = (str_field(step, "startTime"), str_field(step, "endTime"));
        if !(truthy(step.get("interval")) && !start.is_empty() && !end.is_empty()) {
            return Some(base);
        }
        let window_start = time_to_seconds(start);
        let window_end = time_to_seconds(end);
        let current = time_of_day(base);
        if window_start > window_end {
            let inside = current >= window_start || current <= window_end;
            return if inside {
                Some(base)
            } else {
                set_time_of_day(base, window_start)
            };
        }
        if current < window_start {
            return set_time_of_day(base, window_start);
        }
        if current > window_end {
            return set_time_of_day(base + Duration::days(1), window_start);
        }
        return Some(base);
    }
    if delay_type == "random" {
        let unit_ms = delay_unit_to_ms(str_field(step, "unit"));
        let (min, max) = (js_number(step.get("min")), js_number(step.get("max")));
        let rand = (random * (max - min + 1.0) + min).floor();
        return to_date(now_ms + (rand * unit_ms as f64) as i64);
    }
    let mut trigger_at = None;
    if delay_type == "date" {
        if step["dateType"] == "specific" {
            let datetime = str_field(step, "datetime");
            if datetime.is_empty() {
                return None;
            }
            trigger_at = crate::js::parse_date(datetime).and_then(to_date);
        }
        if step["dateType"] == "dynamic" {
            if !truthy(step.get("outputFieldId")) {
                return None;
            }
            let raw = custom_field_value.filter(|r| !r.is_empty())?;
            let parsed = crate::js::parse_date(raw).and_then(to_date)?;
            trigger_at = Some(parsed);
            if truthy(step.get("offset"))
                && truthy(step.get("offsetValue"))
                && truthy(step.get("offsetUnit"))
                && truthy(step.get("offsetOperator"))
            {
                let offset_ms = delay_unit_to_ms(str_field(step, "offsetUnit"));
                let sign = if step["offsetOperator"] == "add" {
                    1.0
                } else {
                    -1.0
                };
                let delta = (sign * js_number(step.get("offsetValue")) * offset_ms as f64) as i64;
                trigger_at = to_date(parsed.timestamp_millis() + delta);
            }
        }
    }
    trigger_at
}
