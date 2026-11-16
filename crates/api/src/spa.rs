//! `/rpc/pages/{name}`: the page loaders of the SPA (`apps/web`). The builder
//! read this data on the server in its layouts and pages; the SPA asks the
//! Rust server for it (conversion/specs/P4-web.md §5.2).
//!
//! `/rpc/actions/{name}`: the "use server" actions (§5.1). None is ported
//! yet: after auth a call answers `NOT_FOUND "Action not ported yet"`.
//!
//! Request and response use the oRPC RPC encoding (`{ json, meta }`).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chatbotx_db::rusqlite::types::ValueRef;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};
use chatbotx_db::{DbError, repo};
use serde_json::{Map, Value, json};

use crate::iso8601;
use crate::rpc::{RpcSession, find_rpc_session};
use crate::session::AuthState;

/// The router of `/rpc/pages/*` and `/rpc/actions/*`.
pub fn spa_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/rpc/pages/{name}", post(page))
        .route("/rpc/actions/{name}", post(action))
        .with_state(state)
}

/// Every `WorkspaceMemberPermissions` flag (`FULL_WORKSPACE_MEMBER_PERMISSIONS`).
const FULL_PERMISSIONS: &str = r#"{"superAdmin":true,"analytics":true,"flows":true,"contacts":true,"onlyAssignedContacts":true,"emailAndPhone":true,"broadcast":true,"ecommerce":true}"#;
/// `DEFAULT_MEMBER_NOTIFICATION_TYPES` / `_CHANNELS` (workspace/service.ts).
const NOTIFICATION_TYPES: &str = r#"{"notifyAdmin":true,"newMessageToHuman":true,"newOrder":true}"#;
const NOTIFICATION_CHANNELS: &str =
    r#"{"messenger":true,"email":true,"telegram":true,"browser":true}"#;
/// Integer columns that hold booleans.
const BOOL_COLUMNS: &[&str] = &[
    "developmentMode",
    "isActive",
    "capiLimitedDataUse",
    "isTrash",
    "emailVerified",
];
/// Text columns that hold JSON.
const JSON_COLUMNS: &[&str] = &["paths", "permissions"];

fn rpc_json(status: StatusCode, json: Value, meta: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "json": json, "meta": meta }).to_string(),
    )
        .into_response()
}

fn ok(value: Value) -> Response {
    rpc_json(StatusCode::OK, value, json!([]))
}

/// An oRPC error (`data` undefined), as `rpc.rs` encodes it.
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    rpc_json(
        status,
        json!({ "defined": false, "code": code, "status": status.as_u16(), "message": message }),
        json!([[3, "data"]]),
    )
}

fn internal(err: impl std::fmt::Display) -> Response {
    tracing::error!(error = %err, "SPA loader failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_SERVER_ERROR",
        "Internal server error",
    )
}

fn unauthorized() -> Response {
    error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "Unauthorized")
}

/// The signed-in caller, or the error response.
async fn require_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<RpcSession, Box<Response>> {
    match find_rpc_session(state, headers).await {
        Ok(Some(session)) if session.must_change_password => Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Password change required",
        ))),
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(Box::new(unauthorized())),
        Err(err) => Err(Box::new(internal(err))),
    }
}

/// `input.workspaceId` (a string id) as an integer.
fn workspace_id(input: &Value) -> Option<i64> {
    input.get("workspaceId")?.as_str()?.parse().ok()
}

/// A row as the JSON of the builder resources: ids as strings, dates as ISO
/// strings, flags as booleans, JSON text parsed.
fn row_json(row: &Row<'_>) -> Result<Value, chatbotx_db::rusqlite::Error> {
    let statement = row.as_ref();
    let mut object = Map::new();
    for index in 0..statement.column_count() {
        let name = statement.column_name(index)?;
        let value = match row.get_ref(index)? {
            ValueRef::Null | ValueRef::Blob(_) => Value::Null,
            ValueRef::Integer(int) if BOOL_COLUMNS.contains(&name) => Value::Bool(int != 0),
            ValueRef::Integer(int) if name.ends_with("At") || name.ends_with("Until") => {
                Value::String(iso8601::format_millis(
                    u64::try_from(int).unwrap_or_default(),
                ))
            }
            ValueRef::Integer(int) if name == "id" || name.ends_with("Id") => {
                Value::String(int.to_string())
            }
            ValueRef::Integer(int) => json!(int),
            ValueRef::Real(real) => json!(real),
            ValueRef::Text(text) => {
                let text = String::from_utf8_lossy(text).into_owned();
                if JSON_COLUMNS.contains(&name) {
                    ids_as_strings(serde_json::from_str(&text).unwrap_or(Value::Null))
                } else {
                    Value::String(text)
                }
            }
        };
        object.insert(name.to_string(), value);
    }
    Ok(Value::Object(object))
}

