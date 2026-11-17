//! `features/working-hours/api/public.ts` (`workingHoursPublicRouter`, scope
//! `inbox`) and `api/private.ts`, plus the request checks the `/rpc`
//! actions share (`schema/action.ts`).
//!
//! ponytail: a failed partial schema (intervals, ranges, date) is one issue
//! per zod issue with code `custom`; zod's own codes (`too_small`, …) differ.

use chatbotx_business::working_hours::{self as service, Update};
use chatbotx_db::repo::partials::shared::ZodError;
use chatbotx_db::repo::partials::working_hours::{
    WORKING_HOURS_MAX_DATE_OVERRIDES, WORKING_HOURS_MAX_NAME_LENGTH,
    parse_working_hours_date_ranges, parse_working_hours_intervals, parse_working_hours_local_date,
};
use chatbotx_db::repo::working_hours::{DateOverrideInput, IntervalInput};
use serde_json::{Value, json};

use super::input::{Input, Path, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "workingHours.list",
    "workingHours.create",
    "workingHours.get",
    "workingHours.update",
    "workingHours.delete",
    "workingHours.setMembers",
    "workingHours.listHolidays",
    "workingHours.createHoliday",
    "workingHours.updateHoliday",
    "workingHours.deleteHoliday",
];

const MAX_MEMBERS: usize = 1000;

fn push_zod(input: &mut Input, at: &[Seg], error: ZodError) {
    for issue in error.issues {
        let mut path = at.to_vec();
        path.extend(issue.path.split('.').filter(|p| !p.is_empty()).map(|p| {
            p.parse::<usize>()
                .map_or_else(|_| Seg::Key(p.to_string()), Seg::Index)
        }));
        input.issue(&path, "custom", &issue.message);
    }
}

fn intervals_at(input: &mut Input, at: &[Seg], value: &Value) -> Option<Vec<IntervalInput>> {
    match parse_working_hours_intervals(value) {
        Ok(rows) => Some(
            rows.into_iter()
                .map(|i| IntervalInput {
                    weekday: i.weekday,
                    start_minute: i.start_minute,
                    end_minute: i.end_minute,
                })
                .collect(),
        ),
        Err(error) => {
            push_zod(input, at, error);
            None
        }
    }
}

fn ranges_at(input: &mut Input, at: &[Seg], value: Option<&Value>) -> Option<Value> {
    let Some(value) = value else {
        input.issue(
            at,
            "invalid_type",
            "Invalid input: expected array, received undefined",
        );
        return None;
    };
    parse_working_hours_date_ranges(value)
        .map_err(|error| push_zod(input, at, error))
        .ok()
}

fn date_at(input: &mut Input, at: &[Seg], value: Option<&Value>) -> Option<String> {
    let text = input.string_at(at, value, Str::default(), true)?;
    match parse_working_hours_local_date(&text) {
        Ok(()) => Some(text),
        Err(error) => {
            push_zod(input, at, error);
            None
        }
    }
}

/// `name: z.string().trim().max(100).nullish()` at `at`: `None` = absent.
fn holiday_name_at(input: &mut Input, at: &[Seg], value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(input.string_at(
            at,
            Some(v),
            Str {
                trim: true,
                ..Str::max(WORKING_HOURS_MAX_NAME_LENGTH)
            },
            true,
        )),
    }
}

