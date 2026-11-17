//! `workingHours.*` (`features/working-hours/api/public.ts`).

use chatbotx_business::working_hours::{self as wh, DateOverride, Interval, Range, Schedule};
use chatbotx_db::repo::models::WorkingHoursDateOverride;
use serde_json::{Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content,
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

fn intervals_json(intervals: &[Interval]) -> Value {
    intervals
        .iter()
        .map(|i| json!({ "weekday": i.weekday, "startMinute": i.start_minute, "endMinute": i.end_minute }))
        .collect()
}

fn override_json(o: &WorkingHoursDateOverride) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    out.insert("date".into(), json!(o.date));
    out.insert("name".into(), json!(o.name));
    out.insert("ranges".into(), o.ranges.0.clone());
    out
}

/// `toWorkingHoursResource` (`dateOverrides` only for get / update).
fn schedule_json(s: &Schedule, overrides: Option<&[WorkingHoursDateOverride]>) -> Value {
    let mut out = json!({
        "id": id(s.row.id),
        "name": s.row.name,
        "intervals": intervals_json(&s.intervals),
        "memberUserIds": s.member_user_ids.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
        "createdAt": date(s.row.created_at),
        "updatedAt": date(s.row.updated_at),
    });
    if let Some(overrides) = overrides {
        out["dateOverrides"] = overrides
            .iter()
            .map(|o| Value::Object(override_json(o)))
            .collect();
    }
    out
}

/// `toHolidayResource`.
fn holiday_json(o: &WorkingHoursDateOverride) -> Value {
    let mut out = override_json(o);
    out.insert("id".into(), id(o.id));
    out.insert("createdAt".into(), date(o.created_at));
    out.insert("updatedAt".into(), date(o.updated_at));
    Value::Object(out)
}

/// `paginateInMemory` (page default 1, perPage default 50).
fn paged(input: &mut Input, items: Vec<Value>) -> Value {
    let page = input
        .opt_coerce_int(
            "page",
            Num {
                int: true,
                min: Some(1.0),
                max: None,
            },
        )
        .unwrap_or(1);
    let per_page = input
        .opt_coerce_int("perPage", Num::int(1.0, 50.0))
        .unwrap_or(50);
    let start = usize::try_from((page - 1) * per_page).unwrap_or(usize::MAX);
    let page_count = ((items.len() as i64 + per_page - 1) / per_page).max(1);
    let data: Vec<Value> = items
        .into_iter()
        .skip(start)
        .take(per_page as usize)
        .collect();
    json!({ "data": data, "pageCount": page_count })
}

fn array<'a>(input: &mut Input, at: &Path, value: Option<&'a Value>, max: usize) -> &'a [Value] {
    match value {
        Some(Value::Array(items)) => {
            if items.len() > max {
                input.issue(
                    at,
                    "too_big",
                    &format!("Too big: expected array to have <={max} items"),
                );
            }
            items
        }
        other => {
            input.issue(
                at,
                "invalid_type",
                &format!(
                    "Invalid input: expected array, received {}",
                    super::input::type_name(other)
                ),
            );
            &[]
        }
    }
}

fn minutes(input: &mut Input, at: &Path, item: &Value) -> Option<(i64, i64)> {
    let start = input.number_at(
        &child(at, "startMinute"),
        item.get("startMinute"),
        Num::int(0.0, 1439.0),
        true,
    );
    let end = input.number_at(
        &child(at, "endMinute"),
        item.get("endMinute"),
        Num::int(1.0, 1440.0),
        true,
    );
    let (start, end) = (start? as i64, end? as i64);
    if start >= end {
        input.issue(
            &child(at, "endMinute"),
            "custom",
            "endMinute must be greater than startMinute",
        );
        return None;
    }
    Some((start, end))
}

fn intervals_at(input: &mut Input, name: &str, value: Option<&Value>) -> Vec<Interval> {
    let at = key(name);
    let items = array(input, &at, value, wh::MAX_INTERVALS_TOTAL);
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key(name.into()), Seg::Index(index)];
        let weekday = input.number_at(
            &child(&path, "weekday"),
            item.get("weekday"),
            Num::int(0.0, 6.0),
            true,
        );
        if let (Some(weekday), Some((start, end))) = (weekday, minutes(input, &path, item)) {
            out.push(Interval {
                weekday: weekday as i64,
                start_minute: start,
                end_minute: end,
            });
        }
    }
    if let Err(message) = wh::validate_intervals(&out) {
        input.issue(&at, "custom", &message);
    }
    out
}

fn ranges_at(input: &mut Input, at: &Path, value: Option<&Value>) -> Vec<Range> {
    let items = array(input, at, value, wh::MAX_INTERVALS_PER_WEEKDAY);
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut path = at.clone();
        path.push(Seg::Index(index));
        if let Some((start, end)) = minutes(input, &path, item) {
            out.push(Range {
                start_minute: start,
                end_minute: end,
            });
        }
    }
    if let Err(message) = wh::validate_ranges(&out) {
        input.issue(at, "custom", &message);
    }
    out
}

/// `holidayNameField` / the override `name`: trimmed, ≤ 100, nullish.
fn override_name(input: &mut Input, at: &Path, value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(input.string_at(
            at,
            Some(value),
            Str {
                trim: true,
                ..Str::max(wh::MAX_NAME_LENGTH)
            },
            true,
        )),
    }
}

fn local_date(input: &mut Input, at: &Path, value: Option<&Value>) -> String {
    let text = input
        .string_at(at, value, Str::default(), true)
        .unwrap_or_default();
    if !text.is_empty()
        && let Err(message) = wh::validate_local_date(&text)
    {
        input.issue(at, "custom", &message);
    }
    text
}

