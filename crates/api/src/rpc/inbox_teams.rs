//! The inbox team procedure and actions of the builder (Q.D11 slice 10):
//!
//! - `inboxTeamsAPI.listInboxTeamsAuthenticatedAPI`
//!   (`enterprise/features/inbox-teams/api/private.ts`)
//! - the actions of `enterprise/features/inbox-teams/actions/*` under the
//!   `/rpc` key `inboxTeamActions`: `createInboxTeam`, `updateInboxTeam`,
//!   `deleteInboxTeam`, `addInboxTeamMember`, `deleteTeamMembers` (input
//!   `{ workspaceId, id?, ...input }`; `id` = the team, the second bind arg).

use chatbotx_business::inbox_team as service;
use serde_json::{Value, json};

use super::RpcContext;
use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::workspace::RpcOut;
use crate::handlers::inbox_teams::{
    TEAM_NAME, after_mutation, team_json, team_with_members_json, user_ids,
};
use crate::handlers::input::Input;
use crate::handlers::tags::{array, digits_at};
use crate::handlers::{ApiState, LISTING, business_error};
use crate::orpc::OrpcError;

/// The procedures of this module.
pub const PORTED: &[&str] = &["inboxTeamsAPI.listInboxTeamsAuthenticatedAPI"];

/// The actions of the `inboxTeamActions` key: all `workspaceActionClient`.
pub fn action_gate(name: &str) -> Option<Gate> {
    matches!(
        name,
        "createInboxTeam"
            | "updateInboxTeam"
            | "deleteInboxTeam"
            | "addInboxTeamMember"
            | "deleteTeamMembers"
    )
    .then_some(Gate::Workspace)
}

/// The team id (second bind arg, `zodBigintAsString`).
fn team_id(raw: &Value) -> Result<i64, ActionError> {
    raw.get("id")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| ActionError::Internal("invalid inbox team id bind arg".into()))
}

/// Runs one inbox team action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let ws = context.workspace_id();
    let db = &api.db;
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    match name {
        "createInboxTeam" => {
            let name = input.string("name", TEAM_NAME);
            let users = user_ids(&mut input);
            input.finish()?;
            let out = service::create(db, ws, name.unwrap_or_default(), users.unwrap_or_default())
                .await?;
            let audit = out.audit.as_ref().map(|(a, d)| (*a, d.as_str()));
            after_mutation(api, ws, &out.revoke_user_ids, audit).await;
            Ok(None)
        }
        "updateInboxTeam" => {
            let name = input.opt_string("name", TEAM_NAME);
            input.finish()?;
            let out = service::update(db, ws, team_id(raw)?, name).await?;
            let audit = out.audit.as_ref().map(|(a, d)| (*a, d.as_str()));
            after_mutation(api, ws, &[], audit).await;
            Ok(Some(team_json(&out.team)))
        }
        "deleteInboxTeam" => {
            let ids = array(&mut input, "ids", 0, usize::MAX, |input, path, value| {
                digits_at(input, path, Some(value), true)
            });
            input.finish()?;
            let (revoke, detail) = service::delete(db, ws, ids.unwrap_or_default()).await?;
            after_mutation(api, ws, &revoke, Some(("delete", detail.as_str()))).await;
            Ok(None)
        }
        "addInboxTeamMember" | "deleteTeamMembers" => {
            let users = user_ids(&mut input);
            input.finish()?;
            let (team, users) = (team_id(raw)?, users.unwrap_or_default());
            let out = if name == "addInboxTeamMember" {
                service::add_members(db, ws, team, users).await?
            } else {
                service::remove_members(db, ws, team, users).await?
            };
            let audit = out.audit.as_ref().map(|(a, d)| (*a, d.as_str()));
            after_mutation(api, ws, &out.revoke_user_ids, audit).await;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!(
            "unknown inbox team action {name}"
        ))),
    }
}

/// Runs a procedure of [`PORTED`]; `None` = not this module's. The dates are
/// marked as `Date` in the meta.
pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcContext,
) -> Option<Result<RpcOut, OrpcError>> {
    if path != PORTED[0] {
        return None;
    }
    let workspace_id = context.workspace.as_ref()?.workspace_id;
    Some(
        service::list_by_workspace(&api.db, workspace_id)
            .await
            .map(|teams| {
                let mut meta = Vec::new();
                for (i, team) in teams.iter().enumerate() {
                    for field in ["createdAt", "updatedAt"] {
                        meta.push(json!([1, "data", i, field]));
                    }
                    for j in 0..team.members.len() {
                        for field in ["createdAt", "updatedAt"] {
                            meta.push(json!([1, "data", i, "inboxTeamMembers", j, field]));
                            meta.push(json!([1, "data", i, "inboxTeamMembers", j, "user", field]));
                        }
                    }
                }
                let data: Vec<Value> = teams.iter().map(team_with_members_json).collect();
                (json!({ "data": data }), meta)
            })
            .map_err(|e| business_error(e, LISTING)),
    )
}
