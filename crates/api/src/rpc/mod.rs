//! `/rpc/*`: the builder's oRPC RPC endpoint (`app/rpc/[[...rest]]/route.ts`)
//! with the session auth of `authorizedAPI` (`middlewares/auth.ts`
//! `authMiddleware`) and, when the input carries a `workspaceId`,
//! `workspaceAuthorizedMidddleware`.
//!
//! Every procedure of the builder router is in [`RPC_PROCEDURES`] (generated
//! by `conversion/tools/rpc-table-gen/gen.mjs`) with its auth base, method and
//! workspace mapper. A ported procedure runs its handler ([`procedures`]); any
//! other answers the RPC-encoded `NOT_FOUND "Operation not ported yet"` after
//! auth.

pub mod actions;
pub mod admin_workspaces;
pub mod channel_connect;
pub mod help_items;
pub mod inbox_teams;
pub mod platform_admins;
pub mod platform_credentials;
pub mod platform_settings;
pub mod procedures;
mod table;
pub mod template_actions;
pub mod workspace;
pub mod workspace_members;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

pub use table::RPC_PROCEDURES;

use crate::auth::decode_query_component;
use crate::handlers::ApiState;
use crate::session::{
    AuthState, SESSION_COOKIE, apply_bearer, request_cookie, verify_signed_cookie,
};

/// The top-level keys of the builder router (`routers/index.ts`).
pub const RPC_ROUTER_KEYS: &[&str] = &[
    "adsAPI",
    "adsCampaignAPI",
    "googleAdsAPI",
    "appointmentCalendarsAPI",
    "appointmentsAPI",
    "aiMcpServerAPIs",
    "aiAgentsAPI",
    "decisionProfilesAPI",
    "broadcastAPIs",
    "conversationsAPI",
    "miniAppsAPI",
    "couponsAPI",
    "pipelinesAPI",
    "pollsAPI",
    "placesNearMeAPI",
    "leaderboardsAPI",
    "deviceTokensAPI",
    "dynamicImagesAPI",
    "connectionsAPI",
    "connectSessionsAPI",
    "emailTopicsAPI",
    "tagsAPI",
    "contactFilterAPI",
    "customFieldsAPI",
    "flowsAPI",
    "contactsAPIs",
    "contactNotesAPI",
    "contactSequencesAPI",
    "contactScanAPIs",
    "aiHandoverAPIs",
    "channelPostAPIs",
    "botFieldAPIs",
    "integrationActiveCampaignAPI",
    "integrationFacebookAdsAPI",
    "integrationDripAPI",
    "integrationGetResponseAPI",
    "integrationInstagramAPIs",
    "integrationKlaviyoAPI",
    "integrationMailchimpAPI",
    "integrationMailerLiteAPI",
    "integrationMessengerAPIs",
    "integrationMoosendAPI",
    "integrationSmtpAPI",
    "integrationSendGridAPI",
    "integrationWhatsappAPIs",
    "whatsappMessageTemplateAPIs",
    "whatsappFlowAPIs",
    "messengerMessageTemplateAPIs",
    "savedRepliesAPI",
    "commentAutomationsAPI",
    "fbCommentsAPI",
    "igCommentsAPI",
    "threadsCommentsAPI",
    "igStoriesAPI",
    "facebookLeadAdsAPI",
    "facebookMarketingMessagesAPI",
    "responseSlaAPI",
    "workingHoursAPI",
    "sequencesAPI",
    "aiFilesAPI",
    "inboxesAPI",
    "spreadsheetsAPI",
    "workspaceMembersAPI",
    "inboxTeamsAPI",
    "foldersAPI",
    "messagesAPI",
    "personasAPIs",
    "workspacesAPI",
    "aiFunctionsAPI",
    "platformCredentialsAPI",
    "productsAPI",
    "productCategoriesAPI",
    "questionnairesAPI",
    "refLinksAPI",
    "realtimeAPI",
    "userPersistentMenusAPI",
    "templatesAPI",
    "analyticsRoutes",
    "mediaLibraryAPI",
];

