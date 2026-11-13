//! `openHoursRangeSchema` / `openHoursScheduleSchema` (`database/partials/ecommerce.ts`).
//! Issues are zod-shaped `{ code, path, message }` objects.

use chatbotx_core::js;
use serde_json::{Map, Value, json};

/// `OpenHoursRange`: `day` 0 (Sunday) … 6, `from` / `to` as `HH:mm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHoursRange {
    pub day: u32,
    pub from: String,
    pub to: String,
}

impl OpenHoursRange {
    /// The stored / output JSON.
    pub fn to_json(&self) -> Value {
        todo!("P3.6-D1")
    }
}

/// `OpenHoursSchedule` (`partial`: each part optional).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenHoursSchedule {
    pub name: Option<String>,
    pub ranges: Option<Vec<OpenHoursRange>>,
}

/// At most 7 days × 4 ranges.
pub const MAX_RANGES: usize = 7 * 4;

/// `TIME_OF_DAY_PATTERN` (`/^([01]\d|2[0-3]):[0-5]\d$/`).
pub fn is_time_of_day(text: &str) -> bool {
    todo!("P3.6-D1")
}

fn issue(issues: &mut Vec<Value>, path: Vec<Value>, code: &str, message: &str) {
    issues.push(json!({ "code": code, "path": path, "message": message }));
}

fn path(base: &[Value], key: impl Into<Value>) -> Vec<Value> {
    let mut out = base.to_vec();
    out.push(key.into());
    out
}

fn parse_range(issues: &mut Vec<Value>, at: &[Value], value: &Value) -> Option<OpenHoursRange> {
    let Some(obj) = value.as_object() else {
        issue(
            issues,
            at.to_vec(),
            "invalid_type",
            "Invalid input: expected object",
        );
        return None;
    };
    let before = issues.len();
    let day = match obj.get("day") {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(d) if d.fract() == 0.0 && (0.0..=6.0).contains(&d) => Some(d as u32),
            Some(d) if d.fract() != 0.0 => {
                issue(
                    issues,
                    path(at, "day"),
                    "invalid_type",
                    "Invalid input: expected int, received number",
                );
                None
            }
            _ => {
                issue(
                    issues,
                    path(at, "day"),
                    "too_big",
                    "Number must be between 0 and 6",
                );
                None
            }
        },
        _ => {
            issue(
                issues,
                path(at, "day"),
                "invalid_type",
                "Invalid input: expected number",
            );
            None
        }
    };
    let mut time = |key: &str| match obj.get(key) {
        Some(Value::String(text)) if is_time_of_day(text) => Some(text.clone()),
        Some(Value::String(_)) => {
            issue(
                issues,
                path(at, key),
                "invalid_format",
                "Invalid string: must match pattern",
            );
            None
        }
        _ => {
            issue(
                issues,
                path(at, key),
                "invalid_type",
                "Invalid input: expected string",
            );
            None
        }
    };
    let from = time("from");
    let to = time("to");
    if issues.len() != before {
        return None;
    }
    let (day, from, to) = (day?, from?, to?);
    if from == to {
        issue(
            issues,
            at.to_vec(),
            "custom",
            "Opening and closing time must differ.",
        );
        return None;
    }
    Some(OpenHoursRange { day, from, to })
}

/// Parses `name` / `ranges` of `obj` (`partial` = `openHoursScheduleSchema.partial()`).
pub fn parse_schedule(
    obj: &Map<String, Value>,
    partial: bool,
) -> Result<OpenHoursSchedule, Vec<Value>> {
    todo!("P3.6-D1")
}

/// The ranges stored as JSON text (unknown keys dropped, like the output schema).
pub fn ranges_from_json(text: &str) -> Vec<OpenHoursRange> {
    todo!("P3.6-D1")
}
