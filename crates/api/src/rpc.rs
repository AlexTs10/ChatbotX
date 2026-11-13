//! `/rpc/*`: the builder's oRPC RPC endpoint (`app/rpc/[[...rest]]/route.ts`)
//! with the session auth of `authorizedAPI` (`middlewares/auth.ts`
//! `authMiddleware`) and, when the input carries a `workspaceId`,
//! `workspaceAuthorizedMidddleware`.
//!
//! The procedures themselves are the domain ports (P3.6+): after auth a call
//! answers the RPC-encoded `NOT_FOUND "Operation not ported yet"`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::auth::decode_query_component;
use crate::session::{AuthState, SESSION_COOKIE, request_cookie, verify_signed_cookie};

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

/// The router of `/rpc/*`.
pub fn rpc_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/rpc/{*rest}", any(handle))
        .with_state(state)
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
fn rpc_error(status: StatusCode, code: &str, message: &str) -> Response {
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
    } else {
        String::from_utf8_lossy(body).into_owned()
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null)
}

/// `auth.api.getSession({ headers })` for the oRPC context: a signed,
/// unexpired session cookie and its user.
pub async fn find_rpc_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<Option<RpcSession>, chatbotx_db::DbError> {
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
/// workspace; `Some(scheduledDeletionAt)` when a member.
async fn find_membership(
    state: &AuthState,
    workspace_id: i64,
    user_id: i64,
) -> Result<Option<Option<i64>>, chatbotx_db::DbError> {
    state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT w."scheduledDeletionAt"
                       FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
                       WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
                    params![workspace_id, user_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?)
        })
        .await
}

async fn handle(
    State(state): State<Arc<AuthState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = uri
        .path()
        .strip_prefix("/rpc/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    if !RPC_ROUTER_KEYS.contains(&key) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let session = match find_rpc_session(&state, &headers).await {
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
    if let Some(workspace_id) = input.get("workspaceId").and_then(Value::as_str) {
        let Ok(workspace_id) = workspace_id.parse::<i64>() else {
            return unauthorized();
        };
        match find_membership(&state, workspace_id, session.user_id).await {
            Ok(None) => return unauthorized(),
            Ok(Some(Some(_scheduled))) => {
                return rpc_error(
                    StatusCode::FORBIDDEN,
                    "FORBIDDEN",
                    "Workspace deletion scheduled",
                );
            }
            Ok(Some(None)) => {}
            Err(error) => return internal(error),
        }
    }
    rpc_error(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "Operation not ported yet",
    )
}
