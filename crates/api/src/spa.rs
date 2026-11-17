//! `/rpc/pages/{name}`: the page loaders of the SPA (`apps/web`). The builder
//! read this data on the server in its layouts and pages; the SPA asks the
//! Rust server for it (conversion/specs/P4-web.md §5.2).
//!
//! `/rpc/actions/{name}` (the "use server" actions, §5.1) run in the `/rpc`
//! router (`crate::rpc_procedures::action`).
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

/// The router of `/rpc/pages/*`.
pub fn spa_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/rpc/pages/{name}", post(page))
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
        "channelsCreate" => match require_session(&state, &headers).await {
            Ok(session) => channels_create_page(&state, session.user_id, &input).await,
            Err(response) => *response,
        },
        "invitation" => invitation_page(&state, &headers, &input).await,
        "workspace" | "contactsAccess" | "folder" | "flow" | "aiAgents" | "settingsChannels" => {
            let session = match require_session(&state, &headers).await {
                Ok(session) => session,
                Err(response) => return *response,
            };
            workspace_page(&state, &name, session.user_id, input).await
        }
        _ => error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found"),
    }
}

/// `app/(no-sidebar)/invitations/[code]/page.tsx`: `findInvitation` (null
/// for a rejected code) and whether a user is signed in. No session needed.
async fn invitation_page(state: &AuthState, headers: &HeaderMap, input: &Value) -> Response {
    let signed_in = match find_rpc_session(state, headers).await {
        Ok(session) => session.is_some(),
        Err(err) => return internal(err),
    };
    let code = input
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match chatbotx_business::invitation::details(&state.db, code).await {
        Ok(found) => ok(json!({
            "invitation": found.map(|i| json!({
                "code": i.code,
                "expiresAt": iso8601::format_millis(u64::try_from(i.expires_at).unwrap_or_default()),
                "workspace": {
                    "name": i.workspace_name,
                    "logo": i.workspace_logo,
                    "scheduledDeletionAt": i
                        .scheduled_deletion_at
                        .map(|at| iso8601::format_millis(u64::try_from(at).unwrap_or_default())),
                },
                "inviter": { "name": i.inviter_name, "image": i.inviter_image },
            })),
            "isAuthenticated": signed_in,
        })),
        Err(err) => internal(err),
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
            let label: String = tx.query_row(
                r#"SELECT COALESCE(NULLIF("name", ''), "email") FROM "User" WHERE "id" = ?1"#,
                params![user_id],
                |row| row.get(0),
            )?;
            let workspace_id = insert_workspace(tx, user_id, &format!("{label}'s workspace"))?;
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

/// `workspaceService.create`: a workspace owned by `user_id` (tenant of the
/// user) and its owner membership with full permissions.
pub(crate) fn insert_workspace(tx: &Connection, user_id: i64, name: &str) -> Result<i64, DbError> {
    let tenant_id: i64 = tx.query_row(
        r#"SELECT "tenantId" FROM "User" WHERE "id" = ?1"#,
        params![user_id],
        |row| row.get(0),
    )?;
    let workspace_id = repo::new_id();
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (?1, ?2, ?3, ?4)"#,
        params![workspace_id, name, user_id, tenant_id],
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
    Ok(workspace_id)
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
        // `requireContactsAccess`: no contact scope = the not-found page.
        "contactsAccess"
            if !(flag(permissions, "superAdmin")
                || flag(permissions, "contacts")
                || flag(permissions, "onlyAssignedContacts")) =>
        {
            error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found")
        }
        "contactsAccess" => ok(json!({
            "canViewEmailAndPhone": flag(permissions, "superAdmin") || flag(permissions, "emailAndPhone"),
        })),
        "folder" => folder_page(state, workspace_id, &input).await,
        "flow" if flag(permissions, "superAdmin") || flag(permissions, "flows") => {
            flow_page(state, workspace_id, &input).await
        }
        "flow" => error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found"),
        // `resolveGuardedWorkspaceId(params, "flows")` of the `(ai)` layout.
        "aiAgents" if flag(permissions, "superAdmin") || flag(permissions, "flows") => {
            read_page(state, move |conn| ai_agents_page(conn, workspace_id)).await
        }
        "aiAgents" => error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found"),
        // `resolveGuardedWorkspaceId(params, "superAdmin")` of the settings layout.
        "settingsChannels" if flag(permissions, "superAdmin") => {
            read_page(state, move |conn| {
                settings_channels_page(conn, workspace_id)
            })
            .await
        }
        "settingsChannels" => error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found"),
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

/// `channels/create/page.tsx`: with a `workspaceId` the caller must be its
/// super admin (`requireWorkspacePermission`).
// ponytail: tenant channel visibility and platform credentials are not ported;
// only the channels with a ported create action (webchat, API) are offered.
async fn channels_create_page(state: &AuthState, user_id: i64, input: &Value) -> Response {
    if let Some(workspace_id) = workspace_id(input) {
        let list = match state.db.read(move |conn| memberships(conn, user_id)).await {
            Ok(list) => list,
            Err(err) => return internal(err),
        };
        let allowed = list.iter().any(|m| {
            m.workspace["id"] == Value::String(workspace_id.to_string())
                && flag(&m.permissions, "superAdmin")
        });
        if !allowed {
            return unauthorized();
        }
    }
    ok(json!({ "offeredChannels": ["webchat", "api"], "configuredChannels": [] }))
}

/// `flows/[id]/page.tsx`: the `FlowDetail` props. 404 = `notFound()`.
async fn flow_page(state: &AuthState, workspace_id: i64, input: &Value) -> Response {
    let not_found = || error(StatusCode::NOT_FOUND, "NOT_FOUND", "Page not found");
    let Some(id) = input
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<i64>().ok())
    else {
        return not_found();
    };
    let flow = match chatbotx_business::flow::find_by_id(&state.db, workspace_id, id).await {
        Ok(flow) => flow,
        Err(_) => return not_found(),
    };
    let Some(draft) = flow.flow_versions.iter().find(|v| v.is_draft) else {
        return not_found();
    };
    let published = flow
        .flow_versions
        .iter()
        .find(|v| v.is_latest && !v.is_draft);
    let can_revert = published.is_some_and(|p| {
        flow_content(&draft.nodes.0, &draft.edges.0) != flow_content(&p.nodes.0, &p.edges.0)
    });
    let locked = chatbotx_business::tx::read(&state.db, move |conn| {
        Ok(repo::template_installed_resource::find_blocking(
            conn,
            workspace_id,
            "flow",
            &[id],
            repo::template_installed_resource::TemplateResourcePermission::AllowEdit,
            &[],
        )?)
    })
    .await;
    let target_country = state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
                params![workspace_id],
                |row| row.get::<_, Option<String>>(0),
            )?)
        })
        .await;
    let compatible =
        chatbotx_business::integration_openai_compatible::list(&state.db, workspace_id).await;
    let (Ok(locked), Ok(target_country), Ok(compatible)) = (locked, target_country, compatible)
    else {
        return internal("flow page read failed");
    };
    let mut flow_json = flow.flow.to_json();
    flow_json["flowVersions"] = flow.flow_versions.iter().map(|v| v.to_json()).collect();
    ok(json!({
        "flow": flow_json,
        "flowVersion": draft.to_json(),
        "hasPublishedVersion": published.is_some(),
        "canRevertToPublished": can_revert,
        "isTemplateLocked": !locked.is_empty(),
        "openaiCompatibleIntegrations": compatible
            .iter()
            .map(crate::rpc_procedures::integrations::openai_compatible_json)
            .collect::<Vec<_>>(),
        "workspaceTargetCountry": target_country,
    }))
}

