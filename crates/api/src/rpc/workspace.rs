//! The workspace procedures of the builder (Q.D11):
//!
//! - `workspacesAPI.listMyWorkspacesAPI`, `workspacesAPI.listDeletedWorkspacesAPI`
//!   (`features/workspaces/api/private.ts`)
//! - the server actions of `features/workspaces/actions/*` under the
//!   `/rpc` key `workspaceActions` (name = the export without `Action`).

use chatbotx_business::BusinessError;
use chatbotx_business::workspace::{self, SettingsPatch, codes};
use chatbotx_business::workspace_api_token::{self, credentials::generate_workspace_token};
use chatbotx_db::repo::models::{Workspace, WorkspaceDeletionRecord};
use chatbotx_db::repo::workspace_api_token::{
    InsertWorkspaceApiTokenInput, WORKSPACE_API_TOKEN_PERMISSIONS, WORKSPACE_API_TOKEN_SCOPES,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::{Value, json};

use super::RpcContext;
use super::actions::{ActionContext, ActionError, ActionResult, Gate, exception};
use crate::handlers::input::{Input, Str, key};
use crate::handlers::workspace_settings::{
    DEFAULT_REPLY_FREQUENCIES, SMART_RESPONSE_DELAY_OPTIONS, after_write,
};
use crate::handlers::{ApiState, LISTING, business_error};
use crate::orpc::OrpcError;

/// The procedures of this module.
pub const PORTED: &[&str] = &[
    "workspacesAPI.listMyWorkspacesAPI",
    "workspacesAPI.listDeletedWorkspacesAPI",
];

/// An RPC output and its meta (`[1, ...path]` = a Date).
pub type RpcOut = (Value, Vec<Value>);

/// `errors.superAdminRequired` (en).
/// `SUPPORT_ACCESS_WINDOW_DAYS` (7 days).
const SUPPORT_ACCESS_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";
/// `workspace.deletion.alreadyStarted` (en).
const DELETION_ALREADY_STARTED: &str = "Workspace deletion has already started.";

fn date(ms: i64, path: Vec<Value>, meta: &mut Vec<Value>) -> Value {
    let mut entry = vec![json!(1)];
    entry.extend(path);
    meta.push(Value::Array(entry));
    Value::String(chatbotx_db::repo::iso(ms))
}

fn opt_date(ms: Option<i64>, path: Vec<Value>, meta: &mut Vec<Value>) -> Value {
    ms.map_or(Value::Null, |ms| date(ms, path, meta))
}

fn at(prefix: &[Value], name: &str) -> Vec<Value> {
    let mut path = prefix.to_vec();
    path.push(json!(name));
    path
}

/// `workspaceResource` (every column but `token`).
pub fn workspace_resource(w: &Workspace, prefix: &[Value], meta: &mut Vec<Value>) -> Value {
    json!({
        "id": w.id.to_string(),
        "createdAt": date(w.created_at, at(prefix, "createdAt"), meta),
        "updatedAt": date(w.updated_at, at(prefix, "updatedAt"), meta),
        "name": w.name,
        "defaultReply": w.default_reply,
        "defaultReplyFrequency": w.default_reply_frequency,
        "targetCountry": w.target_country,
        "language": w.language,
        "timezone": w.timezone,
        "brandColor": w.brand_color,
        "developmentMode": w.development_mode,
        "smartResponseDelaySeconds": w.smart_response_delay_seconds,
        "isActive": w.is_active,
        "startTime": w.start_time,
        "endTime": w.end_time,
        "logo": w.logo,
        "scheduledDeletionAt": opt_date(w.scheduled_deletion_at, at(prefix, "scheduledDeletionAt"), meta),
        "deletionRequestedById": w.deletion_requested_by_id.map(|id| id.to_string()),
        "deletionRequestedAt": opt_date(w.deletion_requested_at, at(prefix, "deletionRequestedAt"), meta),
        "purgeStartedAt": opt_date(w.purge_started_at, at(prefix, "purgeStartedAt"), meta),
        "supportAccessUntil": opt_date(w.support_access_until, at(prefix, "supportAccessUntil"), meta),
        "capiLimitedDataUse": w.capi_limited_data_use,
        "ownerId": w.owner_id.to_string(),
        "tenantId": w.tenant_id.to_string(),
    })
}

/// `workspaceDeletionRecordResource`.
fn deletion_record(r: &WorkspaceDeletionRecord, prefix: &[Value], meta: &mut Vec<Value>) -> Value {
    json!({
        "id": r.id.to_string(),
        "workspaceId": r.workspace_id.to_string(),
        "workspaceName": r.workspace_name,
        "ownerId": r.owner_id.to_string(),
        "ownerEmail": r.owner_email,
        "tenantId": r.tenant_id.to_string(),
        "deletionRequestedById": r.deletion_requested_by_id.map(|id| id.to_string()),
        "deletionRequestedByEmail": r.deletion_requested_by_email,
        "deletionRequestedAt": opt_date(r.deletion_requested_at, at(prefix, "deletionRequestedAt"), meta),
        "scheduledDeletionAt": opt_date(r.scheduled_deletion_at, at(prefix, "scheduledDeletionAt"), meta),
        "purgedAt": opt_date(r.purged_at, at(prefix, "purgedAt"), meta),
        "details": r.details.0,
        "usageSnapshot": r.usage_snapshot.0,
        "createdAt": date(r.created_at, at(prefix, "createdAt"), meta),
        "updatedAt": date(r.updated_at, at(prefix, "updatedAt"), meta),
    })
}

fn validation_failed() -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error
}

