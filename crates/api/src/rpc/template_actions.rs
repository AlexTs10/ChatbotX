//! The template actions of the builder (`features/templates/actions/*`,
//! Q.D4 slice 6) under the `/rpc` key `templateActions`. Each action runs the
//! matching `/v1` template route ([`handlers::templates::handle_as`]) with a
//! member actor and the acting user, so input checks, errors and business
//! rules are the route's; only the output is reshaped to the action's.
//!
//! ponytail: validation issues name the route's fields (`enabled`,
//! `expiresAt`), not the action's (`shareEnabled`, `shareExpiresAt`).
//! `rememberTemplateInstallIntent` / `clearTemplateInstallIntent` (the
//! install-intent cookie) are not ported: no Rust install-intent store yet.

use chatbotx_business::BusinessError;
use chatbotx_business::template::permissions::{Actor, assert_can_manage, can_view_installs};
use serde_json::{Map, Value, json};

use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use crate::handlers::{ApiState, Call, templates};

/// Who may run an action (`templateActionClient*`, `installerActionClient*`,
/// or the plain workspace client).
#[derive(Clone, Copy)]
enum Client {
    Manage,
    Installer,
    Member,
}

/// `(action, gate, client)`.
const ACTIONS: &[(&str, Gate, Client)] = &[
    ("deleteTemplate", Gate::AllowExpired, Client::Manage),
    ("getInstallerIssues", Gate::AllowExpired, Client::Manage),
    ("listTemplateInstallers", Gate::AllowExpired, Client::Manage),
    (
        "previewUninstallInstaller",
        Gate::AllowExpired,
        Client::Manage,
    ),
    ("uninstallInstaller", Gate::AllowExpired, Client::Manage),
    ("getShareSettings", Gate::Workspace, Client::Manage),
    ("rotateShareToken", Gate::Workspace, Client::Manage),
    ("updateShareSettings", Gate::Workspace, Client::Manage),
    ("saveTemplate", Gate::Workspace, Client::Manage),
    ("retryInstallation", Gate::Workspace, Client::Installer),
    ("syncInstallation", Gate::Workspace, Client::Installer),
    ("restoreSyncBackup", Gate::Workspace, Client::Installer),
    (
        "setInstallationAutoUpdate",
        Gate::Workspace,
        Client::Installer,
    ),
    (
        "previewUninstallInstallation",
        Gate::AllowExpired,
        Client::Installer,
    ),
    (
        "uninstallInstallation",
        Gate::AllowExpired,
        Client::Installer,
    ),
    (
        "cancelUninstallInstallation",
        Gate::AllowExpired,
        Client::Installer,
    ),
    ("installTemplate", Gate::Workspace, Client::Member),
    ("previewInstallCollisions", Gate::Workspace, Client::Member),
];

/// The actions of the `templateActions` key.
pub fn action_gate(name: &str) -> Option<Gate> {
    ACTIONS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, g, _)| *g)
}

/// The `n`-th bind arg after the workspace id (`zodBigintAsString` or the
/// publisher ref): `id`, then `bindArgs[2]`.
fn bind(raw: &Value, n: usize) -> String {
    let value = if n == 1 {
        raw.get("id")
    } else {
        raw.get("bindArgs").and_then(|b| b.get(n))
    };
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn field(raw: &Value, name: &str) -> Value {
    raw.get(name).cloned().unwrap_or(Value::Null)
}

/// The action input without the bound args.
fn body_of(raw: &Value) -> Map<String, Value> {
    let mut body = raw.as_object().cloned().unwrap_or_default();
    for key in ["workspaceId", "id", "bindArgs"] {
        body.remove(key);
    }
    body
}

/// The action's output from the route's output and the action input.
type Shape = fn(Value, &Value) -> Option<Value>;
/// `(route, path params, body, shape)`.
type Plan = (&'static str, Vec<(&'static str, String)>, Value, Shape);

