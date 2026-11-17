//! The ported procedures of the private `/rpc` router and the builder queries
//! that call the same operations.
//!
//! - `questionnairesAPI.listQuestionnairesForFlowAPI`
//!   (`features/questionnaires/api/private.ts`)
//! - `placesNearMeAPI.listPlaceCategoryOptionsAPI`
//!   (`features/places-near-me/api/private.ts`)
//! - `realtimeAPI.mintWorkspaceConnectTokenAuthenticatedAPI`
//!   (`features/realtime/api/private.ts`)
//! - `findPoll` (`features/polls/queries/index.ts`)

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chatbotx_business::BusinessError;
use chatbotx_business::poll::service as poll_service;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_rt::token::{ChatScope, MemberConnect, sign_member_connect_token};
use serde_json::{Value, json};

use super::{RpcContext, RpcState};
use crate::handlers::{ApiState, LISTING, business_error, internal_error};
use crate::orpc::OrpcError;

/// The procedures with a Rust handler.
pub const PORTED: &[&str] = &[
    "questionnairesAPI.listQuestionnairesForFlowAPI",
    "placesNearMeAPI.listPlaceCategoryOptionsAPI",
    "realtimeAPI.mintWorkspaceConnectTokenAuthenticatedAPI",
];

/// Runs the handler of `path`; `None` = not ported (or no handler state).
pub async fn call(
    state: &RpcState,
    path: &str,
    context: &RpcContext,
    input: &Value,
) -> Option<Result<Value, OrpcError>> {
    let api = state.api.as_ref()?;
    if !PORTED.contains(&path) {
        return crate::rpc_procedures::procedure(state, path, context, input).await;
    }
    let workspace = context.workspace.as_ref()?;
    let workspace_id = workspace.workspace_id;
    Some(match path {
        "questionnairesAPI.listQuestionnairesForFlowAPI" => {
            let keyword = match input.get("keyword") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(_) => return Some(Err(validation_failed())),
            };
            list_questionnaires_for_flow(api, workspace_id, keyword)
                .await
                .map_err(|error| business_error(error, LISTING))
        }
        "placesNearMeAPI.listPlaceCategoryOptionsAPI" => {
            list_place_category_options(api, workspace_id)
                .await
                .map_err(|error| business_error(error, LISTING))
        }
        _ => {
            let Some(secret) = state.realtime_secret.as_deref() else {
                return Some(Err(internal_error("REALTIME_BROADCAST_SECRET is not set")));
            };
            mint_member_connect_token(
                &api.db,
                secret,
                workspace_id,
                context.session.user_id,
                &workspace.permissions,
                context.request_issued_at_ms,
            )
            .await
            .map(|token| json!({ "token": token }))
            .map_err(|error| business_error(error, LISTING))
        }
    })
}

fn validation_failed() -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error
}

/// An RPC-encoded procedure error (`{ json: error.toJSON(), meta }`).
pub fn error_response(error: &OrpcError) -> Response {
    let mut body = json!({ "json": error.to_json() });
    if error.data.is_none() {
        body["meta"] = json!([[3, "data"]]);
    }
    let status = StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `questionnaireService.listForFlow({ workspaceId, keyword })`:
/// `[{ id, name }]` of the live questionnaires by name.
pub async fn list_questionnaires_for_flow(
    api: &ApiState,
    workspace_id: i64,
    keyword: Option<String>,
) -> Result<Value, BusinessError> {
    let rows =
        chatbotx_business::questionnaire::list_for_flow(&api.db, workspace_id, keyword).await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|(id, name)| json!({ "id": id.to_string(), "name": name }))
            .collect(),
    ))
}

/// `listPlaceCategoryOptionsAPI`: `{ data: [{ id, name }] }` in
/// `placeCategoryService.list` order.
pub async fn list_place_category_options(
    api: &ApiState,
    workspace_id: i64,
) -> Result<Value, BusinessError> {
    let categories =
        chatbotx_business::place_near_me::category::list(&api.db, workspace_id).await?;
    Ok(json!({
        "data": categories
            .into_iter()
            .map(|category| json!({ "id": category.id.to_string(), "name": category.name }))
            .collect::<Vec<_>>(),
    }))
}

/// `findPoll`: the poll (`pollOperations.get`), `None` when the service
/// answers 404; any other error is returned.
pub async fn find_poll(
    api: &ApiState,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Value>, BusinessError> {
    match poll_service::find_with_vote_counts(&api.db, workspace_id, id).await {
        Ok(poll) => Ok(Some(crate::handlers::polls::poll_json(&poll))),
        Err(BusinessError::Exception { status: 404, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// `mintMemberConnectToken`: the member's conversation scope signed into a
/// 60-second `member-connect` token.
pub async fn mint_member_connect_token(
    db: &Db,
    secret: &str,
    workspace_id: i64,
    user_id: i64,
    permissions: &Value,
    issued_at_ms: i64,
) -> Result<String, BusinessError> {
    let scope = conversation_member_scope(db, workspace_id, user_id, permissions).await?;
    sign_member_connect_token(
        &MemberConnect {
            workspace_id: workspace_id.to_string(),
            user_id: user_id.to_string(),
            chat_scope: scope.chat_scope,
            team_ids: scope.team_ids,
            issued_at_ms,
        },
        secret,
    )
    .map_err(|error| BusinessError::exception(error.to_string(), "INTERNAL_SERVER_ERROR", 500))
}

/// `ConversationMemberScope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMemberScope {
    pub chat_scope: ChatScope,
    pub team_ids: Vec<String>,
}

/// `hasWorkspacePermission(permissions, key)`.
fn has_permission(permissions: &Value, key: &str) -> bool {
    permissions.get("superAdmin") == Some(&Value::Bool(true))
        || permissions.get(key) == Some(&Value::Bool(true))
}

/// `conversationMemberScope.resolve`: `none` without contacts access, `all`
/// for a super admin or a member not limited to assigned contacts, else
/// `assigned` with the member's inbox team ids in the workspace.
pub async fn conversation_member_scope(
    db: &Db,
    workspace_id: i64,
    user_id: i64,
    permissions: &Value,
) -> Result<ConversationMemberScope, BusinessError> {
    if !(has_permission(permissions, "contacts")
        || has_permission(permissions, "onlyAssignedContacts"))
    {
        return Ok(ConversationMemberScope {
            chat_scope: ChatScope::None,
            team_ids: Vec::new(),
        });
    }
    if permissions.get("superAdmin") == Some(&Value::Bool(true))
        || permissions.get("onlyAssignedContacts") != Some(&Value::Bool(true))
    {
        return Ok(ConversationMemberScope {
            chat_scope: ChatScope::All,
            team_ids: Vec::new(),
        });
    }
    let team_ids = db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT m."inboxTeamId" FROM "InboxTeamMember" m
                   JOIN "InboxTeam" t ON t."id" = m."inboxTeamId"
                   WHERE t."workspaceId" = ?1 AND m."userId" = ?2"#,
            )?;
            let rows = stmt
                .query_map(params![workspace_id, user_id], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(BusinessError::from)?;
    Ok(ConversationMemberScope {
        chat_scope: ChatScope::Assigned,
        team_ids: team_ids.into_iter().map(|id| id.to_string()).collect(),
    })
}
