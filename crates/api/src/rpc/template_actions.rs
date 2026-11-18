//! The template actions of the builder (`features/templates/actions/*`,
//! Q.D4 slice 6) under the `/rpc` key `templateActions`. Each action runs the
//! matching `/v1` template route ([`handlers::templates::handle_as`]) with a
//! member actor and the acting user, so input checks, errors and business
//! rules are the route's; only the output is reshaped to the action's.
//!
//! ponytail: validation issues name the route's fields (`enabled`,
//! `expiresAt`), not the action's (`shareEnabled`, `shareExpiresAt`).
//! `rememberTemplateInstallIntent` / `clearTemplateInstallIntent` are
//! `authActionClient` actions (`rpc_procedures::templates::auth_action`).

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
    // The page reads of `app/space/[workspaceId]/templates/**` (P4 SPA).
    ("listTemplates", Gate::AllowExpired, Client::Manage),
    ("getEditableTemplate", Gate::AllowExpired, Client::Manage),
    ("listTemplateVersions", Gate::AllowExpired, Client::Manage),
    (
        "listTemplateInstallations",
        Gate::AllowExpired,
        Client::Installer,
    ),
    (
        "getTemplateInstallation",
        Gate::AllowExpired,
        Client::Installer,
    ),
    (
        "listTemplateInstallationResources",
        Gate::AllowExpired,
        Client::Installer,
    ),
    (
        "listTemplateSyncHistory",
        Gate::AllowExpired,
        Client::Installer,
    ),
];

/// `RESOURCES_PER_TAB`: an installation page tab shows its whole category.
const RESOURCES_PER_TAB: i64 = 1000;

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
    let workspace_id = context.workspace_id();
    let numeric_id = || id.parse::<i64>().unwrap_or(-1);
    match name {
        // `getEditableTemplateOrFail`: the detail with deleted picks dropped.
        "getEditableTemplate" => {
            let mut template = route_json(
                api,
                context,
                "templates.get",
                vec![("id", id.clone())],
                actor,
            )
            .await?;
            let selection = template["selection"].take();
            template["selection"] = chatbotx_business::tx::read(&api.db, move |conn| {
                chatbotx_business::template::snapshot::without_deleted_resources(
                    conn,
                    workspace_id,
                    &selection,
                )
            })
            .await?;
            return Ok(Some(template));
        }
        "listTemplateInstallationResources" => {
            let category = field(raw, "category").as_str().map(str::to_string);
            return Ok(Some(
                chatbotx_business::template::request::list_resources(
                    &api.db,
                    workspace_id,
                    numeric_id(),
                    None,
                    Some(RESOURCES_PER_TAB),
                    category,
                )
                .await?,
            ));
        }
        "listTemplateSyncHistory" => {
            return Ok(Some(Value::Array(
                chatbotx_business::template::installations::list_history(
                    &api.db,
                    workspace_id,
                    numeric_id(),
                )
                .await?,
            )));
        }
        "listTemplates" | "listTemplateInstallations" => {
            let route = if name == "listTemplates" {
                "templates.list"
            } else {
                "templateInstallations.list"
            };
            let params = ["cursor", "status", "activeOnly", "limit"]
                .into_iter()
                .filter_map(|key| match field(raw, key) {
                    Value::String(text) => Some((key, text)),
                    Value::Bool(flag) => Some((key, flag.to_string())),
                    Value::Number(n) => Some((key, n.to_string())),
                    _ => None,
                })
                .collect();
            return Ok(Some(route_json(api, context, route, params, actor).await?));
        }
        "listTemplateVersions" | "getTemplateInstallation" => {
            let route = if name == "listTemplateVersions" {
                "templates.listVersions"
            } else {
                "templateInstallations.get"
            };
            let out = route_json(api, context, route, vec![("id", id)], actor).await?;
            return Ok(Some(if name == "listTemplateVersions" {
                out["items"].clone()
            } else {
                out
            }));
        }
        _ => {}
    }
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
    let out = templates::handle_json(
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
    .await
    .map_err(action_error);
    // Installing here settles a share page's choice, either way: no
    // workspace is coming (`installTemplateAction`'s `finally`).
    if name == "installTemplate"
        && let Some(kv) = api.kv.as_deref()
    {
        chatbotx_business::template::install_intent::clear(&kv.durable, context.user_id).await;
    }
    Ok(shape(out?, raw))
}

/// A route error as the action's: input issues stay validation errors.
fn action_error(error: crate::orpc::OrpcError) -> ActionError {
    if error.code == "invalidRequestData" {
        ActionError::from(error)
    } else {
        ActionError::Exception(BusinessError::exception(
            error.message,
            &error.code,
            error.status,
        ))
    }
}

/// A read route (no body) as JSON.
async fn route_json(
    api: &ApiState,
    context: &ActionContext,
    route: &str,
    params: Vec<(&'static str, String)>,
    actor: Actor,
) -> Result<Value, ActionError> {
    templates::handle_json(
        api,
        route,
        Call {
            workspace_id: context.workspace_id(),
            params: &params,
            query: None,
            body: b"",
        },
        actor,
        Some(context.user_id),
    )
    .await
    .map_err(action_error)
}
