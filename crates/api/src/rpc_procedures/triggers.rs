//! Triggers: `features/triggers/actions/*` (Q.D5). The builder's trigger
//! pages read through server components (`queries/index.ts`), not `/rpc`.

use chatbotx_business::BusinessError;
use chatbotx_business::trigger as service;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::handlers::input::Input;
use crate::handlers::triggers::{condition_inputs, parse_update};
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createTriggerAction",
    "deleteTriggersAction",
    "updateTriggerAction",
    "updateTriggerSettingsAction",
    // The list / edit page queries (`features/triggers/queries`).
    "getTriggers",
    "triggerEditPage",
];

/// The page queries: the pages are `superAdmin` only (`(has-folder)/triggers`
/// layout, `requireWorkspacePermission` of the edit page): 404 otherwise.
async fn query(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    if !ctx.has("superAdmin") {
        return Err(fail(BusinessError::not_found("Page not found")));
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input = In(arg(args, 1));
    if name == "triggerEditPage" {
        // `findTrigger` + `ecommerceSettingsService.storeSource`; no trigger = `null`.
        let Ok(id) = input.id("id") else {
            return Ok(Value::Null);
        };
        let Some(found) = service::find(db, ws, id).await.map_err(fail)? else {
            return Ok(Value::Null);
        };
        let store_source = chatbotx_business::tx::read(db, move |conn| {
            chatbotx_business::store_source::resolve_store_source(conn, ws)
        })
        .await
        .map_err(fail)?;
        return Ok(json!({
            "trigger": service::trigger_json(&found),
            "storeSource": store_source,
        }));
    }
    // `getTriggers`: `folderId` null = the top level (no folder).
    let (data, page_count) = service::list(
        db,
        ws,
        Some(input.opt_id("folderId")?),
        input.opt_str("name")?.filter(|n| !n.is_empty()),
        input.opt_positive("page")?.unwrap_or(1),
        input.opt_positive("perPage")?.unwrap_or(10),
    )
    .await
    .map_err(fail)?;
    Ok(super::page(
        data.iter().map(service::trigger_json),
        page_count,
    ))
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

/// `t(error.message, error.data)` of `createTriggerAction` (English copy).
fn localized(error: BusinessError) -> OrpcError {
    if let BusinessError::Exception {
        code,
        message,
        data: Some(data),
        ..
    } = &error
        && code == "validation"
        && message == "validation.maxItemsReached"
    {
        let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
        let text = format!(
            "Maximum {} {} allowed per workspace",
            text(&data["max"]),
            text(&data["feature"]),
        );
        return fail(BusinessError::exception(text, "systemError", 400));
    }
    action_fail(error)
}

fn object(value: &Value) -> Result<Input, OrpcError> {
    match value {
        Value::Object(map) => Ok(Input::from_object(map.clone())),
        _ => Err(invalid()),
    }
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    if matches!(name, "getTriggers" | "triggerEditPage") {
        return Some(query(ctx, name, args).await);
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            match name {
                // [workspaceId], { name, folderId }
                "createTriggerAction" => {
                    let input = In(arg(args, 1));
                    let trigger_name = input.str("name", false, 1, usize::MAX)?;
                    // `zodBigintAsString().nullable()`: the key is required.
                    if input.0.get("folderId").is_none() {
                        return Err(invalid());
                    }
                    let folder_id = input.opt_id("folderId")?;
                    let created = service::create(db, ws, trigger_name, folder_id)
                        .await
                        .map_err(localized)?;
                    ctx.api
                        .record_audit("create", &format!("created a new trigger (#{})", created.id))
                        .await;
                    Ok(created.to_json())
                }
                // [workspaceId], { ids }
                "deleteTriggersAction" => {
                    let ids = In(arg(args, 1)).ids("ids")?;
                    let deleted = service::delete_many(db, ws, ids).await.map_err(action_fail)?;
                    if let Some(detail) = service::delete_audit_detail(&deleted) {
                        ctx.api.record_audit("delete", &detail).await;
                    }
                    Ok(Value::Null)
                }
                // [workspaceId, id], { conditions, actions }
                "updateTriggerAction" => {
                    let id = bound_id(arg(args, 1))?;
                    let mut input = object(arg(args, 2))?;
                    let parsed = parse_update(&mut input);
                    input.finish().map_err(|_| invalid())?;
                    let parsed = parsed.unwrap_or(Value::Null);
                    let actions = parsed["actions"].as_array().cloned().unwrap_or_default();
                    let updated = service::update_with_conditions(db, ws, id, actions, condition_inputs(&parsed))
                        .await
                        .map_err(action_fail)?;
                    let Some((found, changed)) = updated else {
                        return Ok(Value::Null);
                    };
                    if changed {
                        ctx.api
                            .record_audit("update", &format!("updated a trigger (#{id})"))
                            .await;
                    }
                    Ok(json!({
                        "trigger": found.trigger.to_json(),
                        "conditions": found.conditions.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
                    }))
                }
                // [workspaceId, id], { name?, active? }
                _ => {
                    let id = bound_id(arg(args, 1))?;
                    let input = In(arg(args, 2));
                    let trigger_name = match input.0.get("name") {
                        None => None,
                        Some(_) => Some(input.str("name", true, 1, 255)?),
                    };
                    let active = input.opt_bool("active")?;
                    let (_, detail) = service::update_settings(db, ws, id, trigger_name, active)
                        .await
                        .map_err(action_fail)?;
                    if let Some(detail) = detail {
                        ctx.api.record_audit("update", &detail).await;
                    }
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}