/// The auth base of a procedure (`authorizedAPI`, `workspaceTokenAuthAPIForScope(scope)`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcAuth {
    /// `authorizedAPI`: a better-auth session.
    Session,
    /// `workspaceTokenAuthAPIForScope(scope)`: a workspace API token.
    Token(&'static str),
    /// `channelApiTokenAPI`.
    ChannelToken,
    /// `mobileUserAPI`.
    MobileUser,
    /// `mobileUserPasswordChangeAPI`.
    MobileUserPasswordChange,
    /// `os` / `base` without an auth middleware.
    Public,
}

/// One procedure of the builder router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcProcedure {
    /// The router path, `.`-joined (`pollsAPI.listPollsAPI`).
    pub path: &'static str,
    pub auth: RpcAuth,
    /// `.route({ method })`; `None` = not set (the owner gate treats it as POST).
    pub method: Option<&'static str>,
    /// The input path the `workspaceAuthorizedMidddleware` mapper reads
    /// (`["workspaceId"]`); `None` = no workspace check.
    pub workspace: Option<&'static [&'static str]>,
    /// `true` when the input schema is validated before the workspace check
    /// (`.input(...)` before `.use(workspaceAuthorizedMidddleware, ...)`).
    pub validate_first: bool,
}

/// The procedure of a `.`-joined router path.
pub fn find_procedure(path: &str) -> Option<&'static RpcProcedure> {
    RPC_PROCEDURES
        .iter()
        .find(|procedure| procedure.path == path)
}

/// What the `/rpc` handler needs: the session store and, for the ported
/// procedures, the handler state and the realtime broadcast secret.
#[derive(Clone)]
pub struct RpcState {
    pub auth: Arc<AuthState>,
    /// `None` = no procedure is ported (every call answers "not ported").
    pub api: Option<Arc<ApiState>>,
    /// `resolveBroadcastSecret()` (`REALTIME_BROADCAST_SECRET`); `None` = the
    /// realtime gateway is off.
    pub realtime_secret: Option<String>,
}

/// The router of `/rpc/*` without procedure handlers (auth only).
pub fn rpc_router(state: Arc<AuthState>) -> Router {
    rpc_router_with(RpcState {
        auth: state,
        api: None,
        realtime_secret: None,
    })
}

/// The router of `/rpc/*` with the ported procedures.
pub fn rpc_router_with(state: RpcState) -> Router {
    Router::new()
        .route("/rpc/{*rest}", any(handle))
        // next.config `serverActions.bodySizeLimit: "21mb"` (device files).
        .layer(axum::extract::DefaultBodyLimit::max(21 * 1024 * 1024))
        .with_state(Arc::new(state))
}

/// The workspace context of a call that passed `workspaceAuthorizedMidddleware`.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcWorkspace {
    pub workspace_id: i64,
    /// `context.workspaceMember.role` (`owner` / `admin` / `member`).
    pub role: String,
    /// `context.workspaceMember.permissions` (the stored JSON object).
    pub permissions: Value,
}

/// The context of a procedure call (`context.session`, `context.user`,
/// `context.workspace`, `context.requestIssuedAtMs`).
#[derive(Debug, Clone, PartialEq)]
pub struct RpcContext {
    pub session: RpcSession,
    pub workspace: Option<RpcWorkspace>,
    pub request_issued_at_ms: i64,
}

/// The authenticated caller of a `/rpc` call (`context.session` + `context.user`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcSession {
    pub session_id: i64,
    pub user_id: i64,
    pub must_change_password: bool,
}

/// An RPC-encoded oRPC error (`{ json: error.toJSON(), meta }`; `data` is
/// `undefined`, so the meta marks it).
pub(crate) fn rpc_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({
        "json": {
            "defined": false,
            "code": code,
            "status": status.as_u16(),
            "message": message,
        },
        "meta": [[3, "data"]],
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn unauthorized() -> Response {
    rpc_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "Unauthorized")
}

fn internal(error: impl std::fmt::Display) -> Response {
    tracing::error!(error = %error, "RPC handler failed");
    rpc_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_SERVER_ERROR",
        "Internal server error",
    )
}

