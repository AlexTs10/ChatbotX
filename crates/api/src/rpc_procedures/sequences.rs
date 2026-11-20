//! Sequences: `features/sequences/api/authorized.ts` and
//! `features/sequences/actions/*` (Q.D5). The step stats / contacts
//! procedures need the sequence analytics (Q.D11).

use chatbotx_business::BusinessError;
use chatbotx_business::sequence::{self, UpdateSequence};
use chatbotx_db::repo::sequence::SequenceListInput;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, page};
use crate::handlers::input::Input;
use crate::handlers::sequences::STEP_EVENT_TYPES;
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createSequenceAction",
    "updateSequenceAction",
    "deleteSequenceAction",
    "upsertSequenceStepAction",
    "deleteSequenceStepAction",
    // The detail page query (`features/sequences/queries`).
    "getSequence",
];

/// `getSequence` → `findWithSteps`: the sequence, its steps by `order` with
/// their flow (`steps` and `sequenceSteps`); `null` = not found.
async fn get_sequence(ctx: &Ctx<'_>, args: &[Value]) -> Result<Value, OrpcError> {
    let Ok(id) = bound_id(arg(args, 1)) else {
        return Ok(Value::Null);
    };
    match sequence::find_with_steps(&ctx.api.db, ctx.workspace_id, id).await {
        Ok(found) => {
            let mut out = found.sequence.to_json();
            let steps: Vec<Value> = found
                .steps
                .iter()
                .map(|s| {
                    let mut step = s.step.to_json();
                    step["flow"] = s.flow.as_ref().map_or(Value::Null, |f| f.to_json());
                    step
                })
                .collect();
            out["steps"] = json!(steps);
            out["sequenceSteps"] = json!(steps);
            Ok(out)
        }
        Err(error) if error.code() == Some("notFound") => Ok(Value::Null),
        Err(error) => Err(fail(error)),
    }
}

/// `privateGetSequenceStepStatsAPI` / `privateListSequenceStepContactsAPI`.
async fn step_procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let sequence_id = input.id("sequenceId")?;
    let step_id = input.id("stepId")?;
    if path == "sequencesAPI.privateGetSequenceStepStatsAPI" {
        return sequence::step_stats(&ctx.api.db, ctx.workspace_id, sequence_id, step_id)
            .await
            .map_err(fail);
    }
    let event_type = input
        .get("eventType")
        .and_then(Value::as_str)
        .filter(|t| STEP_EVENT_TYPES.contains(t))
        .ok_or_else(invalid)?
        .to_owned();
    let number = |key: &str, default: f64| match input.get(key) {
        None => Ok(default),
        Some(value) => value.as_f64().ok_or_else(invalid),
    };
    let page = number("page", 1.0)? as i64;
    let per_page = (number("perPage", 20.0)? as i64).max(1);
    let mut out = sequence::list_step_contacts_page(
        &ctx.api.db,
        ctx.workspace_id,
        sequence_id,
        step_id,
        event_type,
        page,
        per_page,
    )
    .await
    .map_err(fail)?;
    out["page"] = json!(page);
    Ok(out)
}

/// A `zodBigintAsString` bind arg.
fn bound_id(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse::<i64>().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// `returnValidationErrors` of the create / update actions for a taken name.
fn name_exists(error: BusinessError) -> OrpcError {
    if error.code() == Some("validation") {
        let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some("Validation Exception"));
        out.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                "name": { "_errors": ["Sequence name already exists"] },
            }
        }));
        return out;
    }
    action_fail(error)
}

