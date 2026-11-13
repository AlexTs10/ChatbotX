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
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "openHours.list" => service::list(db, ws)
            .await
            .map(|rows| {
                json(
                    200,
                    json!({ "data": rows.iter().map(open_hours_json).collect::<Vec<_>>() }),
                )
            })
            .map_err(|e| business_error(e, super::LISTING)),
        "openHours.create" => {
            let input = Input::from_body(call.params, call.body);
            let body = input.obj.clone();
            input.finish()?;
            service::create(db, ws, &body)
                .await
                .map(|row| json(200, open_hours_json(&row)))
                .map_err(|e| schedule_error(e, super::CREATING))
        }
        "openHours.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let open_hours_id = path_id(&mut input);
            let mut body = input.obj.clone();
            body.remove("id");
            if let Err(issues) =
                chatbotx_business::ecommerce_open_hours::parse_schedule(&body, true)
            {
                input.extend_issues(issues);
            }
            input.finish()?;
            service::update(db, ws, open_hours_id.unwrap_or_default(), &body)
                .await
                .map(|row| json(200, open_hours_json(&row)))
                .map_err(|e| schedule_error(e, super::MUTATING))
        }
        "openHours.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let open_hours_id = path_id(&mut input);
            input.finish()?;
            service::delete(db, ws, open_hours_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