/// `z.number().int().positive()` with a default (`max` when given).
fn positive_int(input: &Value, name: &str, default: i64, max: Option<i64>) -> Option<i64> {
    match input.get(name) {
        None | Some(Value::Null) => Some(default),
        Some(Value::Number(n)) => {
            let value = n.as_f64()?;
            let ok = value.fract() == 0.0 && value > 0.0 && max.is_none_or(|m| value <= m as f64);
            ok.then_some(value as i64)
        }
        Some(_) => None,
    }
}

/// Runs a procedure of [`PORTED`]; `None` = not this module's.
pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcContext,
    input: &Value,
) -> Option<Result<RpcOut, OrpcError>> {
    let user_id = context.session.user_id;
    Some(match path {
        "workspacesAPI.listMyWorkspacesAPI" => workspace::list_for_member(&api.db, user_id)
            .await
            .map(|rows| {
                let mut meta = Vec::new();
                let list: Vec<Value> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        workspace_resource(w, &[json!("workspaces"), json!(i)], &mut meta)
                    })
                    .collect();
                (json!({ "workspaces": list }), meta)
            })
            .map_err(|e| business_error(e, LISTING)),
        "workspacesAPI.listDeletedWorkspacesAPI" => {
            let (Some(page), Some(per_page)) = (
                positive_int(input, "page", 1, None),
                positive_int(input, "perPage", 20, Some(100)),
            ) else {
                return Some(Err(validation_failed()));
            };
            let offset = (page - 1) * per_page;
            api.db
                .read(move |conn| {
                    chatbotx_db::repo::workspace_deletion_record::list_for_owner(
                        conn, user_id, per_page, offset,
                    )
                    .map_err(|e| chatbotx_db::DbError::other(e.message()))
                })
                .await
                .map(|(rows, total)| {
                    let mut meta = Vec::new();
                    let data: Vec<Value> = rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| deletion_record(r, &[json!("data"), json!(i)], &mut meta))
                        .collect();
                    let page_count = (total + per_page - 1) / per_page;
                    (json!({ "data": data, "pageCount": page_count }), meta)
                })
                .map_err(|e| business_error(BusinessError::from(e), LISTING))
        }
        _ => return None,
    })
}

/// The actions of the `workspaceActions` key and their clients.
pub fn action_gate(name: &str) -> Option<Gate> {
    Some(match name {
        "updateWorkspaceBasic" | "updateWorkspaceAdvanced" | "toggleSupportAccess" => {
            Gate::AllowScheduledDeletion
        }
        "updateSmartResponseDelay"
        | "updateWorkspaceStatus"
        | "createWorkspaceToken"
        | "deleteWorkspaceToken"
        | "deleteErrorLog" => Gate::Workspace,
        "scheduleWorkspaceDeletion" | "cancelWorkspaceDeletion" => Gate::AllowExpired,
        _ => return None,
    })
}

/// `bulkUpdateIdsRequest`: `ids: z.array(zodBigintAsString())`.
fn ids_request(input: &mut Input) -> Option<Vec<i64>> {
    let path = key("ids");
    let value = input.get("ids").cloned();
    let Some(Value::Array(items)) = value else {
        let received = match value {
            None => "undefined",
            Some(Value::Null) => "null",
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(_) => "object",
        };
        input.issue(
            &path,
            "invalid_type",
            &format!("Invalid input: expected array, received {received}"),
        );
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let mut at = path.clone();
        at.push(crate::handlers::input::Seg::Index(index));
        match input.id_at(&at, Some(item), true) {
            Some(id) => out.push(id),
            None => ok = false,
        }
    }
    ok.then_some(out)
}

