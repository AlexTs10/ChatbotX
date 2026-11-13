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
        json!({ "day": self.day, "from": self.from, "to": self.to })
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
    let b = text.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && matches!(
            (b[0], b[1]),
            (b'0' | b'1', b'0'..=b'9') | (b'2', b'0'..=b'3')
        )
        && (b'0'..=b'5').contains(&b[3])
        && b[4].is_ascii_digit()
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
    let mut issues = Vec::new();
    let name = match obj.get("name") {
        None if partial => None,
        Some(Value::String(text)) => {
            let trimmed = js::trim(text).to_string();
            let length = js::utf16_len(&trimmed);
            if length < 1 {
                issue(
                    &mut issues,
                    vec![json!("name")],
                    "too_small",
                    "Too small: expected string to have >=1 characters",
                );
                None
            } else if length > 255 {
                issue(
                    &mut issues,
                    vec![json!("name")],
                    "too_big",
                    "Too big: expected string to have <=255 characters",
                );
                None
            } else {
                Some(trimmed)
            }
        }
        _ => {
            issue(
                &mut issues,
                vec![json!("name")],
                "invalid_type",
                "Invalid input: expected string",
            );
            None
        }
    };
    let ranges = match obj.get("ranges") {
        None if partial => None,
        Some(Value::Array(items)) => {
            let at = vec![json!("ranges")];
            let parsed: Vec<Option<OpenHoursRange>> = items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_range(&mut issues, &path(&at, index), item))
                .collect();
            if items.len() > MAX_RANGES {
                issue(
                    &mut issues,
                    at.clone(),
                    "too_big",
                    "Too big: expected array to have <=28 items",
                );
            }
            if items.is_empty() {
                issue(
                    &mut issues,
                    at,
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
            }
            parsed.into_iter().collect::<Option<Vec<_>>>()
        }
        _ => {
            issue(
                &mut issues,
                vec![json!("ranges")],
                "invalid_type",
                "Invalid input: expected array",
            );
            None
        }
    };
    if issues.is_empty() {
        Ok(OpenHoursSchedule { name, ranges })
    } else {
        Err(issues)
    }
}

/// The ranges stored as JSON text (unknown keys dropped, like the output schema).
pub fn ranges_from_json(text: &str) -> Vec<OpenHoursRange> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(OpenHoursRange {
                day: u32::try_from(item.get("day")?.as_u64()?).ok()?,
                from: item.get("from")?.as_str()?.to_string(),
                to: item.get("to")?.as_str()?.to_string(),
            })
        })
        .collect()
}
