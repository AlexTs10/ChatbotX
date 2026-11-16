//! The workspace member procedures of the builder (Q.D11 slice 2):
//!
//! - `workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI`
//!   (`features/workspace-members/api/private.ts`)
//! - the actions of `features/workspace-members/actions/*` under the `/rpc`
//!   key `workspaceMemberActions`: `inviteWorkspaceMember`,
//!   `updateWorkspaceMember`, `deleteWorkspaceMember` (input
//!   `{ workspaceId, id?, ...input }`; `id` = the member, the second bind arg).

use chatbotx_business::BusinessError;
use chatbotx_business::workspace::PERMISSION_KEYS;
use chatbotx_business::workspace_member::{
    self, MemberUpdate, full_permissions, normalize_contacts_permissions,
};
use chatbotx_business::{invitation, user_quota, workspace};
use chatbotx_rt::{RevokeArgs, RevokeReason};
use serde_json::{Map, Value, json};

use super::RpcContext;
use super::actions::{ActionContext, ActionError, ActionResult, Gate, exception};
use super::workspace::RpcOut;
use crate::handlers::input::{Input, Path, child, key};
use crate::handlers::{ApiState, LISTING, business_error};
use crate::orpc::OrpcError;

/// The procedures of this module.
pub const PORTED: &[&str] = &["workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI"];

const NOTIFICATION_TYPE_KEYS: &[&str] = &["notifyAdmin", "newMessageToHuman", "newOrder"];
const NOTIFICATION_CHANNEL_KEYS: &[&str] = &["messenger", "email", "telegram", "browser"];

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `z.object({ <keys>: z.boolean() })` at `path` (unknown keys stripped).
fn bool_object(
    input: &mut Input,
    path: &Path,
    value: Option<&Value>,
    keys: &[&str],
) -> Option<Value> {
    let Some(Value::Object(object)) = value else {
        let received = value.map_or("undefined", type_name);
        input.issue(
            path,
            "invalid_type",
            &format!("Invalid input: expected object, received {received}"),
        );
        return None;
    };
    let mut out = Map::new();
    let mut ok = true;
    for name in keys {
        match input.bool_at(&child(path, name), object.get(*name), true) {
            Some(flag) => {
                out.insert((*name).to_string(), Value::Bool(flag));
            }
            None => ok = false,
        }
    }
    ok.then_some(Value::Object(out))
}

/// `workspaceMemberPermissionsRequest`: every flag, at least one set.
fn permissions_request(input: &mut Input) -> Option<Value> {
    let value = input.get("permissions").cloned();
    let path = key("permissions");
    let permissions = bool_object(input, &path, value.as_ref(), PERMISSION_KEYS)?;
    let any = permissions
        .as_object()
        .is_some_and(|o| o.values().any(|v| v == &Value::Bool(true)));
    if !any {
        input.issue(
            &child(&path, "permissions"),
            "custom",
            "At least one permission must be selected.",
        );
        return None;
    }
    Some(permissions)
}

/// The member id (second bind arg, `zodBigintAsString`).
fn member_id(raw: &Value) -> Result<i64, ActionError> {
    raw.get("id")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| ActionError::Internal("invalid member id bind arg".into()))
}

/// The actions of the `workspaceMemberActions` key and their clients.
pub fn action_gate(name: &str) -> Option<Gate> {
    Some(match name {
        "inviteWorkspaceMember" | "updateWorkspaceMember" => Gate::Workspace,
        "deleteWorkspaceMember" => Gate::AllowExpired,
        _ => return None,
    })
}

/// `tryRevokeWorkspaceMemberRealtimeConnections`; `true` = revoked (or no
/// realtime gateway to revoke on).
async fn try_revoke(
    api: &ApiState,
    workspace_id: i64,
    user_id: i64,
    reason: RevokeReason,
    message: &str,
) -> bool {
    let Some(realtime) = &api.realtime else {
        return true;
    };
    realtime
        .try_revoke_member(
            RevokeArgs {
                workspace_id: workspace_id.to_string(),
                user_id: user_id.to_string(),
                reason,
                revoked_at_ms: None,
            },
            message,
        )
        .await
}

fn role_label(permissions: &Value) -> &'static str {
    if permissions.get("superAdmin") == Some(&Value::Bool(true)) {
        "admin"
    } else {
        "member"
    }
}