/// `HH:mm` (`/^([01]\d|2[0-3]):[0-5]\d$/`).
fn is_time(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && matches!(
            (b[0], b[1]),
            (b'0' | b'1', b'0'..=b'9') | (b'2', b'0'..=b'3')
        )
        && matches!(b[3], b'0'..=b'5')
        && b[4].is_ascii_digit()
}

/// A nullable `HH:mm` (`timeFormat.nullable()`, required key).
fn nullable_time(input: &mut Input, name: &str) -> Option<Option<String>> {
    match input.get(name).cloned() {
        Some(Value::Null) => Some(None),
        value => {
            let path = key(name);
            let text = input.string_at(&path, value.as_ref(), Str::default(), true)?;
            if is_time(&text) {
                Some(Some(text))
            } else {
                input.issue(&path, "invalid_format", "Invalid time format");
                None
            }
        }
    }
}

/// `z.array(workspaceApiTokenScopes)`.
fn scopes_request(input: &mut Input) -> Option<Vec<String>> {
    let path = key("scopes");
    match input.get("scopes").cloned() {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let mut at = path.clone();
                at.push(crate::handlers::input::Seg::Index(i));
                out.push(input.enum_at(&at, Some(item), WORKSPACE_API_TOKEN_SCOPES, true));
            }
            out.into_iter().collect()
        }
        other => {
            let received = match other {
                None => "undefined",
                Some(Value::Null) => "null",
                Some(Value::String(_)) => "string",
                Some(Value::Number(_)) => "number",
                Some(Value::Bool(_)) => "boolean",
                Some(_) => "object",
            };
            input.issue(
                &path,
                "invalid_type",
                &format!("Invalid input: expected array, received {received}"),
            );
            None
        }
    }
}

/// `requireWorkspaceTokenSuperAdmin`.
fn require_token_super_admin(context: &ActionContext) -> Result<(), ActionError> {
    if context.has_permission("superAdmin") {
        Ok(())
    } else {
        Err(exception(
            "You need to be a super admin to manage workspace API tokens",
        ))
    }
}

fn input_of(value: &Value) -> Input {
    Input::from_object(value.as_object().cloned().unwrap_or_default())
}

