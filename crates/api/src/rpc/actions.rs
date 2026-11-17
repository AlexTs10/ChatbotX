//! Builder server actions served as `/rpc` procedures for the SPA (Q.D11),
//! in the style of P3.5f's `authActions`: `POST /rpc/<key>/<name>` with the
//! oRPC RPC body `{ json: input }`; a workspace action's input carries the
//! bound `workspaceId`.
//!
//! The safe-action clients (`lib/safe-action.ts`) become [`Gate`]s:
//! `authActionClient` (session, `mustChangePassword` 403),
//! `workspaceActionClientAllowExpired` (membership),
//! `workspaceActionClient` (+ scheduled-deletion 403) and
//! `workspaceActionClientAllowScheduledDeletion`. The owner-quota denial of
//! the last two (`checkWorkspaceOwnerAccess`) is cloud-only and not ported.
//!
//! Errors: a `ChatbotXException` keeps its code, status and message (the
//! next-safe-action `serverError`); an input that fails its schema is
//! `400 BAD_REQUEST` with `data.validationErrors` (next-safe-action's
//! formatted shape); any other failure is `DEFAULT_SERVER_ERROR_MESSAGE`.

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chatbotx_business::BusinessError;
use chatbotx_business::audit::{AuditActor, with_audit_context};
use chatbotx_db::repo::models::Workspace;
use serde_json::{Map, Value, json};

use super::{RpcState, find_rpc_session, rpc_input};
use crate::handlers::ApiState;
use crate::orpc::OrpcError;

/// The action keys (`/rpc/<key>/<name>`).
pub const ACTION_KEYS: &[&str] = &[
    "workspaceActions",
    "workspaceMemberActions",
    "platformCredentialActions",
    "inboxTeamActions",
    "helpItemActions",
];

/// next-safe-action `DEFAULT_SERVER_ERROR_MESSAGE`.
pub const DEFAULT_SERVER_ERROR_MESSAGE: &str =
    "Something went wrong while executing the operation.";

/// The safe-action client of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// `authActionClient`: a signed-in user.
    Auth,
    /// `workspaceActionClientAllowExpired`.
    AllowExpired,
    /// `workspaceActionClient`: also refused while deletion is scheduled.
    Workspace,
    /// `workspaceActionClientAllowScheduledDeletion`.
    AllowScheduledDeletion,
}

/// The context an action runs with (`ctx`).
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub user_id: i64,
    /// The workspace of a workspace action.
    pub workspace: Option<Workspace>,
    /// `ctx.workspaceMemberPermissions`.
    pub permissions: Value,
    /// `ctx.isSupportSession` (support access is not ported: always `false`).
    pub is_support_session: bool,
}

impl ActionContext {
    /// The workspace id of a workspace action (`0` for an auth action).
    pub fn workspace_id(&self) -> i64 {
        self.workspace.as_ref().map_or(0, |w| w.id)
    }

    /// `hasWorkspacePermission(permissions, key)` (`superAdmin` grants all).
    pub fn has_permission(&self, key: &str) -> bool {
        self.permissions.get("superAdmin") == Some(&Value::Bool(true))
            || self.permissions.get(key) == Some(&Value::Bool(true))
    }
}

/// An action outcome: `Ok(None)` = `undefined`.
pub type ActionResult = Result<Option<Value>, ActionError>;

/// A failed action.
#[derive(Debug)]
pub enum ActionError {
    /// A `ChatbotXException` (code, status, message).
    Exception(BusinessError),
    /// The input failed its schema: zod issues (`{ code, path, message }`).
    Validation(Vec<Value>),
    /// `returnValidationErrors(schema, errors)`: next-safe-action errors as they are.
    ValidationErrors(Value),
    /// Any other failure (logged; the default message is sent).
    Internal(String),
}

impl From<BusinessError> for ActionError {
    fn from(error: BusinessError) -> ActionError {
        match error {
            BusinessError::Repo(error) => ActionError::Internal(error.message()),
            other => ActionError::Exception(other),
        }
    }
}

