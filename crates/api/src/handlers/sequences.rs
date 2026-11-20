//! `features/sequences/api/public.ts` (P3.6-D2 `sequences.create`; Q.D5 the
//! rest; Q.P6-U5 `sequences.listStepContacts`).

use chatbotx_business::BusinessError;
use chatbotx_business::folder;
use chatbotx_business::sequence::{self as service, DELAY_UNITS, StepPayload, UpdateSequence};
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::sequence::{SequenceListInput, SequenceWithSteps};
use serde_json::{Value, json};

use super::flows::query_bool;
use super::folders::folder_json;
use super::input::{Input, Num, Str, key};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "sequences.list",
    "sequences.get",
    "sequences.create",
    "sequences.update",
    "sequences.delete",
    "sequences.upsertStep",
    "sequences.deleteStep",
    "sequences.listFolders",
    "sequences.createFolder",
    "sequences.updateFolder",
    "sequences.deleteFolder",
    "sequences.listStepContacts",
];

/// `sequenceStepEventTypes` (message + flow event types).
pub(crate) const STEP_EVENT_TYPES: &[&str] = &[
    "message:sent",
    "message:delivered",
    "message:seen",
    "message:failed",
    "message:received",
    "flow:clicked",
    "flow:ref",
];

/// `possibleErrorsOnCreatingInFolder`.
const CREATING_IN_FOLDER: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
];

/// `sequenceDetailResource`: the sequence and its steps (no `flow`).
pub fn detail_json(found: &SequenceWithSteps) -> Value {
    let mut out = found.sequence.to_json();
    out["steps"] = found.steps.iter().map(|s| s.step.to_json()).collect();
    out
}

/// `sequenceFolderResource` (no `workspaceId`).
fn sequence_folder_json(folder: &chatbotx_db::repo::models::Folder) -> Value {
    let mut out = folder_json(folder);
    if let Value::Object(map) = &mut out {
        map.remove("workspaceId");
    }
    out
}

/// `createSequenceRequest.name` / the folder names: trimmed, 1..=255.
fn name(input: &mut Input) -> Option<String> {
    input.string("name", Str::trimmed(1, 255))
}

/// A nullish `zodBigintAsString` body key: absent = `None`, `null` = `Some(None)`.
fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.get(name).cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => input.digits_id(name).map(Some),
    }
}

/// The step fields of `upsertSequenceStepBaseShape` (no `sequenceId`) and
/// `validateStepDelayConsistency`: `(stepId, payload)`.
pub(crate) fn step_payload(input: &mut Input) -> (Option<i64>, StepPayload) {
    let step_id = nullish_digits(input, "stepId").flatten();
    let order = input
        .number("order", Num::int(0.0, f64::MAX))
        .map(|n| n as i64);
    let delay_days = input
        .opt_number("delayDays", Num::int(0.0, f64::MAX))
        .map(|n| n as i64);
    let delay_minutes = input
        .opt_number("delayMinutes", Num::int(0.0, f64::MAX))
        .map(|n| n as i64);
    let delay_unit = match input.get("delayUnit").cloned() {
        None => None,
        Some(value) => input.enum_at(&key("delayUnit"), Some(&value), DELAY_UNITS, true),
    };
    let specific_date_time = match input.get("specificDateTime").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(text)) if super::minigames::is_iso_datetime(&text) => {
            Some(chatbotx_core::js::parse_date(&text))
        }
        Some(_) => {
            input.issue(
                &key("specificDateTime"),
                "invalid_format",
                "Invalid ISO datetime",
            );
            None
        }
    };
    let flow_id = match input.get("flowId") {
        None => None,
        Some(_) => input.digits_id("flowId"),
    };
    let is_active = input.opt_bool("isActive");
    let anytime = input.opt_bool("anytime");
    let send_time_start = input.nullish_string("sendTimeStart", Str::default());
    let send_time_end = input.nullish_string("sendTimeEnd", Str::default());
    let send_days = match input.get("sendDays").cloned() {
        None => None,
        Some(Value::Array(items)) => {
            let mut days = Vec::new();
            for (index, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(day) => days.push(day.to_string()),
                    None => input.issue(
                        &[
                            super::input::Seg::Key("sendDays".into()),
                            super::input::Seg::Index(index),
                        ],
                        "invalid_type",
                        "Invalid input: expected string",
                    ),
                }
            }
            Some(days)
        }
        Some(_) => {
            input.issue(
                &key("sendDays"),
                "invalid_type",
                "Invalid input: expected array",
            );
            None
        }
    };
    // `validateStepDelayConsistency` (a refinement: only when the shape passed).
    if !input.failed()
        && let (Some(unit), Some(days), Some(minutes)) = (&delay_unit, delay_days, delay_minutes)
    {
        if !service::is_stored_delay_consistent(unit, days, minutes) {
            input.issue(
                &key("delayUnit"),
                "custom",
                "delayUnit does not match delayDays/delayMinutes",
            );
        } else if unit == "specificTime" && specific_date_time.flatten().is_none() {
            input.issue(
                &key("specificDateTime"),
                "custom",
                "specificDateTime is required for delayUnit specificTime",
            );
        }
    }
    (
        step_id,
        StepPayload {
            order: order.unwrap_or_default(),
            delay_days,
            delay_minutes,
            delay_unit,
            flow_id,
            specific_date_time,
            is_active,
            anytime,
            send_time_start,
            send_time_end,
            send_days,
        },
    )
}