/// `workingHoursDateOverrideInputSchema` at `at`.
fn date_override_at(input: &mut Input, at: &[Seg], value: &Value) -> Option<DateOverrideInput> {
    let Some(obj) = value.as_object() else {
        input.issue(at, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let date = date_at(input, &child(at, "date"), obj.get("date"));
    let name = holiday_name_at(input, &child(at, "name"), obj.get("name"));
    let ranges = ranges_at(input, &child(at, "ranges"), obj.get("ranges"));
    Some(DateOverrideInput {
        date: date?,
        name: name.flatten(),
        ranges: ranges?,
    })
}

fn date_overrides(input: &mut Input) -> Option<Vec<DateOverrideInput>> {
    let rows = array(
        input,
        "dateOverrides",
        0,
        WORKING_HOURS_MAX_DATE_OVERRIDES,
        |input, path, value| date_override_at(input, path, value),
    )?;
    let mut seen = std::collections::HashSet::new();
    if let Some(dup) = rows.iter().find(|o| !seen.insert(o.date.clone())) {
        input.issue(
            &key("dateOverrides"),
            "custom",
            &format!("Duplicate date {}", dup.date),
        );
        return None;
    }
    Some(rows)
}

fn user_ids(input: &mut Input) -> Option<Vec<i64>> {
    array(input, "userIds", 0, MAX_MEMBERS, |input, path, value| {
        digits_at(input, path, Some(value), true)
    })
}

fn name(input: &mut Input) -> Option<String> {
    input.string("name", Str::trimmed(1, 255))
}

/// `createWorkingHoursRequest`: `(name, intervals, userIds)`.
pub fn create_request(input: &mut Input) -> (String, Vec<IntervalInput>, Vec<i64>) {
    let name = name(input);
    let intervals = match input.get("intervals").cloned() {
        None => Some(Vec::new()),
        Some(value) => intervals_at(input, &key("intervals"), &value),
    };
    let users = if input.has("userIds") {
        user_ids(input)
    } else {
        Some(Vec::new())
    };
    (
        name.unwrap_or_default(),
        intervals.unwrap_or_default(),
        users.unwrap_or_default(),
    )
}

/// `updateWorkingHoursRequest` (with its "at least one" refine).
pub fn update_request(input: &mut Input) -> Update {
    let name = input.has("name").then(|| name(input)).flatten();
    let intervals = input
        .get("intervals")
        .cloned()
        .and_then(|value| intervals_at(input, &key("intervals"), &value));
    let date_overrides = if input.has("dateOverrides") {
        date_overrides(input)
    } else {
        None
    };
    if !input.failed()
        && !input.has("name")
        && !input.has("intervals")
        && !input.has("dateOverrides")
    {
        input.issue(
            &[],
            "custom",
            "Provide at least one of `name`, `intervals` or `dateOverrides`",
        );
    }
    Update {
        name,
        intervals,
        date_overrides,
    }
}

/// `setWorkingHoursMembersRequest`.
pub fn members_request(input: &mut Input) -> Vec<i64> {
    user_ids(input).unwrap_or_default()
}

/// `createHolidayRequest`.
pub fn create_holiday_request(input: &mut Input) -> DateOverrideInput {
    let date = date_at(input, &key("date"), input.get("date").cloned().as_ref());
    let name = holiday_name_at(input, &key("name"), input.get("name").cloned().as_ref());
    let ranges = ranges_at(input, &key("ranges"), input.get("ranges").cloned().as_ref());
    DateOverrideInput {
        date: date.unwrap_or_default(),
        name: name.flatten(),
        ranges: ranges.unwrap_or(Value::Array(Vec::new())),
    }
}

/// `updateHolidayRequest`: `(name, ranges)`; `None` = unchanged.
pub fn update_holiday_request(input: &mut Input) -> (Option<Option<String>>, Option<Value>) {
    let name = holiday_name_at(input, &key("name"), input.get("name").cloned().as_ref());
    let ranges = input
        .get("ranges")
        .cloned()
        .and_then(|value| ranges_at(input, &key("ranges"), Some(&value)));
    if !input.failed() && !input.has("name") && !input.has("ranges") {
        input.issue(&[], "custom", "Provide at least one of `name` or `ranges`");
    }
    (name, ranges)
}

/// The `{id}` path param (`zodBigintAsString`).
pub fn path_id(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    let at: Path = key("id");
    digits_at(input, &at, value.as_ref(), true).unwrap_or_default()
}

/// `listWorkingHoursOptionsWorkspaceAuthAPI`.
pub async fn options(state: &ApiState, workspace_id: i64) -> chatbotx_business::Result<Value> {
    let rows = service::list(&state.db, workspace_id).await?;
    Ok(json!({
        "data": rows
            .iter()
            .map(|r| json!({ "id": r.working_hours.id.to_string(), "name": r.working_hours.name }))
            .collect::<Vec<_>>()
    }))
}

fn page(input: &mut Input, rows: Vec<Value>) -> Value {
    let (page, per_page) = public_paging(input);
    let start = ((page - 1) * per_page).max(0) as usize;
    let page_count = ((rows.len() as i64 + per_page - 1) / per_page).max(1);
    let data: Vec<Value> = rows
        .into_iter()
        .skip(start)
        .take(per_page as usize)
        .collect();
    json!({ "data": data, "pageCount": page_count })
}

/// Runs a service call that returns `(output, audit detail)` and records the audit.
pub async fn audited<T>(
    state: &ApiState,
    action: &str,
    outcome: chatbotx_business::Result<(T, String)>,
) -> chatbotx_business::Result<T> {
    let (out, detail) = outcome?;
    state.record_audit(action, &detail).await;
    Ok(out)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "workingHours.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let rows = service::list(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let body = page(
                &mut input,
                rows.iter().map(|r| service::resource(r, None)).collect(),
            );
            input.finish()?;
            Ok(json(200, body))
        }
        "workingHours.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            service::get(db, ws, id)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, FINDING))
        }
        "workingHours.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, intervals, users) = create_request(&mut input);
            input.finish()?;
            audited(
                state,
                "create",
                service::create(db, ws, name, intervals, users).await,
            )
            .await
            .map(|out| json(201, out))
            .map_err(|e| business_error(e, CREATING))
        }
        "workingHours.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = update_request(&mut input);
            let id = path_id(&mut input);
            input.finish()?;
            audited(state, "update", service::update(db, ws, id, data).await)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            let outcome = service::delete(db, ws, id).await.map(|d| ((), d));
            audited(state, "delete", outcome)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.setMembers" => {
            let mut input = Input::from_body(call.params, call.body);
            let users = members_request(&mut input);
            let id = path_id(&mut input);
            input.finish()?;
            audited(
                state,
                "update",
                service::replace_members(db, ws, id, users).await,
            )
            .await
            .map(|out| json(200, out))
            .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.listHolidays" => {
            let mut input = Input::from_query(call.params, call.query);
            let rows = service::list_holidays(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let body = page(&mut input, rows.iter().map(service::holiday_json).collect());
            input.finish()?;
            Ok(json(200, body))
        }
        "workingHours.createHoliday" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = create_holiday_request(&mut input);
            input.finish()?;
            audited(state, "create", service::add_holiday(db, ws, data).await)
                .await
                .map(|row| json(201, service::holiday_json(&row)))
                .map_err(|e| business_error(e, CREATING))
        }
        "workingHours.updateHoliday" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, ranges) = update_holiday_request(&mut input);
            let id = path_id(&mut input);
            input.finish()?;
            audited(
                state,
                "update",
                service::update_holiday(db, ws, id, name, ranges).await,
            )
            .await
            .map(|row| json(200, service::holiday_json(&row)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.deleteHoliday" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            let outcome = service::delete_holiday(db, ws, id).await.map(|d| ((), d));
            audited(state, "delete", outcome)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