/// The procedure input: `{ json }` of a POST body, or `?data=` of a GET.
fn rpc_input(method: &Method, uri: &Uri, body: &Bytes) -> Value {
    let raw = if method == Method::GET {
        uri.query()
            .and_then(|query| {
                query
                    .split('&')
                    .filter_map(|pair| pair.split_once('='))
                    .find(|(key, _)| decode_query_component(key) == "data")
                    .map(|(_, value)| decode_query_component(value))
            })
            .unwrap_or_default()
    } else if let Some(input) = multipart_input(body) {
        return input;
    } else {
        String::from_utf8_lossy(body).into_owned()
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null)
}

/// The oRPC RPC `FormData` body of an input with `Blob`s: part `data` is
/// `{ json, meta, maps }`, part `<i>` the blob at the path `maps[i]`. Each
/// blob becomes a file value (`handlers::contact_messages::file_value`).
fn multipart_input(body: &[u8]) -> Option<Value> {
    let parts = crate::handlers::ai_files::parse_multipart(body)?;
    let data = parts.iter().find(|part| part.name == "data")?;
    let payload: Value = serde_json::from_slice(&data.data).ok()?;
    let mut json = payload.get("json").cloned().unwrap_or(Value::Null);
    for (index, path) in payload["maps"].as_array().into_iter().flatten().enumerate() {
        let Some(part) = parts.iter().find(|part| part.name == index.to_string()) else {
            continue;
        };
        let mut at = &mut json;
        for segment in path.as_array().into_iter().flatten() {
            at = match segment {
                Value::String(key) => &mut at[key.as_str()],
                Value::Number(n) => match (n.as_u64(), at.as_array_mut()) {
                    (Some(i), Some(items)) if (i as usize) < items.len() => &mut items[i as usize],
                    _ => return None,
                },
                _ => return None,
            };
        }
        *at = crate::handlers::contact_messages::file_value(
            part.filename.as_deref().unwrap_or("blob"),
            part.content_type.as_deref().unwrap_or_default(),
            &part.data,
        );
    }
    Some(json)
}