/// Runs one template action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let Some((_, _, client)) = ACTIONS.iter().find(|(n, ..)| *n == name) else {
        return Err(ActionError::Internal(format!(
            "unknown template action {name}"
        )));
    };
    let actor = Actor::Member(context.permissions.clone());
    match client {
        Client::Manage => assert_can_manage(&actor)?,
        Client::Installer if !can_view_installs(&actor) => {
            return Err(ActionError::Exception(BusinessError::exception(
                "You don't have permission to use template installs",
                "templateCategoryPermissionDenied",
                403,
            )));
        }
        _ => {}
    }
    let id = bind(raw, 1);
    let mut body = body_of(raw);
    let (route, params, body, shape): Plan = match name {
        "deleteTemplate" => ("templates.delete", vec![("id", id)], json!({}), |_, _| None),
        "getInstallerIssues" => (
            "templates.getInstallerIssues",
            vec![
                ("id", id),
                (
                    "installerId",
                    field(raw, "publisherRef")
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                ),
            ],
            json!({}),
            |out, _| Some(out),
        ),
        "listTemplateInstallers" => {
            let mut params = vec![("id", id)];
            for key in ["search", "cursor"] {
                if let Some(v) = raw
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    params.push((key, v.to_string()));
                }
            }
            if let Some(limit) = raw.get("limit").filter(|v| !v.is_null()) {
                params.push(("limit", limit.to_string()));
            }
            ("templates.listInstallers", params, json!({}), |out, _| {
                Some(out)
            })
        }
        "previewUninstallInstaller" => (
            "templates.previewUninstallInstaller",
            vec![("id", id), ("installerId", bind(raw, 2))],
            json!({}),
            |out, _| Some(out),
        ),
        "uninstallInstaller" => (
            "templates.uninstallInstaller",
            vec![("id", id), ("installerId", bind(raw, 2))],
            json!({}),
            |out, _| Some(out),
        ),
        "getShareSettings" => (
            "templates.getLink",
            vec![("id", id)],
            json!({}),
            |out, _| {
                Some(json!({
                    "id": out["templateId"],
                    "shareToken": out["templateKey"],
                    "shareUrl": out["url"],
                    "shareEnabled": out["shareEnabled"],
                    "shareExpiresAt": out["shareExpiresAt"],
                }))
            },
        ),
        "rotateShareToken" => (
            "templates.rotateShare",
            vec![("id", id)],
            json!({}),
            |out, _| Some(json!({ "shareToken": out["templateKey"] })),
        ),
        "updateShareSettings" => (
            "templates.updateShare",
            vec![(
                "id",
                field(raw, "templateId")
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )],
            json!({ "enabled": field(raw, "shareEnabled"), "expiresAt": field(raw, "shareExpiresAt") }),
            |out, _| {
                Some(
                    json!({ "shareToken": out["templateKey"], "shareExpiresAt": out["shareExpiresAt"] }),
                )
            },
        ),
        "saveTemplate" => {
            let existing = body
                .remove("templateId")
                .and_then(|v| v.as_str().map(str::to_string));
            let route = if existing.is_some() {
                "templates.update"
            } else {
                "templates.create"
            };
            let params = existing.map(|t| vec![("id", t)]).unwrap_or_default();
            (route, params, Value::Object(body), |out, _| {
                Some(json!({ "templateId": out["id"] }))
            })
        }
        "retryInstallation" => (
            "templateInstallations.retry",
            vec![("id", id)],
            json!({}),
            |out, _| Some(json!({ "installationId": out["installationId"] })),
        ),
        "syncInstallation" => (
            "templateInstallations.sync",
            vec![("id", id)],
            json!({}),
            |out, _| Some(json!({ "syncId": out["syncId"] })),
        ),
        "restoreSyncBackup" => (
            "templateInstallations.restoreBackup",
            vec![
                ("id", id),
                (
                    "backupId",
                    field(raw, "backupId")
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                ),
            ],
            json!({}),
            |_, raw| Some(json!({ "backupId": raw["backupId"] })),
        ),
        "setInstallationAutoUpdate" => (
            "templateInstallations.update",
            vec![("id", id)],
            json!({ "autoUpdate": field(raw, "autoUpdate") }),
            |_, raw| Some(json!({ "autoUpdate": raw["autoUpdate"] })),
        ),
        "previewUninstallInstallation" => (
            "templateInstallations.previewUninstall",
            vec![("id", id)],
            json!({}),
            |out, _| Some(out),
        ),
        "uninstallInstallation" => (
            "templateInstallations.uninstall",
            vec![("id", id)],
            json!({}),
            |out, _| Some(out),
        ),
        "cancelUninstallInstallation" => (
            "templateInstallations.cancelUninstall",
            vec![("id", id)],
            json!({}),
            |out, _| Some(out),
        ),
        "installTemplate" => {
            body.remove("idempotencyKey");
            (
                "templateInstallations.install",
                Vec::new(),
                Value::Object(body),
                |out, _| {
                    Some(
                        json!({ "installationId": out["installationId"], "syncId": out.get("syncId").cloned().unwrap_or(Value::Null) }),
                    )
                },
            )
        }
        "previewInstallCollisions" => (
            "templatePreviews.get",
            vec![(
                "id",
                field(raw, "shareToken")
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )],
            json!({}),
            |out, _| Some(out),
        ),
        _ => {
            return Err(ActionError::Internal(format!(
                "unknown template action {name}"
            )));
        }
    };
    let body = body.to_string();
    let reply = templates::handle_as(
        api,
        route,
        Call {
            workspace_id: context.workspace_id(),
            params: &params,
            query: None,
            body: body.as_bytes(),
        },
        actor,
        Some(context.user_id),
    )
    .await;
    let response = reply.map_err(|error| {
        if error.code == "invalidRequestData" {
            ActionError::from(error)
        } else {
            ActionError::Exception(BusinessError::exception(
                error.message,
                &error.code,
                error.status,
            ))
        }
    })?;
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| ActionError::Internal(e.to_string()))?;
    let out = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|e| ActionError::Internal(e.to_string()))?
    };
    Ok(shape(out, raw))
}
