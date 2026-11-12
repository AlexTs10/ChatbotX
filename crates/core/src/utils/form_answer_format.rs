//! `packages/utils/src/form-answer-format.ts`.

use serde_json::Value;

/// `formatDateRange`: a `{ "start-date", "end-date" }` answer as
/// `start - end`; `None` for any other value.
pub fn format_date_range(value: &Value) -> Option<String> {
    let range = value.as_object()?;
    match (range.get("start-date"), range.get("end-date")) {
        (Some(Value::String(start)), Some(Value::String(end))) => Some(format!("{start} - {end}")),
        _ => None,
    }
}