/// `auth.api.getSession({ headers })` for the oRPC context: a signed,
/// unexpired session cookie and its user.
pub async fn find_rpc_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<Option<RpcSession>, chatbotx_db::DbError> {
    let headers = &apply_bearer(headers, &state.settings.secret);
    let Some(token) = request_cookie(headers, SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret))
    else {
        return Ok(None);
    };
    let now = (state.now_ms)();
    state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT s."id", u."id", u."mustChangePassword"
                       FROM "Session" s JOIN "User" u ON u."id" = s."userId"
                       WHERE s."token" = ?1 AND s."expiresAt" > ?2"#,
                    params![token, now],
                    |row| {
                        Ok(RpcSession {
                            session_id: row.get(0)?,
                            user_id: row.get(1)?,
                            must_change_password: row.get(2)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
}

/// `workspaceAuthorizedMidddleware`: the caller's membership in the
/// workspace; `Some((scheduledDeletionAt, permissions, role))` when a member.
async fn find_membership(
    state: &AuthState,
    workspace_id: i64,
    user_id: i64,
) -> Result<Option<(Option<i64>, String, String)>, chatbotx_db::DbError> {
    state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT w."scheduledDeletionAt", m."permissions", m."role"
                       FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
                       WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
                    params![workspace_id, user_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
}

/// The input value at `path` (`input.workspaceId`).
fn input_at<'a>(input: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(input, |value, key| value.get(key))
}

/// `zodBigintAsString` / `z.string()` of a workspace id: digits only.
fn workspace_id_of(value: Option<&Value>) -> Option<i64> {
    let text = value?.as_str()?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// oRPC's input validation error (the zod issues are not reported).
fn input_validation_failed() -> Response {
    rpc_error(
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
        "Input validation failed",
    )
}

/// A procedure output, RPC-encoded (`{ json, meta }`): the timestamps
/// ([`date_meta`]) become `Date`s on the client.
fn rpc_output(output: Value) -> Response {
    let mut meta = Vec::new();
    date_meta(&output, &mut Vec::new(), &mut meta);
    let body = if meta.is_empty() {
        json!({ "json": output })
    } else {
        json!({ "json": output, "meta": meta })
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// A procedure output with its meta (`[1, ...path]` per Date).
fn rpc_output_with_meta(output: Value, meta: Vec<Value>) -> Response {
    let body = if meta.is_empty() {
        json!({ "json": output })
    } else {
        json!({ "json": output, "meta": meta })
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `[1, ...path]` (oRPC `DATE`) for every ISO string under a timestamp key
/// (`chatbotx_db::repo::is_date_column`).
// ponytail: dates are found by key name + ISO shape (the builder's zod
// `z.date()` outputs); a per-procedure date list if a text column ever matches.
fn date_meta(value: &Value, path: &mut Vec<Value>, meta: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                path.push(Value::String(key.clone()));
                if let Value::String(text) = child
                    && chatbotx_db::repo::is_date_column(key)
                    && is_iso_millis(text)
                {
                    let mut entry = vec![json!(1)];
                    entry.extend(path.iter().cloned());
                    meta.push(Value::Array(entry));
                } else {
                    date_meta(child, path, meta);
                }
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                path.push(json!(index));
                date_meta(child, path, meta);
                path.pop();
            }
        }
        _ => {}
    }
}

/// `2026-11-16T10:00:00.000Z`.
fn is_iso_millis(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 24
        && bytes[4] == b'-'
        && bytes[10] == b'T'
        && bytes[19] == b'.'
        && bytes[23] == b'Z'
}

/// `forceChangePasswordAction` (a Next.js server action in the original,
/// served here as an RPC procedure for the SPA). It runs before the
/// `mustChangePassword` gate, like the action on the base `actionClient`.
pub const FORCE_CHANGE_PASSWORD_PATH: &str = "authActions/forceChangePassword";

/// The SPA stub path of `forceChangePasswordAction` (`{ args: [input] }`).
const FORCE_CHANGE_PASSWORD_SPA_PATH: &str = "actions/forceChangePasswordAction";

/// `submitOAuthConsentAction` and the consent page data (Q.A1): served as
/// `authActions/submitOAuthConsent` and `authActions/oauthConsentPage`.
async fn oauth_consent_action(
    state: &RpcState,
    rest: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<Response> {
    let input = rpc_input(method, uri, body);
    let result = match rest.trim_end_matches('/') {
        "authActions/submitOAuthConsent" => {
            crate::session::submit_oauth_consent(&state.auth, headers, &input).await
        }
        "authActions/oauthConsentPage" => {
            crate::session::oauth_consent_page(&state.auth, headers, &input).await
        }
        _ => return None,
    };
    Some(match result {
        Ok(output) => rpc_output(output),
        Err(error) => rpc_error(
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            &error.code,
            &error.message,
        ),
    })
}

async fn force_change_password_action(
    state: &RpcState,
    headers: &HeaderMap,
    input: Value,
) -> Response {
    match crate::session::force_change_password(&state.auth, headers, &input).await {
        Ok(cookie) => {
            let mut response = rpc_output(json!({ "ok": true }));
            if let Some(cookie) = cookie {
                response.headers_mut().append(header::SET_COOKIE, cookie);
                crate::session::expose_auth_token(response.headers_mut());
            }
            response
        }
        Err(error) => rpc_error(
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            &error.code,
            &error.message,
        ),
    }
}

async fn handle(
    State(state): State<Arc<RpcState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_issued_at_ms = (state.auth.now_ms)();
    let rest = uri.path().strip_prefix("/rpc/").unwrap_or_default();
    match rest.trim_end_matches('/') {
        FORCE_CHANGE_PASSWORD_PATH => {
            let input = rpc_input(&method, &uri, &body);
            return force_change_password_action(&state, &headers, input).await;
        }
        FORCE_CHANGE_PASSWORD_SPA_PATH => {
            let input = actions::spa_input(&rpc_input(&method, &uri, &body));
            return force_change_password_action(&state, &headers, input).await;
        }
        _ => {}
    }
    if let Some(response) = oauth_consent_action(&state, rest, &method, &uri, &headers, &body).await
    {
        return response;
    }
    if let Some(response) = actions::handle(&state, rest, &method, &uri, &headers, &body).await {
        return response;
    }
    // A Q.D11 action called through the SPA stub path: same gates and errors.
    if let Some(export) = rest.trim_end_matches('/').strip_prefix("actions/")
        && let Some((key, name)) = actions::spa_target(export)
    {
        let mut input = actions::spa_input(&rpc_input(&method, &uri, &body));
        // The credential actions bind their `scope` ("user" / "platform").
        if key == "platformCredentialActions"
            && let Some(scope) = input.as_object_mut().and_then(|o| o.remove("workspaceId"))
        {
            input["scope"] = scope;
        }
        let body = Bytes::from(json!({ "json": input }).to_string());
        let rest = format!("{key}/{name}");
        if let Some(response) =
            actions::handle(&state, &rest, &Method::POST, &uri, &headers, &body).await
        {
            return response;
        }
    }
    let key = rest.split('/').next().unwrap_or_default();
    let is_action = key == "actions";
    if !is_action && !RPC_ROUTER_KEYS.contains(&key) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let path = rest.trim_end_matches('/').replace('/', ".");
    // A name that is not in the table keeps the generic session auth (and the
    // workspace check when the input has a `workspaceId`).
    let procedure = find_procedure(&path);
    let session = match find_rpc_session(&state.auth, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => return unauthorized(),
        Err(error) => return internal(error),
    };
    if session.must_change_password {
        return rpc_error(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Password change required",
        );
    }
    let input = rpc_input(&method, &uri, &body);
    let workspace_path: &[&str] = match procedure {
        Some(procedure) => procedure.workspace.unwrap_or_default(),
        None if input.get("workspaceId").is_some_and(Value::is_string) => &["workspaceId"],
        None => &[],
    };
    let mut workspace = None;
    if !workspace_path.is_empty() {
        let Some(workspace_id) = workspace_id_of(input_at(&input, workspace_path)) else {
            return if procedure.is_some_and(|procedure| procedure.validate_first) {
                input_validation_failed()
            } else {
                unauthorized()
            };
        };
        match find_membership(&state.auth, workspace_id, session.user_id).await {
            Ok(None) => return unauthorized(),
            Ok(Some((Some(_scheduled), _, _))) => {
                return rpc_error(
                    StatusCode::FORBIDDEN,
                    "FORBIDDEN",
                    "Workspace deletion scheduled",
                );
            }
            Ok(Some((None, permissions, role))) => {
                workspace = Some(RpcWorkspace {
                    workspace_id,
                    role,
                    permissions: serde_json::from_str(&permissions).unwrap_or(Value::Null),
                });
            }
            Err(error) => return internal(error),
        }
    }
    let context = RpcContext {
        session,
        workspace,
        request_issued_at_ms,
    };
    if is_action {
        let name = path.strip_prefix("actions.").unwrap_or_default();
        return match crate::rpc_procedures::action(&state, name, &context, &input).await {
            Some(Ok(output)) => rpc_output(output),
            Some(Err(error)) => procedures::error_response(&error),
            None => rpc_error(StatusCode::NOT_FOUND, "NOT_FOUND", "Action not ported yet"),
        };
    }
    if let Some(procedure) = procedure
        && let Some(outcome) = procedures::call(&state, procedure.path, &context, &input).await
    {
        return match outcome {
            Ok(output) => rpc_output(output),
            Err(error) => procedures::error_response(&error),
        };
    }
    if let Some(api) = state.api.as_ref() {
        let outcome = match workspace::call(api, &path, &context, &input).await {
            Some(outcome) => Some(outcome),
            None => match workspace_members::call(api, &path, &context, &input).await {
                Some(outcome) => Some(outcome),
                None => match platform_credentials::call(api, &path, &context).await {
                    Some(outcome) => Some(outcome),
                    None => match inbox_teams::call(api, &path, &context).await {
                        Some(outcome) => Some(outcome),
                        None => channel_connect::call(api, &path, &context, &input).await,
                    },
                },
            },
        };
        if let Some(outcome) = outcome {
            return match outcome {
                Ok((output, meta)) => rpc_output_with_meta(output, meta),
                Err(error) => procedures::error_response(&error),
            };
        }
    }
    rpc_error(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "Operation not ported yet",
    )
}