/// Runs one member action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let ws = context.workspace_id();
    let db = &api.db;
    let edition = api.edition;
    let input_of =
        |value: &Value| Input::from_object(value.as_object().cloned().unwrap_or_default());
    match name {
        "inviteWorkspaceMember" => {
            let mut input = input_of(raw);
            let permissions = permissions_request(&mut input);
            input.finish()?;
            let workspace = workspace::find_by_id(db, ws).await?;
            if !context.has_permission("superAdmin") {
                return Err(exception(
                    "You are not authorized to invite a workspace member. You need to be a super admin to do this.",
                ));
            }
            let owner = workspace.owner_id;
            let at_limit = db
                .read(move |conn| {
                    user_quota::team_member_limit_reached(conn, owner)
                        .map_err(|e| chatbotx_db::DbError::other(e.message()))
                })
                .await
                .map_err(BusinessError::from)?;
            if at_limit {
                return Err(exception(
                    "Team member limit reached for this workspace plan",
                ));
            }
            let permissions = if edition.is_community() {
                full_permissions()
            } else {
                normalize_contacts_permissions(&permissions.unwrap_or_default())
            };
            let label = role_label(&permissions);
            let created = invitation::create(db, ws, context.user_id, permissions).await?;
            api.record_audit("invite", &format!("invited a new {label}"))
                .await;
            Ok(Some(json!({
                "id": created.id.to_string(),
                "code": created.code,
                "inviteUrl": invitation::build_invite_url(&api.app_url, &created.code),
            })))
        }
        "updateWorkspaceMember" => {
            let mut input = input_of(raw);
            let permissions = permissions_request(&mut input);
            let types = {
                let v = input.get("notificationTypes").cloned();
                bool_object(
                    &mut input,
                    &key("notificationTypes"),
                    v.as_ref(),
                    NOTIFICATION_TYPE_KEYS,
                )
            };
            let channels = {
                let v = input.get("notificationChannels").cloned();
                bool_object(
                    &mut input,
                    &key("notificationChannels"),
                    v.as_ref(),
                    NOTIFICATION_CHANNEL_KEYS,
                )
            };
            input.finish()?;
            let id = member_id(raw)?;
            let member = workspace_member::find_by_id_or_fail(db, ws, id).await?;
            if !context.has_permission("superAdmin") {
                return Err(exception(
                    "You are not authorized to update this workspace member. You need to be a super admin to do this.",
                ));
            }
            let permissions = if edition.is_community() {
                full_permissions()
            } else {
                normalize_contacts_permissions(&permissions.unwrap_or_default())
            };
            let data = MemberUpdate {
                permissions,
                notification_types: types.unwrap_or_default(),
                notification_channels: channels.unwrap_or_default(),
            };
            let permissions_changed = member.permissions.0 != data.permissions;
            let notifications_changed = member.notification_types.0 != data.notification_types
                || member.notification_channels.0 != data.notification_channels;
            if !(permissions_changed || notifications_changed) {
                return Ok(Some(json!({ "revokeWarning": false })));
            }
            let label = role_label(&data.permissions);
            let Some(user_id) = workspace_member::update(db, ws, member.id, data).await? else {
                return Ok(Some(json!({ "revokeWarning": false })));
            };
            let revoke_warning = permissions_changed
                && !try_revoke(
                    api,
                    ws,
                    user_id,
                    RevokeReason::Reauth,
                    "Failed to revoke workspace member realtime connections",
                )
                .await;
            if permissions_changed {
                let target = member.user_id;
                let who = db
                    .read(move |conn| {
                        use chatbotx_db::rusqlite::OptionalExtension as _;
                        Ok(conn
                            .query_row(
                                r#"SELECT COALESCE("name", "email") FROM "User" WHERE "id" = ?1"#,
                                [target],
                                |row| row.get::<_, String>(0),
                            )
                            .optional()?)
                    })
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "a member".to_string());
                api.record_audit("role_change", &format!("changed role of {who} to {label}"))
                    .await;
            }
            Ok(Some(json!({ "revokeWarning": revoke_warning })))
        }
        "deleteWorkspaceMember" => {
            let id = member_id(raw)?;
            let member = workspace_member::find_by_id_or_fail(db, ws, id).await?;
            if member.role == "owner" {
                return Err(exception("You cannot delete the owner of the workspace"));
            }
            if !context.has_permission("superAdmin") {
                return Err(exception(
                    "You are not authorized to delete this workspace member. You need to be a super admin to do this.",
                ));
            }
            let Some(removed) = workspace_member::delete(db, ws, id).await? else {
                return Ok(Some(json!({ "revokeWarning": false })));
            };
            api.record_audit(
                "delete",
                &format!("removed {} from workspace", removed.label),
            )
            .await;
            let revoke_warning = !try_revoke(
                api,
                ws,
                removed.user_id,
                RevokeReason::Deleted,
                "Failed to revoke removed member realtime connections",
            )
            .await;
            Ok(Some(json!({ "revokeWarning": revoke_warning })))
        }
        _ => Err(ActionError::Internal(format!(
            "unknown member action {name}"
        ))),
    }
}

/// `z.number().int().positive()` (nullish = `None`).
fn opt_positive_int(input: &Value, name: &str) -> Result<Option<i64>, ()> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if v.fract() == 0.0 && v > 0.0 => Ok(Some(v as i64)),
            _ => Err(()),
        },
        Some(_) => Err(()),
    }
}

/// Runs a procedure of [`PORTED`]; `None` = not this module's.
pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcContext,
    input: &Value,
) -> Option<Result<RpcOut, OrpcError>> {
    if path != PORTED[0] {
        return None;
    }
    let workspace_id = context.workspace.as_ref()?.workspace_id;
    let (Ok(page), Ok(per_page)) = (
        opt_positive_int(input, "page"),
        opt_positive_int(input, "perPage"),
    ) else {
        let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
        error.defined = false;
        return Some(Err(error));
    };
    let keyword = input
        .get("keyword")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(
        workspace_member::list_paginated(&api.db, workspace_id, page, per_page, keyword)
            .await
            .map(|page| {
                let mut meta = Vec::new();
                let data: Vec<Value> = page
                    .data
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        for (field, set) in [
                            ("createdAt", true),
                            ("updatedAt", true),
                            ("onlineSince", row.online_since.is_some()),
                        ] {
                            if set {
                                meta.push(json!([1, "data", i, field]));
                            }
                        }
                        crate::handlers::workspace_members::member_json(row)
                    })
                    .collect();
                (json!({ "data": data, "pageCount": page.page_count }), meta)
            })
            .map_err(|e| business_error(e, LISTING)),
    )
}
