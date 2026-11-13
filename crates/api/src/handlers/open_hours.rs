//! `features/ecommerce-open-hours/api/public.ts`: `openHours.*`.

use chatbotx_business::BusinessError;
use chatbotx_business::ecommerce_open_hours::service::{self, OpenHoursRow};
use serde_json::{Value, json};

use super::input::Input;
use super::{ApiState, Call, Reply, business_error, id, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "openHours.list",
    "openHours.create",
    "openHours.update",
    "openHours.delete",
];

/// `publicOpenHoursResource`.
fn open_hours_json(row: &OpenHoursRow) -> Value {
    json!({
        "id": id(row.id),
        "name": row.name,
        "ranges": row.ranges.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
    })
}

/// `zodBigintAsString()` path id (digits; a value past i64 is an unknown id).
fn path_id(input: &mut Input) -> Option<i64> {
    let text = input.string("id", super::input::Str::default())?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &super::input::key("id"),
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

/// A service-side schedule parse error is already the oRPC 422.
fn schedule_error(error: BusinessError, errors: super::ErrorMap) -> crate::orpc::OrpcError {
    let mut out = business_error(error, errors);
    if out.code == "invalidRequestData" {
        out.defined = true;
    }
    out
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    todo!("P3.6-D1")
}