impl From<OrpcError> for ActionError {
    /// An `Input::finish` error (`invalidRequestData` with `data.issues`).
    fn from(error: OrpcError) -> ActionError {
        let issues = error
            .data
            .as_ref()
            .and_then(|data| data.get("issues"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        ActionError::Validation(issues)
    }
}

/// `new ChatbotXException(message)` (`systemError`, 400).
pub fn exception(message: &str) -> ActionError {
    ActionError::Exception(BusinessError::exception(message, "systemError", 400))
}

/// next-safe-action's formatted validation errors of zod issues:
/// `{ _errors?: [...], field: { _errors: [...] } }`.
pub fn validation_errors(issues: &[Value]) -> Value {
    let mut root = Map::new();
    for issue in issues {
        let message = issue
            .get("message")
            .cloned()
            .unwrap_or(Value::String(String::new()));
        let path: Vec<String> = issue
            .get("path")
            .and_then(Value::as_array)
            .map(|p| {
                p.iter()
                    .map(|seg| match seg {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut node = &mut root;
        for seg in &path {
            node = node
                .entry(seg.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("validation node is an object");
        }
        node.entry("_errors")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("_errors is an array")
            .push(message);
    }
    Value::Object(root)
}

fn error_response(status: StatusCode, code: &str, message: &str, data: Option<Value>) -> Response {
    let mut error = OrpcError::new(code, Some(status.as_u16()), Some(message));
    error.defined = false;
    error.data = data;
    super::procedures::error_response(&error)
}

fn action_error_response(error: ActionError) -> Response {
    match error {
        ActionError::Exception(BusinessError::Exception {
            message,
            code,
            status,
            ..
        }) => error_response(
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            &code,
            &message,
            None,
        ),
        ActionError::Exception(BusinessError::Repo(error)) => internal(&error.message()),
        ActionError::Internal(error) => internal(&error),
        ActionError::Validation(issues) => error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "Input validation failed",
            Some(json!({ "validationErrors": validation_errors(&issues) })),
        ),
        ActionError::ValidationErrors(errors) => error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "Input validation failed",
            Some(json!({ "validationErrors": errors })),
        ),
    }
}

fn internal(cause: &str) -> Response {
    tracing::error!(cause, "Error in actionClient");
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_SERVER_ERROR",
        DEFAULT_SERVER_ERROR_MESSAGE,
        None,
    )
}

/// The action's return value, RPC-encoded (`undefined` -> meta `[[3]]`).
fn output(value: Option<Value>) -> Response {
    let body = match value {
        Some(value) => json!({ "json": value }),
        None => json!({ "json": null, "meta": [[3]] }),
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `input.workspaceId` (`zodBigintAsString` of the bound arg).
fn workspace_id_of(input: &Value) -> Option<i64> {
    let text = input.get("workspaceId")?.as_str()?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The gate of `<key>/<name>`; `None` = no such action.
fn gate_of(key: &str, name: &str) -> Option<Gate> {
    match key {
        "workspaceActions" => super::workspace::action_gate(name),
        "workspaceMemberActions" => super::workspace_members::action_gate(name),
        "inboxTeamActions" => super::inbox_teams::action_gate(name),
        "helpItemActions" => super::help_items::action_gate(name),
        "platformCredentialActions" => super::platform_credentials::action_gate(name),
        _ => None,
    }
}

async fn run(
    api: &ApiState,
    key: &str,
    name: &str,
    context: &ActionContext,
    input: &Value,
) -> ActionResult {
    match key {
        "workspaceActions" => super::workspace::run_action(api, name, context, input).await,
        "workspaceMemberActions" => {
            super::workspace_members::run_action(api, name, context, input).await
        }
        "platformCredentialActions" => {
            super::platform_credentials::run_action(api, name, context, input).await
        }
        "inboxTeamActions" => super::inbox_teams::run_action(api, name, context, input).await,
        "helpItemActions" => super::help_items::run_action(api, name, context, input).await,
        _ => Err(ActionError::Internal(format!("unknown action key {key}"))),
    }
}

/// Serves `/rpc/<key>/<name>` when `key` is an action key; `None` otherwise.
pub async fn handle(
    state: &RpcState,
    rest: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<Response> {
    let rest = rest.trim_end_matches('/');
    let (key, name) = rest.split_once('/')?;
    if !ACTION_KEYS.contains(&key) {
        return None;
    }
    let Some(gate) = gate_of(key, name) else {
        return Some(error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Action not ported yet",
            None,
        ));
    };
    let Some(api) = state.api.as_ref() else {
        return Some(internal("no handler state"));
    };
    let session = match find_rpc_session(&state.auth, headers).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Some(error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Unauthorized",
                None,
            ));
        }
        Err(error) => return Some(internal(&error.to_string())),
    };
    if session.must_change_password {
        return Some(error_response(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Password change required",
            None,
        ));
    }
    let input = rpc_input(method, uri, body);
    let mut context = ActionContext {
        user_id: session.user_id,
        workspace: None,
        permissions: json!({}),
        is_support_session: false,
    };
    if gate != Gate::Auth {
        let not_found = || internal("Workspace not found");
        let Some(workspace_id) = workspace_id_of(&input) else {
            return Some(not_found());
        };
        let user_id = session.user_id;
        let found = api
            .db
            .read(move |conn| {
                let Some(workspace) = chatbotx_business::workspace::find(conn, workspace_id)
                    .map_err(|e| chatbotx_db::DbError::other(e.message()))?
                else {
                    return Ok(None);
                };
                let permissions: Option<String> = conn
                    .query_row(
                        r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                        [workspace_id, user_id],
                        |row| row.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        chatbotx_db::rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                Ok(permissions.map(|p| (workspace, p)))
            })
            .await;
        let (workspace, permissions) = match found {
            Ok(Some(found)) => found,
            Ok(None) => return Some(not_found()),
            Err(error) => return Some(internal(&error.to_string())),
        };
        if gate == Gate::Workspace && workspace.scheduled_deletion_at.is_some() {
            return Some(error_response(
                StatusCode::FORBIDDEN,
                "workspaceScheduledDeletion",
                "Workspace deletion scheduled",
                None,
            ));
        }
        context.permissions = serde_json::from_str(&permissions).unwrap_or_else(|_| json!({}));
        context.workspace = Some(workspace);
    }
    let actor = AuditActor {
        user_id: Some(session.user_id.to_string()),
        workspace_id: context.workspace.as_ref().map(|w| w.id.to_string()),
        ip_address: Some(crate::auth::guest_client_ip(headers)),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        source: None,
    };
    let outcome = with_audit_context(actor, run(api, key, name, &context, &input)).await;
    Some(match outcome {
        Ok(value) => output(value),
        Err(error) => action_error_response(error),
    })
}