fn overrides_at(input: &mut Input, value: Option<&Value>) -> Vec<DateOverride> {
    let at = key("dateOverrides");
    let items = array(input, &at, value, wh::MAX_DATE_OVERRIDES);
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key("dateOverrides".into()), Seg::Index(index)];
        let date = local_date(input, &child(&path, "date"), item.get("date"));
        let name = override_name(input, &child(&path, "name"), item.get("name")).flatten();
        let ranges = ranges_at(input, &child(&path, "ranges"), item.get("ranges"));
        out.push(DateOverride { date, name, ranges });
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(dup) = out.iter().find(|o| !seen.insert(o.date.clone())) {
        input.issue(&at, "custom", &format!("Duplicate date {}", dup.date));
    }
    out
}

fn user_ids_at(input: &mut Input, value: Option<&Value>) -> Vec<i64> {
    let at = key("userIds");
    let items = array(input, &at, value, MAX_MEMBERS);
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            input.id_at(
                &[Seg::Key("userIds".into()), Seg::Index(index)],
                Some(item),
                true,
            )
        })
        .collect()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "workingHours.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let rows = wh::list(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let body = paged(
                &mut input,
                rows.iter().map(|s| schedule_json(s, None)).collect(),
            );
            input.finish()?;
            Ok(json(200, body))
        }
        "workingHours.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let schedule_id = input.id("id").unwrap_or(-1);
            input.finish()?;
            let row = wh::find_or_fail(db, ws, schedule_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let overrides = wh::list_date_overrides(db, ws, Some(schedule_id))
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, schedule_json(&row, Some(&overrides))))
        }
        "workingHours.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            let intervals = match input.get("intervals").cloned() {
                None => Vec::new(),
                Some(value) => intervals_at(&mut input, "intervals", Some(&value)),
            };
            let user_ids = match input.get("userIds").cloned() {
                None => Vec::new(),
                Some(value) => user_ids_at(&mut input, Some(&value)),
            };
            input.finish()?;
            wh::create(db, ws, name.unwrap_or_default(), intervals, user_ids)
                .await
                .map(|s| json(201, schedule_json(&s, None)))
                .map_err(|e| business_error(e, CREATING))
        }
        "workingHours.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let schedule_id = input.id("id").unwrap_or(-1);
            let name = match input.get("name").cloned() {
                None => None,
                Some(value) => {
                    input.string_at(&key("name"), Some(&value), Str::trimmed(1, 255), true)
                }
            };
            let intervals = input
                .get("intervals")
                .cloned()
                .map(|value| intervals_at(&mut input, "intervals", Some(&value)));
            let overrides = input
                .get("dateOverrides")
                .cloned()
                .map(|value| overrides_at(&mut input, Some(&value)));
            if input.get("name").is_none() && intervals.is_none() && overrides.is_none() {
                input.issue(
                    &[],
                    "custom",
                    "Provide at least one of `name`, `intervals` or `dateOverrides`",
                );
            }
            input.finish()?;
            let row = wh::update(db, ws, schedule_id, name, intervals, overrides)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let overrides = wh::list_date_overrides(db, ws, Some(schedule_id))
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, schedule_json(&row, Some(&overrides))))
        }
        "workingHours.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let schedule_id = input.id("id").unwrap_or(-1);
            input.finish()?;
            wh::delete(db, ws, schedule_id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.setMembers" => {
            let mut input = Input::from_body(call.params, call.body);
            let schedule_id = input.id("id").unwrap_or(-1);
            let value = input.get("userIds").cloned();
            let user_ids = user_ids_at(&mut input, value.as_ref());
            input.finish()?;
            wh::set_members(db, ws, schedule_id, user_ids)
                .await
                .map(|s| json(200, schedule_json(&s, None)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.listHolidays" => {
            let mut input = Input::from_query(call.params, call.query);
            let rows = wh::list_date_overrides(db, ws, None)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let body = paged(&mut input, rows.iter().map(holiday_json).collect());
            input.finish()?;
            Ok(json(200, body))
        }
        "workingHours.createHoliday" => {
            let mut input = Input::from_body(call.params, call.body);
            let date_value = input.get("date").cloned();
            let date = local_date(&mut input, &key("date"), date_value.as_ref());
            let name_value = input.get("name").cloned();
            let name = override_name(&mut input, &key("name"), name_value.as_ref()).flatten();
            let ranges_value = input.get("ranges").cloned();
            let ranges = ranges_at(&mut input, &key("ranges"), ranges_value.as_ref());
            input.finish()?;
            wh::add_holiday(db, ws, DateOverride { date, name, ranges })
                .await
                .map(|o| json(201, holiday_json(&o)))
                .map_err(|e| business_error(e, CREATING))
        }
        "workingHours.updateHoliday" => {
            let mut input = Input::from_body(call.params, call.body);
            let holiday_id = input.id("id").unwrap_or(-1);
            let name_value = input.get("name").cloned();
            let name = override_name(&mut input, &key("name"), name_value.as_ref());
            let ranges = input
                .get("ranges")
                .cloned()
                .map(|value| ranges_at(&mut input, &key("ranges"), Some(&value)));
            if name.is_none() && ranges.is_none() {
                input.issue(&[], "custom", "Provide at least one of `name` or `ranges`");
            }
            input.finish()?;
            wh::update_holiday(db, ws, holiday_id, name, ranges)
                .await
                .map(|o| json(200, holiday_json(&o)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "workingHours.deleteHoliday" => {
            let mut input = Input::from_query(call.params, call.query);
            let holiday_id = input.id("id").unwrap_or(-1);
            input.finish()?;
            wh::delete_holiday(db, ws, holiday_id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