/// `updateSequenceSchema` (`.partial()`, no unknown-key check).
fn update_data(input: &In<'_>) -> Result<UpdateSequence, OrpcError> {
    let name = match input.0.get("name") {
        None => None,
        Some(_) => Some(input.str("name", true, 1, 255)?),
    };
    let active = match input.0.get("active") {
        None => None,
        Some(Value::Bool(flag)) => Some(*flag),
        Some(_) => return Err(invalid()),
    };
    let folder_id = match input.0.get("folderId") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(bound_id(value)?)),
    };
    Ok(UpdateSequence {
        name,
        active,
        folder_id,
    })
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    if matches!(
        path,
        "sequencesAPI.privateGetSequenceStepStatsAPI"
            | "sequencesAPI.privateListSequenceStepContactsAPI"
    ) {
        return Some(step_procedure(ctx, path, input).await);
    }
    if path != "sequencesAPI.listSequencesWorkspaceAuthAPI" {
        return None;
    }
    let input = In(input);
    Some(
        async {
            let (data, page_count) = sequence::list(
                &ctx.api.db,
                SequenceListInput {
                    workspace_id: ctx.workspace_id,
                    name: input.opt_str("name")?,
                    folder_id: input.opt_id("folderId")?.map(|id| id.to_string()),
                    active: input.opt_bool("active")?,
                    page: input.opt_positive("page")?,
                    per_page: input.opt_positive("perPage")?,
                    sort: input.sort(),
                },
            )
            .await
            .map_err(fail)?;
            Ok(page(
                data.iter().map(|row| {
                    let mut out = row.sequence.to_json();
                    out["stepsCount"] = json!(row.steps_count);
                    out["subscribersCount"] = json!(row.subscribers_count);
                    out
                }),
                page_count,
            ))
        }
        .await,
    )
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    if name == "getSequence" {
        return Some(get_sequence(ctx, args).await);
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            match name {
                // [workspaceId], { name, folderId }
                "createSequenceAction" => {
                    let input = In(arg(args, 1));
                    let sequence_name = input.str("name", true, 1, 255)?;
                    let folder_id = input.opt_id("folderId")?;
                    let id = sequence::create(db, ws, &sequence_name, folder_id)
                        .await
                        .map_err(name_exists)?;
                    ctx.api
                        .record_audit("create", &format!("created a new sequence (#{id})"))
                        .await;
                    Ok(json!({ "sequenceId": id.to_string() }))
                }
                // [workspaceId, id], data
                "updateSequenceAction" => {
                    let id = bound_id(arg(args, 1))?;
                    let data = update_data(&In(arg(args, 2)))?;
                    if let Some(detail) = sequence::update(db, ws, id, data)
                        .await
                        .map_err(name_exists)?
                    {
                        ctx.api.record_audit("update", &detail).await;
                    }
                    Ok(Value::Null)
                }
                // [workspaceId, id]
                "deleteSequenceAction" => {
                    let id = bound_id(arg(args, 1))?;
                    sequence::delete(db, ws, id).await.map_err(action_fail)?;
                    ctx.api
                        .record_audit("delete", &format!("deleted a sequence (#{id})"))
                        .await;
                    Ok(Value::Null)
                }
                // [workspaceId], upsertSequenceStepRequest
                "upsertSequenceStepAction" => {
                    let Value::Object(map) = arg(args, 1).clone() else {
                        return Err(invalid());
                    };
                    let mut input = Input::from_object(map);
                    let sequence_id = input.digits_id("sequenceId");
                    let (step_id, data) = crate::handlers::sequences::step_payload(&mut input);
                    input.finish().map_err(|_| invalid())?;
                    let sequence_id = sequence_id.unwrap_or_default();
                    sequence::assert_owned(db, ws, sequence_id)
                        .await
                        .map_err(action_fail)?;
                    let step_id = sequence::upsert_step(db, ws, sequence_id, step_id, data)
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({ "stepId": step_id.to_string() }))
                }
                // [workspaceId], { stepId, sequenceId }
                _ => {
                    let input = In(arg(args, 1));
                    let step_id = input.id("stepId")?;
                    let sequence_id = input.id("sequenceId")?;
                    sequence::assert_owned(db, ws, sequence_id)
                        .await
                        .map_err(action_fail)?;
                    sequence::delete_step(db, ws, step_id, Some(sequence_id))
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({ "success": true }))
                }
            }
        }
        .await,
    )
}