/// Runs one workspace action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let ws = context.workspace_id();
    let db = &api.db;
    match name {
        // `updateWorkspaceBasicRequest`: `name` 1..255, `logo` nullish.
        "updateWorkspaceBasic" => {
            let mut input = input_of(raw);
            let name = input.string(
                "name",
                Str {
                    min: Some(1),
                    max: Some(255),
                    ..Str::default()
                },
            );
            let logo = input.nullish_string("logo", Str::default());
            input.finish()?;
            let mut patch = vec![("name", Sql::Text(name.unwrap_or_default()))];
            if let Some(logo) = logo {
                patch.push(("logo", logo.map_or(Sql::Null, Sql::Text)));
            }
            let write = workspace::update(db, ws, patch).await?;
            after_write(api, &write).await;
            Ok(None)
        }
        // `updateWorkspaceAdvancedRequest` -> `updateSettings` (an empty or
        // missing Default Reply clears it).
        "updateWorkspaceAdvanced" => {
            let mut input = input_of(raw);
            let default_reply = input.nullish_string("defaultReply", Str::default());
            let frequency = {
                let v = input.get("defaultReplyFrequency").cloned();
                input.enum_at(
                    &key("defaultReplyFrequency"),
                    v.as_ref(),
                    DEFAULT_REPLY_FREQUENCIES,
                    false,
                )
            };
            let enum_req = |input: &mut Input, name: &str, ok: &dyn Fn(&str) -> bool| {
                let v = input.get(name).cloned();
                match v {
                    Some(Value::String(text)) if ok(&text) => Some(text),
                    _ => {
                        input.issue(&key(name), "invalid_value", "Invalid option");
                        None
                    }
                }
            };
            let target_country = enum_req(&mut input, "targetCountry", &codes::is_country_code);
            let language = enum_req(&mut input, "language", &|t| {
                codes::LANGUAGE_CODES.contains(&t)
            });
            let timezone = enum_req(&mut input, "timezone", &codes::is_timezone_code);
            let brand_color = input.string(
                "brandColor",
                Str {
                    pattern: Some(|t| {
                        t.len() == 7
                            && t.starts_with('#')
                            && t[1..].bytes().all(|b| b.is_ascii_hexdigit())
                    }),
                    ..Str::default()
                },
            );
            let development_mode = input.bool("developmentMode");
            let capi = input.opt_bool("capiLimitedDataUse");
            input.finish()?;
            // A non-numeric id matches no flow.
            let default_reply = default_reply
                .flatten()
                .filter(|t| !t.is_empty())
                .map(|text| text.parse::<i64>().unwrap_or(-1));
            let write = workspace::update_settings(
                db,
                ws,
                SettingsPatch {
                    default_reply: Some(default_reply),
                    default_reply_frequency: frequency,
                    capi_limited_data_use: capi,
                    target_country,
                    language,
                    timezone,
                    brand_color,
                    development_mode,
                    ..SettingsPatch::default()
                },
            )
            .await?;
            after_write(api, &write).await;
            Ok(None)
        }
        // `updateSmartResponseDelayRequest`: "none" / null / "" -> no delay.
        "updateSmartResponseDelay" => {
            let seconds = match raw.get("smartResponseDelaySeconds") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) if text.is_empty() || text == "none" => None,
                Some(value @ (Value::String(_) | Value::Number(_))) => {
                    let number = match value {
                        Value::String(text) => text.trim().parse::<f64>().ok(),
                        other => other.as_f64(),
                    };
                    match number.filter(|n| {
                        n.fract() == 0.0 && SMART_RESPONSE_DELAY_OPTIONS.contains(&(*n as i64))
                    }) {
                        Some(n) => Some(n as i64),
                        None => {
                            return Err(ActionError::Validation(vec![json!({
                                "code": "custom",
                                "path": ["smartResponseDelaySeconds"],
                                "message": "Invalid smart response delay",
                            })]));
                        }
                    }
                }
                Some(_) => {
                    return Err(ActionError::Validation(vec![json!({
                        "code": "invalid_union",
                        "path": ["smartResponseDelaySeconds"],
                        "message": "Invalid input",
                    })]));
                }
            };
            let write = workspace::update_settings(
                db,
                ws,
                SettingsPatch {
                    smart_response_delay_seconds: Some(seconds),
                    ..SettingsPatch::default()
                },
            )
            .await?;
            after_write(api, &write).await;
            Ok(None)
        }
        // `updateWorkspaceStatusRequest` + super admin.
        "updateWorkspaceStatus" => {
            let mut input = input_of(raw);
            let is_active = input.bool("isActive");
            let start = nullable_time(&mut input, "startTime");
            let end = nullable_time(&mut input, "endTime");
            if !input.failed()
                && start.as_ref().map(Option::is_none) != end.as_ref().map(Option::is_none)
            {
                input.issue(
                    &key("endTime"),
                    "custom",
                    "startTime and endTime must both be set or both be null",
                );
            }
            input.finish()?;
            if !context.has_permission("superAdmin") {
                return Err(exception(
                    "You need to be a super admin to change workspace active hours",
                ));
            }
            let text = |v: Option<Option<String>>| v.flatten().map_or(Sql::Null, Sql::Text);
            let write = workspace::update(
                db,
                ws,
                vec![
                    (
                        "isActive",
                        Sql::Integer(i64::from(is_active.unwrap_or(false))),
                    ),
                    ("startTime", text(start)),
                    ("endTime", text(end)),
                ],
            )
            .await?;
            after_write(api, &write).await;
            Ok(None)
        }
        "scheduleWorkspaceDeletion" => {
            if !context.has_permission("superAdmin") {
                return Err(exception(SUPER_ADMIN_REQUIRED));
            }
            workspace::schedule_deletion(db, context.user_id, ws, chatbotx_db::repo::now_ms())
                .await?;
            api.record_audit("schedule_deletion", "scheduled the workspace for deletion")
                .await;
            Ok(None)
        }
        // `toggleSupportAccessAction` + `workspaceSupportAccessService.enable /
        // disable`: a real super admin member only; one audit row each.
        "toggleSupportAccess" => {
            let mut input = input_of(raw);
            let enabled = input.bool("enabled");
            input.finish()?;
            if !context.has_permission("superAdmin") {
                return Err(exception(
                    "You need to be a super admin to change platform support access",
                ));
            }
            if context.is_support_session {
                return Err(exception(
                    "Platform support access cannot be changed from within a support session",
                ));
            }
            let until = enabled
                .unwrap_or(false)
                .then(|| chatbotx_db::repo::now_ms() + SUPPORT_ACCESS_WINDOW_MS);
            let updated = chatbotx_business::tx::write(db, move |conn| {
                Ok(conn
                    .execute(
                        r#"UPDATE "Workspace" SET "supportAccessUntil" = ?1 WHERE "id" = ?2"#,
                        chatbotx_db::rusqlite::params![until, ws],
                    )
                    .map_err(chatbotx_db::repo::Error::from)?)
            })
            .await?;
            if updated == 0 {
                return Err(BusinessError::not_found("Workspace not found").into());
            }
            let (action, detail) = match until {
                Some(until) => (
                    "support_access_enabled",
                    format!(
                        "enabled platform support access for workspace {ws} until {}",
                        chatbotx_db::repo::iso(until)
                    ),
                ),
                None => (
                    "support_access_disabled",
                    format!("disabled platform support access for workspace {ws}"),
                ),
            };
            tracing::info!(action, user_id = context.user_id, workspace_id = ws, "{detail}");
            api.audit
                .record(chatbotx_business::audit::AuditRecordInput {
                    user_id: Some(context.user_id.to_string()),
                    workspace_id: Some(ws.to_string()),
                    ..chatbotx_business::audit::AuditRecordInput::new(action, detail)
                })
                .await;
            Ok(None)
        }
        "cancelWorkspaceDeletion" => {
            if !context.has_permission("superAdmin") {
                return Err(exception(SUPER_ADMIN_REQUIRED));
            }
            match workspace::cancel_deletion(db, ws).await {
                Ok(_) => {}
                Err(error) if error.code() == Some("workspaceDeletionStarted") => {
                    return Err(ActionError::Exception(BusinessError::exception(
                        DELETION_ALREADY_STARTED,
                        "workspaceDeletionStarted",
                        error.status().unwrap_or(409),
                    )));
                }
                Err(error) => return Err(error.into()),
            }
            api.record_audit("cancel_deletion", "canceled the workspace deletion")
                .await;
            Ok(None)
        }
        "createWorkspaceToken" => {
            let mut input = input_of(raw);
            let name = input.string(
                "name",
                Str {
                    min: Some(1),
                    max: Some(100),
                    ..Str::default()
                },
            );
            let permission = {
                let v = input.get("permission").cloned();
                input.enum_at(
                    &key("permission"),
                    v.as_ref(),
                    WORKSPACE_API_TOKEN_PERMISSIONS,
                    true,
                )
            };
            let all_scopes = input.bool("allScopes");
            let scopes = scopes_request(&mut input);
            if !input.failed()
                && all_scopes == Some(false)
                && scopes.as_ref().is_some_and(Vec::is_empty)
            {
                input.issue(
                    &key("scopes"),
                    "custom",
                    "Select at least one scope, or choose All scopes",
                );
            }
            input.finish()?;
            require_token_super_admin(context)?;
            let credentials = generate_workspace_token();
            let created = workspace_api_token::create_token(
                db,
                InsertWorkspaceApiTokenInput {
                    workspace_id: ws,
                    token_hash: credentials.token_hash,
                    name: name.unwrap_or_default(),
                    permission: permission.unwrap_or_default(),
                    token_prefix: credentials.token_prefix,
                    scopes: if all_scopes == Some(true) {
                        None
                    } else {
                        scopes
                    },
                },
            )
            .await?;
            api.record_audit("create", &created.audit).await;
            // The plaintext exists only in this return value.
            Ok(Some(json!({ "token": credentials.token })))
        }
        "deleteWorkspaceToken" => {
            let mut input = input_of(raw);
            let id = input.string(
                "id",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            input.finish()?;
            require_token_super_admin(context)?;
            // A non-numeric id matches no row.
            let id = id.and_then(|t| t.parse::<i64>().ok()).unwrap_or(-1);
            if !workspace_api_token::delete_token(db, ws, id).await? {
                return Err(ActionError::ValidationErrors(
                    json!({ "_errors": ["Token no longer exists"] }),
                ));
            }
            api.record_audit("delete", &format!("deleted workspace API token \"{id}\""))
                .await;
            Ok(None)
        }
        // `deleteErrorLogAction` (`features/error-logs`): `bulkUpdateIdsRequest`.
        "deleteErrorLog" => {
            let mut input = input_of(raw);
            let ids = ids_request(&mut input);
            input.finish()?;
            chatbotx_business::error_log::delete(db, ws, ids.unwrap_or_default()).await?;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!(
            "unknown workspace action {name}"
        ))),
    }
}
