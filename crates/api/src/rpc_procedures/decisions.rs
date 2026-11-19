//! `features/decision-connections` + `features/decision-profiles`: the AI
//! Router settings and profile screens of the SPA (`superAdmin` only, as
//! `requireWorkspacePermission(workspaceId, "superAdmin")`).

use chatbotx_business::BusinessError;
use chatbotx_business::decision::{self as service, ListProfiles};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, schema_errors};
use crate::handlers::decisions::{
    connection_fields, deps, enabled_field, ids_field, profile_form, strict, test_result_json,
};
use crate::handlers::input::Input;
use crate::orpc::OrpcError;

const ACTIONS: &[&str] = &[
    "listDecisionConnections",
    "listDecisionProfiles",
    "createDecisionConnectionAction",
    "updateDecisionConnectionAction",
    "toggleDecisionConnectionAction",
    "testDecisionConnectionAction",
    "createDecisionProfileAction",
    "updateDecisionProfileAction",
    "toggleDecisionProfileAction",
    "deleteDecisionProfilesAction",
];

const CONNECTION_KEYS: &[&str] = &[
    "credential",
    "defaultModel",
    "endpoint",
    "modelCatalog",
    "name",
    "providerKind",
];

fn forbidden() -> OrpcError {
    OrpcError::new("FORBIDDEN", Some(403), Some("Forbidden"))
}

/// `decisionProfilesAPI.listGlobalRouterDestinations` (`superAdmin`).
pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, _input: &Value) -> Outcome {
    if path != "decisionProfilesAPI.listGlobalRouterDestinations" {
        return None;
    }
    if !ctx.has("superAdmin") {
        return Some(Err(forbidden()));
    }
    Some(
        service::global_router_destinations(&ctx.api.db, ctx.workspace_id)
            .await
            .map_err(super::fail),
    )
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    if !ctx.has("superAdmin") {
        return Some(Err(forbidden()));
    }
    Some(run(ctx, name, args).await)
}

fn body(args: &[Value], index: usize) -> Input {
    Input::from_body(&[], arg(args, index).to_string().as_bytes())
}

/// The raw model of a profile row (`DecisionProfileModel`, ids as strings).
fn profile_model(row: &chatbotx_db::repo::models::DecisionProfile) -> Value {
    json!({
        "id": row.id.to_string(),
        "createdAt": crate::handlers::date(row.created_at),
        "updatedAt": crate::handlers::date(row.updated_at),
        "workspaceId": row.workspace_id.to_string(),
        "name": row.name,
        "description": row.description,
        "status": row.status,
        "connectionId": row.connection_id.to_string(),
        "model": row.model,
        "contract": row.contract.0,
        "thresholdConfig": row.threshold_config.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "globalRouterEnabled": row.global_router_enabled,
        "globalRouterConfig": row.global_router_config.as_ref().map_or(Value::Null, |j| j.0.clone()),
    })
}

fn checked(input: Input) -> Result<(), OrpcError> {
    input.finish().map_err(schema_errors)
}

fn id_of(input: &mut Input) -> Option<i64> {
    input.digits_id("id")
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let deps = deps(ctx.api);
    match name {
        "listDecisionConnections" => {
            let rows = service::list_connections(db, ws)
                .await
                .map_err(action_fail)?;
            Ok(Value::Array(
                rows.iter().map(service::safe_connection_json).collect(),
            ))
        }
        "listDecisionProfiles" => {
            let search = In(arg(args, 0));
            let sort = search
                .sort()
                .and_then(|items| items.into_iter().next())
                .map(|s| (s.id, s.desc));
            let (rows, page_count) = service::list_profiles(
                db,
                ws,
                ListProfiles {
                    name: search.opt_str("name")?,
                    page: search.opt_positive("page")?,
                    per_page: search.opt_positive("perPage")?,
                    sort,
                },
            )
            .await
            .map_err(action_fail)?;
            let data: Vec<Value> = rows
                .iter()
                .map(|row| {
                    let mut item = profile_model(&row.profile);
                    item["connectionName"] = row
                        .connection
                        .as_ref()
                        .map_or(Value::Null, |c| Value::from(c.name.clone()));
                    item["globalRouter"] = row.global_router();
                    item
                })
                .collect();
            Ok(json!({ "data": data, "pageCount": page_count }))
        }
        "createDecisionConnectionAction" | "updateDecisionConnectionAction" => {
            let create = name == "createDecisionConnectionAction";
            let mut input = body(args, 1);
            strict(
                &mut input,
                CONNECTION_KEYS,
                if create { &[] } else { &["id"] },
            );
            let fields = connection_fields(&mut input, false);
            let id = if create { None } else { id_of(&mut input) };
            checked(input)?;
            let row = if create {
                if fields.credential.as_deref().is_none_or(str::is_empty) {
                    return Err(action_fail(BusinessError::exception(
                        "Decision credential is required",
                        "systemError",
                        500,
                    )));
                }
                service::create_connection(&deps, ws, fields).await
            } else {
                service::update_connection(&deps, ws, id.unwrap_or_default(), fields).await
            }
            .map_err(action_fail)?;
            Ok(service::safe_connection_json(&row))
        }
        "toggleDecisionConnectionAction" | "toggleDecisionProfileAction" => {
            let mut input = body(args, 1);
            let enabled = enabled_field(&mut input).unwrap_or_default();
            let id = id_of(&mut input).unwrap_or_default();
            checked(input)?;
            if name == "toggleDecisionConnectionAction" {
                let row = service::set_connection_enabled(db, ws, id, enabled)
                    .await
                    .map_err(action_fail)?;
                Ok(service::safe_connection_json(&row))
            } else {
                let row = service::set_profile_enabled(db, ws, id, enabled)
                    .await
                    .map_err(action_fail)?;
                Ok(profile_model(&row))
            }
        }
        "testDecisionConnectionAction" => {
            let mut input = body(args, 1);
            strict(&mut input, &["id"], &[]);
            let id = id_of(&mut input).unwrap_or_default();
            checked(input)?;
            let result = service::test_connection(&deps, ws, id)
                .await
                .map_err(action_fail)?;
            Ok(test_result_json(&result))
        }
        "createDecisionProfileAction" | "updateDecisionProfileAction" => {
            let create = name == "createDecisionProfileAction";
            let mut input = body(args, 1);
            let form = profile_form(&mut input, if create { &[] } else { &["id"] });
            let id = if create { None } else { id_of(&mut input) };
            checked(input)?;
            let form = form.ok_or_else(super::invalid)?;
            let row = if create {
                service::create_profile(db, ws, form).await
            } else {
                service::update_profile(db, ws, id.unwrap_or_default(), form).await
            }
            .map_err(action_fail)?;
            Ok(profile_model(&row))
        }
        "deleteDecisionProfilesAction" => {
            let mut input = body(args, 1);
            let ids = ids_field(&mut input);
            checked(input)?;
            service::delete_profiles(db, ws, ids)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        _ => Err(super::invalid()),
    }
}