/// `requireSequenceFolder`: a folder of another type reads as missing.
async fn require_sequence_folder(
    state: &ApiState,
    ws: i64,
    id: i64,
    errors: ErrorMap,
) -> Result<(), crate::orpc::OrpcError> {
    let found = folder::find_or_fail(&state.db, ws, id, None)
        .await
        .map_err(|e| business_error(e, errors))?;
    if found.folder_type != "sequence" {
        return Err(business_error(
            BusinessError::not_found("Folder not found"),
            errors,
        ));
    }
    Ok(())
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "sequences.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", Str::trimmed(1, usize::MAX));
            let folder_id = match input.get("folderId") {
                None => None,
                Some(_) => input.digits_id("folderId"),
            };
            let active = query_bool(&mut input, "active");
            let sort = sort_query(&mut input, None);
            input.finish()?;
            let (data, page_count) = service::list(
                db,
                SequenceListInput {
                    workspace_id: ws,
                    name,
                    folder_id: folder_id.map(|id| id.to_string()),
                    active,
                    page: Some(page),
                    per_page: Some(per_page),
                    sort: Some(sort.unwrap_or_else(|| {
                        vec![SortItem {
                            id: "createdAt".into(),
                            desc: true,
                        }]
                    })),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = data
                .iter()
                .map(|row| {
                    let mut out = row.sequence.to_json();
                    out["stepsCount"] = json!(row.steps_count);
                    out["subscribersCount"] = json!(row.subscribers_count);
                    out
                })
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "sequences.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            service::find_with_steps(db, ws, id.unwrap_or_default())
                .await
                .map(|found| json(200, detail_json(&found)))
                .map_err(|e| business_error(e, FINDING))
        }
        "sequences.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = name(&mut input);
            let folder_id = nullish_digits(&mut input, "folderId");
            input.finish()?;
            // `input.folderId || null`.
            let sequence_id =
                service::create(db, ws, &name.unwrap_or_default(), folder_id.flatten())
                    .await
                    .map_err(|e| business_error(e, CREATING_IN_FOLDER))?;
            state
                .record_audit(
                    "create",
                    &format!("created a new sequence (#{sequence_id})"),
                )
                .await;
            Ok(json(201, json!({ "sequenceId": sequence_id.to_string() })))
        }
        "sequences.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let name = input.opt_string("name", Str::trimmed(1, 255));
            let active = input.opt_bool("active");
            let folder_id = match input.get("folderId").cloned() {
                None => None,
                Some(Value::Null) => Some(None),
                Some(_) => input.digits_id("folderId").map(Some),
            };
            input.finish()?;
            let detail = service::update(
                db,
                ws,
                id.unwrap_or_default(),
                UpdateSequence {
                    name,
                    active,
                    folder_id,
                },
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            if let Some(detail) = detail {
                state.record_audit("update", &detail).await;
            }
            Ok(no_content())
        }
        "sequences.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id").unwrap_or_default();
            input.finish()?;
            service::delete(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            state
                .record_audit("delete", &format!("deleted a sequence (#{id})"))
                .await;
            Ok(no_content())
        }
        "sequences.upsertStep" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let (step_id, data) = step_payload(&mut input);
            input.finish()?;
            let id = id.unwrap_or_default();
            service::assert_owned(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let step_id = service::upsert_step(db, ws, id, step_id, data)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "stepId": step_id.to_string() })))
        }
        "sequences.deleteStep" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id").unwrap_or_default();
            let step_id = input.digits_id("stepId").unwrap_or_default();
            input.finish()?;
            service::assert_owned(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            service::delete_step(db, ws, step_id, Some(id))
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "sequences.listFolders" => {
            let mut input = Input::from_query(call.params, call.query);
            let parent_id = match input.get("parentId") {
                None => None,
                Some(_) => input.digits_id("parentId"),
            };
            let is_trash = query_bool(&mut input, "isTrash");
            input.finish()?;
            let parent = parent_id.map_or_else(|| "0".to_string(), |id| id.to_string());
            let folders = folder::list(db, ws, "sequence", Some(&parent), is_trash)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = folders.iter().map(sequence_folder_json).collect();
            Ok(json(200, json!({ "data": data })))
        }
        "sequences.createFolder" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = name(&mut input);
            let parent_id = nullish_digits(&mut input, "parentId").flatten();
            input.finish()?;
            let parent_id = parent_id.filter(|id| *id != 0);
            folder::create(db, ws, &name.unwrap_or_default(), "sequence", parent_id)
                .await
                .map(|f| json(201, sequence_folder_json(&f)))
                .map_err(|e| business_error(e, CREATING_IN_FOLDER))
        }
        "sequences.updateFolder" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id").unwrap_or_default();
            let name = name(&mut input);
            input.finish()?;
            require_sequence_folder(state, ws, id, MUTATING).await?;
            folder::update(db, ws, id, &name.unwrap_or_default())
                .await
                .map(|f| json(200, sequence_folder_json(&f)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "sequences.deleteFolder" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id").unwrap_or_default();
            input.finish()?;
            require_sequence_folder(state, ws, id, MUTATING).await?;
            folder::bulk_delete(db, ws, &[id])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "sequences.listStepContacts" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id").unwrap_or_default();
            let step_id = input.digits_id("stepId").unwrap_or_default();
            let value = input.get("eventType").cloned();
            let event_type =
                input.enum_at(&key("eventType"), value.as_ref(), STEP_EVENT_TYPES, true);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            service::assert_owned(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            service::list_step_contacts_page(
                db,
                ws,
                id,
                step_id,
                event_type.unwrap_or_default(),
                page,
                per_page,
            )
            .await
            .map(|page| json(200, page))
            .map_err(|e| business_error(e, FINDING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
