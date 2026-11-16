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
use chatbotx_db::Db;
use serde_json::{Value, json};

use super::{RpcContext, RpcState};
use crate::handlers::ApiState;
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
    let _ = (state, path, context, input);
    todo!("P3.5f")
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
    let _ = (api, workspace_id, keyword);
    todo!("P3.5f")
}

/// `listPlaceCategoryOptionsAPI`: `{ data: [{ id, name }] }` in
/// `placeCategoryService.list` order.
pub async fn list_place_category_options(
    api: &ApiState,
    workspace_id: i64,
) -> Result<Value, BusinessError> {
    let _ = (api, workspace_id);
    todo!("P3.5f")
}

/// `findPoll`: the poll (`pollOperations.get`), `None` when the service
/// answers 404; any other error is returned.
pub async fn find_poll(
    api: &ApiState,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Value>, BusinessError> {
    let _ = (api, workspace_id, id);
    todo!("P3.5f")
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
    let _ = (db, secret, workspace_id, user_id, permissions, issued_at_ms);
    todo!("P3.5f")
}