/// `serializeFlowContent` (`flow-version-content.ts`): the nodes without the
/// editor-only keys, the edges with their five link keys.
fn flow_content(nodes: &Value, edges: &Value) -> (Vec<Value>, Vec<Value>) {
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let nodes = list(nodes)
        .into_iter()
        .map(|mut node| {
            if let Some(object) = node.as_object_mut() {
                for key in ["selected", "dragging", "width", "height"] {
                    object.remove(key);
                }
                if let Some(Value::Object(data)) = object.get_mut("data") {
                    data.remove("forceToolbarVisible");
                }
            }
            node
        })
        .collect();
    let edges = list(edges)
        .into_iter()
        .map(|edge| {
            let mut out = Map::new();
            for key in ["id", "source", "sourceHandle", "target", "targetHandle"] {
                if let Some(value) = edge.get(key) {
                    out.insert(key.to_string(), value.clone());
                }
            }
            Value::Object(out)
        })
        .collect();
    (nodes, edges)
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

async fn read_page(
    state: &AuthState,
    f: impl FnOnce(&Connection) -> Result<Value, chatbotx_db::rusqlite::Error> + Send + 'static,
) -> Response {
    match state.db.read(move |conn| Ok(f(conn)?)).await {
        Ok(value) => ok(value),
        Err(err) => internal(err),
    }
}

/// `(ai)/ai-agents/page.tsx`: the agent action options (published flows,
/// active tags, custom fields, admins, inbox teams) as `{ label, value }`.
fn ai_agents_page(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Value, chatbotx_db::rusqlite::Error> {
    let options = |sql: &str| -> Result<Vec<Value>, chatbotx_db::rusqlite::Error> {
        let mut statement = conn.prepare(sql)?;
        let rows = statement.query_map(params![workspace_id], |row| {
            let mut option = json!({
                "label": row.get::<_, String>(1)?,
                "value": row.get::<_, i64>(0)?.to_string(),
            });
            if row.as_ref().column_count() > 2 {
                option["type"] = json!(row.get::<_, String>(2)?);
            }
            Ok(option)
        })?;
        rows.collect()
    };
    // `flowRepository.listPublishedOptions`.
    let flows = options(
        r#"SELECT "id", "name" FROM "Flow" WHERE "workspaceId" = ?1 AND "active" = 1
           AND "currentVersionId" IS NOT NULL ORDER BY "id""#,
    )?;
    let tags = options(
        r#"SELECT "id", "name" FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL ORDER BY "name""#,
    )?;
    let custom_fields = options(
        r#"SELECT "id", "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#,
    )?;
    // `isWorkspaceAdminMember`: the owner or a `superAdmin` member.
    let admins = options(
        r#"SELECT u."id", COALESCE(u."name", u."email") FROM "WorkspaceMember" m
           JOIN "User" u ON u."id" = m."userId"
           WHERE m."workspaceId" = ?1
             AND (m."role" = 'owner' OR json_extract(m."permissions", '$.superAdmin') = 1)
           ORDER BY m."createdAt", m."id""#,
    )?;
    let inbox_teams = options(
        r#"SELECT "id", "name" FROM "InboxTeam" WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
    )?;
    Ok(json!({
        "flows": flows,
        "tags": tags,
        "customFields": custom_fields,
        "admins": admins,
        "inboxTeams": inbox_teams,
    }))
}

/// `resolveChannelPolicy` for the settings channels pages: the channels the
/// owner may create and the connected ones (`distinctConnectedChannels`);
/// the SPA builds `visibleChannels` from them as the builder did.
// ponytail: tenant channel visibility is not ported; the creatable channels
// are the ones with a ported connect path.
fn settings_channels_page(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Value, chatbotx_db::rusqlite::Error> {
    let mut statement = conn.prepare(
        r#"SELECT DISTINCT "channel" FROM "Inbox" WHERE "workspaceId" = ?1 AND "status" = 'connected' ORDER BY "channel""#,
    )?;
    let connected = statement
        .query_map(params![workspace_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "creatable": ["webchat", "api", "telegram"], "connected": connected }))
}