/// Folder `paths` hold ids: numbers become strings.
fn ids_as_strings(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| match item {
                    Value::Number(number) => Value::String(number.to_string()),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}

/// The workspace columns of `workspaceResource` (no `token`).
const WORKSPACE_COLUMNS: &str = r#"w."id", w."createdAt", w."updatedAt", w."name", w."defaultReply",
    w."defaultReplyFrequency", w."targetCountry", w."language", w."timezone", w."brandColor",
    w."developmentMode", w."smartResponseDelaySeconds", w."isActive", w."startTime", w."endTime",
    w."logo", w."scheduledDeletionAt", w."deletionRequestedById", w."deletionRequestedAt",
    w."purgeStartedAt", w."supportAccessUntil", w."capiLimitedDataUse", w."ownerId", w."tenantId""#;

/// The caller's workspaces with the membership role and permissions.
struct Membership {
    workspace: Value,
    role: String,
    permissions: Value,
}

fn memberships(conn: &Connection, user_id: i64) -> Result<Vec<Membership>, DbError> {
    let sql = format!(
        r#"SELECT {WORKSPACE_COLUMNS}, m."role", m."permissions"
           FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
           WHERE m."userId" = ?1 ORDER BY w."createdAt", w."id""#
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params![user_id], |row| {
        let mut workspace = row_json(row)?;
        let object = workspace
            .as_object_mut()
            .expect("row_json returns an object");
        let role = object.remove("role").unwrap_or(Value::Null);
        let permissions = object.remove("permissions").unwrap_or_else(|| json!({}));
        Ok(Membership {
            workspace,
            role: role.as_str().unwrap_or_default().to_string(),
            permissions,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn flag(permissions: &Value, key: &str) -> bool {
    permissions
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn user_json(conn: &Connection, user_id: i64) -> Result<Option<Value>, DbError> {
    Ok(conn
        .query_row(
            r#"SELECT "id", "name", "email", "image", "emailVerified", "mustChangePassword"
               FROM "User" WHERE "id" = ?1"#,
            params![user_id],
            |row| {
                let mut user = row_json(row)?;
                user["mustChangePassword"] = Value::Bool(row.get::<_, bool>(5)?);
                Ok(user)
            },
        )
        .optional()?)
}

fn is_community() -> bool {
    let edition = std::env::var("NEXT_PUBLIC_EDITION").unwrap_or_default();
    edition.is_empty() || edition == "community"
}

/// `QuotaSummary` with no plan (community, no billing).
fn empty_quota() -> Value {
    json!({ "metrics": [], "planName": null, "planStatus": null, "trialEndsAt": null })
}

async fn page(
    State(state): State<Arc<AuthState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let input = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null);
    match name.as_str() {
        "session" => session_page(&state, &headers).await,
        // ponytail: no OAuth providers in the Rust auth yet; add them with the providers.
        "authOptions" => ok(json!({
            "enabledProviders": [],
            "genericOAuth": null,
            "signUpAllowed": !state.settings.sign_up_disabled,
        })),
        "home" => match require_session(&state, &headers).await {
            Ok(session) => home_page(&state, session.user_id).await,
            Err(response) => *response,
        },
        // ponytail: deletion records are a P3.x port; the list is empty until then.
        "deletedWorkspaces" => match require_session(&state, &headers).await {
            Ok(_) => ok(json!({ "data": [] })),
            Err(response) => *response,
        },
        "workspace" | "contactsAccess" | "folder" => {
            let session = match require_session(&state, &headers).await {
                Ok(session) => session,
                Err(response) => return *response,
            };
            workspace_page(&state, &name, session.user_id, input).await
        }
        _ => error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found"),
    }
}

/// `pages.session`: the user (or null). The tenant settings are the SPA
/// defaults until white-label tenants are ported (`tenant: null`).
async fn session_page(state: &AuthState, headers: &HeaderMap) -> Response {
    let session = match find_rpc_session(state, headers).await {
        Ok(session) => session,
        Err(err) => return internal(err),
    };
    let Some(session) = session else {
        return ok(json!({ "user": null, "tenant": null }));
    };
    match state
        .db
        .read(move |conn| user_json(conn, session.user_id))
        .await
    {
        Ok(user) => ok(json!({ "user": user, "tenant": null })),
        Err(err) => internal(err),
    }
}

/// `app/(main)/page.tsx`: the workspace list. A user without a workspace
/// gets a first one (`workspaceService.create`), as the builder did.
async fn home_page(state: &AuthState, user_id: i64) -> Response {
    let created = state
        .db
        .write(move |tx| {
            let count: i64 = tx.query_row(
                r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "userId" = ?1"#,
                params![user_id],
                |row| row.get(0),
            )?;
            if count > 0 {
                return Ok(None);
            }
            let (label, tenant_id): (String, i64) = tx.query_row(
                r#"SELECT COALESCE(NULLIF("name", ''), "email"), "tenantId" FROM "User" WHERE "id" = ?1"#,
                params![user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let workspace_id = repo::new_id();
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (?1, ?2, ?3, ?4)"#,
                params![workspace_id, format!("{label}'s workspace"), user_id, tenant_id],
            )?;
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
                   VALUES (?1, ?2, ?3, 'owner', ?4, ?5, ?6)"#,
                params![
                    repo::new_id(),
                    workspace_id,
                    user_id,
                    FULL_PERMISSIONS,
                    NOTIFICATION_TYPES,
                    NOTIFICATION_CHANNELS
                ],
            )?;
            Ok(Some(workspace_id))
        })
        .await;
    let created = match created {
        Ok(created) => created,
        Err(err) => return internal(err),
    };
    let loaded = state
        .db
        .read(move |conn| Ok((user_json(conn, user_id)?, memberships(conn, user_id)?)))
        .await;
    let (Some(user), list) = (match loaded {
        Ok(loaded) => loaded,
        Err(err) => return internal(err),
    }) else {
        return unauthorized();
    };
    let ids_where = |keep: &dyn Fn(&Membership) -> bool| -> Vec<Value> {
        list.iter()
            .filter(|m| keep(m))
            .map(|m| m.workspace["id"].clone())
            .collect()
    };
    let owner_ids = ids_where(&|m| m.role == "owner");
    let super_admin_ids = ids_where(&|m| flag(&m.permissions, "superAdmin"));
    // `COMMUNITY_MAX_WORKSPACES = 1`.
    let is_at_limit = is_community() && !owner_ids.is_empty();
    ok(json!({
        "user": user,
        "createdWorkspaceId": created.map(|id| id.to_string()),
        "workspaces": list.iter().map(|m| m.workspace.clone()).collect::<Vec<_>>(),
        "ownerWorkspaceIds": owner_ids,
        "superAdminWorkspaceIds": super_admin_ids,
        "blocked": false,
        "reason": null,
        "isAtLimit": is_at_limit,
        "canAccessManage": false,
        "isPlatformContext": false,
        "isSuperAdmin": false,
        "metrics": [],
        "planName": null,
        "planStatus": null,
        "trialEndsAt": null,
    }))
}

/// The loaders behind the workspace membership check
/// (`workspaceAuthorizedMidddleware` / the `space/[workspaceId]` layout).
async fn workspace_page(state: &AuthState, name: &str, user_id: i64, input: Value) -> Response {
    let Some(workspace_id) = workspace_id(&input) else {
        return unauthorized();
    };
    let list = match state.db.read(move |conn| memberships(conn, user_id)).await {
        Ok(list) => list,
        Err(err) => return internal(err),
    };
    let Some(current) = list
        .iter()
        .find(|m| m.workspace["id"] == Value::String(workspace_id.to_string()))
    else {
        return unauthorized();
    };
    let permissions = &current.permissions;
    match name {
        // `requireContactsAccess` → `buildContactPermissionScope`.
        "contactsAccess" => ok(json!({
            "canViewEmailAndPhone": flag(permissions, "superAdmin") || flag(permissions, "emailAndPhone"),
        })),
        "folder" => folder_page(state, workspace_id, &input).await,
        _ => ok(json!({
            "allWorkspaces": list.iter().map(|m| m.workspace.clone()).collect::<Vec<_>>(),
            "permissions": permissions,
            "isSupportSession": false,
            "scheduledForDeletion": !current.workspace["scheduledDeletionAt"].is_null(),
            "supportAccessUntil": current.workspace["supportAccessUntil"],
            "consentRequired": false,
            "canAccessManage": false,
            "isSuperAdmin": false,
            "blocked": false,
            "blockReason": null,
            "quota": empty_quota(),
            "tokenRefreshErrors": [],
            "hasCallCapableChannel": false,
        })),
    }
}

/// `getCurrentFolder` → `folderService.getWithParents`.
async fn folder_page(state: &AuthState, workspace_id: i64, input: &Value) -> Response {
    let Some(id) = input
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<i64>().ok())
    else {
        return ok(json!({ "folder": null, "parents": [] }));
    };
    let loaded = state
        .db
        .read(move |conn| {
            let folder = conn
                .query_row(
                    r#"SELECT * FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, workspace_id],
                    row_json,
                )
                .optional()?;
            let Some(folder) = folder else {
                return Ok(json!({ "folder": null, "parents": [] }));
            };
            let mut parents = Vec::new();
            for parent_id in folder["paths"].as_array().into_iter().flatten() {
                let Some(parent_id) = parent_id.as_str().and_then(|p| p.parse::<i64>().ok()) else {
                    continue;
                };
                if let Some(parent) = conn
                    .query_row(
                        r#"SELECT * FROM "Folder" WHERE "id" = ?1"#,
                        params![parent_id],
                        row_json,
                    )
                    .optional()?
                {
                    parents.push(parent);
                }
            }
            Ok(json!({ "folder": folder, "parents": parents }))
        })
        .await;
    match loaded {
        Ok(value) => ok(value),
        Err(err) => internal(err),
    }
}

/// `/rpc/actions/{name}`: auth, then not ported.
async fn action(
    State(state): State<Arc<AuthState>>,
    Path(_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_session(&state, &headers).await {
        return *response;
    }
    error(StatusCode::NOT_FOUND, "NOT_FOUND", "Action not ported yet")
}
