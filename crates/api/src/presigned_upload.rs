//! `POST /api/presigned-upload` (`apps/builder/src/app/api/presigned-upload/route.ts`,
//! `lib/upload/handlers.ts`, `features/import/schema/presign.ts`): the
//! builder session upload route. A signed-in user gets a presigned `PUT`
//! URL for a validated storage key and a pending `File` row.
//!
//! Answers as the original `serverErrorHandler`: a schema error is `422
//! { message: "Validation error", errors }`, a `ChatbotXException` is
//! `{ message, errors: [] }` with its status (400 by default); the route's
//! own refusals are `{ error }`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use chatbotx_business::import::{ImportType, config, import_upload_path, presigned_upload_url};
use chatbotx_db::repo::{self, patch::Patch};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::handlers::ApiState;
use crate::rpc::find_rpc_session;
use crate::session::AuthState;

/// `t("errors.superAdminRequired")` (en).
const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";

/// The state of the route: the session store, the handler state (db, files,
/// app URL) and `isCloud()`.
#[derive(Clone)]
pub struct UploadState {
    pub auth: Arc<AuthState>,
    pub api: Arc<ApiState>,
    pub is_cloud: bool,
}

/// The `/api/presigned-upload` router.
pub fn router(state: UploadState) -> Router {
    Router::new()
        .route("/api/presigned-upload", post(handle))
        .with_state(Arc::new(state))
}

/// `uploadTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadType {
    Import,
    Generic,
    AdsCampaignCreative,
}

/// `UploadHandlerInput`.
#[derive(Debug, Clone, Default)]
pub struct UploadHandlerInput {
    pub workspace_id: Option<String>,
    pub user_id: Option<String>,
    pub file_name: String,
    pub mime_type: String,
    pub sub_type: String,
    pub path: Option<String>,
}

/// `{ ok: false, error, status }` of an upload handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRefusal {
    pub error: String,
    pub status: u16,
}

/// `PRIVILEGED_PATH_SEGMENTS`.
const PRIVILEGED_PATH_SEGMENTS: &[&str] = &["/ads-campaign/creatives/"];

/// `ENCODED_PATH_SEPARATOR` (`/%2f|%5c/i`).
fn has_encoded_separator(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("%2f") || lower.contains("%5c")
}

fn refuse(error: impl Into<String>, status: u16) -> Result<String, UploadRefusal> {
    Err(UploadRefusal {
        error: error.into(),
        status,
    })
}

/// `getUploadHandler(type)(input)`: the storage key, or the refusal.
pub fn upload_path(kind: UploadType, input: &UploadHandlerInput) -> Result<String, UploadRefusal> {
    match kind {
        UploadType::Import => import_handler(input),
        UploadType::Generic => generic_handler(input),
        UploadType::AdsCampaignCreative => ads_campaign_creative_handler(input),
    }
}

fn import_handler(input: &UploadHandlerInput) -> Result<String, UploadRefusal> {
    let Some(workspace_id) = input.workspace_id.as_deref() else {
        return refuse("workspaceId is required for import", 400);
    };
    // The route's schema only lets an import type or `file` / `generic` /
    // `adsCampaignCreative` through; `getImportEntry` of a non-import type is
    // `undefined` and throws (a 400 from `serverErrorHandler`).
    let Some(import_type) = ImportType::parse(&input.sub_type) else {
        return refuse(
            "Cannot read properties of undefined (reading 'config')",
            400,
        );
    };
    let accepted = config(import_type)
        .accepted_extensions
        .iter()
        .any(|(mime, _)| *mime == input.mime_type);
    if !accepted {
        return refuse(format!("Unsupported MIME type: {}", input.mime_type), 400);
    }
    let workspace_id = workspace_id.parse::<i64>().unwrap_or_default();
    Ok(import_upload_path(
        import_type,
        workspace_id,
        &input.file_name,
    ))
}

fn generic_handler(input: &UploadHandlerInput) -> Result<String, UploadRefusal> {
    let Some(path) = input.path.as_deref() else {
        return refuse("Path is required for generic upload", 400);
    };
    if has_encoded_separator(path) {
        return refuse("Invalid path", 400);
    }
    if let Some(workspace_id) = input.workspace_id.as_deref() {
        let valid = path.starts_with(&format!("workspaces/{workspace_id}/"))
            || path.starts_with(&format!("public/space/{workspace_id}/"));
        if !valid {
            return refuse("Invalid path", 400);
        }
        let lower = path.to_lowercase();
        if PRIVILEGED_PATH_SEGMENTS
            .iter()
            .any(|segment| lower.contains(segment))
        {
            return refuse("Invalid path", 400);
        }
    } else if let Some(user_id) = input.user_id.as_deref() {
        return Ok(format!("public/platform/{user_id}/{path}"));
    } else {
        return refuse("Invalid path", 400);
    }
    Ok(path.to_string())
}

fn ads_campaign_creative_handler(input: &UploadHandlerInput) -> Result<String, UploadRefusal> {
    let (Some(path), Some(workspace_id)) = (input.path.as_deref(), input.workspace_id.as_deref())
    else {
        return refuse(
            "workspaceId and path are required for an ads-creative upload",
            400,
        );
    };
    if !path.starts_with(&format!(
        "public/space/{workspace_id}/ads-campaign/creatives/"
    )) {
        return refuse("Invalid path", 400);
    }
    Ok(path.to_string())
}

/// `presignImportUploadRequest` after parsing.
#[derive(Debug, Clone)]
struct Request {
    kind: UploadType,
    sub_type: String,
    workspace_id: Option<String>,
    path: Option<String>,
    file_name: String,
    mime_type: String,
}

/// `presignImportUploadRequest.parse(body)`: the request, or the zod issues
/// (`{ field, message }`).
fn parse_request(body: &Value) -> Result<Request, Vec<Value>> {
    let mut issues = Vec::new();
    let issue = |issues: &mut Vec<Value>, field: &str, message: &str| {
        issues.push(json!({ "field": field, "message": message }));
    };
    let Some(object) = body.as_object() else {
        return Err(vec![
            json!({ "field": "", "message": "Invalid input: expected object" }),
        ]);
    };
    let text = |key: &str| object.get(key).and_then(Value::as_str);
    let kind = match text("type") {
        Some("import") => Some(UploadType::Import),
        Some("generic") => Some(UploadType::Generic),
        Some("adsCampaignCreative") => Some(UploadType::AdsCampaignCreative),
        _ => {
            issue(&mut issues, "type", "Invalid option");
            None
        }
    };
    let sub_type = match text("subType") {
        Some(value)
            if ImportType::parse(value).is_some()
                || matches!(value, "file" | "generic" | "adsCampaignCreative") =>
        {
            Some(value.to_string())
        }
        _ => {
            issue(&mut issues, "subType", "Invalid input");
            None
        }
    };
    let workspace_id = match object.get("workspaceId") {
        None => None,
        Some(Value::String(value))
            if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(value.clone())
        }
        Some(_) => {
            issue(&mut issues, "workspaceId", "Invalid string");
            None
        }
    };
    let path = match object.get("path") {
        None => None,
        Some(Value::String(value)) => {
            let length = value.encode_utf16().count();
            if !(1..=255).contains(&length) {
                issue(&mut issues, "path", "Invalid length");
            } else if value.contains("..") || value.starts_with('/') || has_encoded_separator(value)
            {
                issue(&mut issues, "path", "Invalid path");
            }
            Some(value.clone())
        }
        Some(_) => {
            issue(&mut issues, "path", "Invalid input: expected string");
            None
        }
    };
    let mut bounded = |key: &str| match object.get(key) {
        Some(Value::String(value)) if (1..=255).contains(&value.encode_utf16().count()) => {
            Some(value.clone())
        }
        _ => {
            issue(&mut issues, key, "Invalid input");
            None
        }
    };
    let file_name = bounded("fileName");
    let mime_type = bounded("mimeType");
    match (kind, sub_type, file_name, mime_type) {
        (Some(kind), Some(sub_type), Some(file_name), Some(mime_type)) if issues.is_empty() => {
            Ok(Request {
                kind,
                sub_type,
                workspace_id,
                path,
                file_name,
                mime_type,
            })
        }
        _ => Err(issues),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    crate::handlers::json(status.as_u16(), json!({ "error": message }))
}

/// `serverErrorHandler` of a `ChatbotXException` / `Error`.
fn exception(status: StatusCode, message: &str) -> Response {
    crate::handlers::json(status.as_u16(), json!({ "message": message, "errors": [] }))
}

fn internal(error: impl std::fmt::Display) -> Response {
    tracing::error!(error = %error, "presigned upload failed");
    exception(StatusCode::INTERNAL_SERVER_ERROR, "Unknown error")
}

/// The caller's membership in a workspace: `(scheduledDeletionAt, permissions)`
/// of the workspace and the member row (the workspace may be gone: `None`).
#[derive(Debug)]
struct Membership {
    workspace_exists: bool,
    scheduled_deletion_at: Option<i64>,
    permissions: Option<String>,
}

async fn membership(
    state: &ApiState,
    workspace_id: i64,
    user_id: i64,
) -> Result<Membership, chatbotx_db::DbError> {
    state
        .db
        .read(move |conn| {
            let workspace: Option<Option<i64>> = conn
                .query_row(
                    r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                    params![workspace_id],
                    |row| row.get(0),
                )
                .optional()?;
            let permissions: Option<String> = conn
                .query_row(
                    r#"SELECT "permissions" FROM "WorkspaceMember"
                       WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                    params![workspace_id, user_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(Membership {
                workspace_exists: workspace.is_some(),
                scheduled_deletion_at: workspace.flatten(),
                permissions,
            })
        })
        .await
}

/// `isPlatformAdmin(user)`: cloud = the owner of an active tenant, else a
/// `PlatformAdmin` row.
async fn is_platform_admin(
    state: &ApiState,
    user_id: i64,
    is_cloud: bool,
) -> Result<bool, chatbotx_db::DbError> {
    state
        .db
        .read(move |conn| {
            let sql = if is_cloud {
                r#"SELECT 1 FROM "Tenant" WHERE "ownerId" = ?1 AND "status" = 'active' LIMIT 1"#
            } else {
                r#"SELECT 1 FROM "PlatformAdmin" WHERE "userId" = ?1"#
            };
            Ok(conn
                .query_row(sql, params![user_id], |_| Ok(()))
                .optional()?
                .is_some())
        })
        .await
}

async fn handle(
    State(state): State<Arc<UploadState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let input = match parse_request(&body) {
        Ok(input) => input,
        Err(errors) => {
            return crate::handlers::json(
                422,
                json!({ "message": "Validation error", "errors": errors }),
            );
        }
    };
    let session = match find_rpc_session(&state.auth, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "Unauthorized"),
        Err(e) => return internal(e),
    };
    let api = &state.api;
    if let Some(workspace_id) = input.workspace_id.as_deref() {
        // ponytail: membership rows only; support access (synthetic membership
        // of a platform admin) comes with the workspace-access port.
        let found = match membership(api, workspace_id.parse().unwrap_or(-1), session.user_id).await
        {
            Ok(found) => found,
            Err(e) => return internal(e),
        };
        let Some(permissions) = found.permissions.as_deref() else {
            return exception(
                StatusCode::BAD_REQUEST,
                "User is not associated with this workspace",
            );
        };
        if !found.workspace_exists || found.scheduled_deletion_at.is_some() {
            return error(StatusCode::FORBIDDEN, "Workspace deletion scheduled");
        }
        let targets_ads_creative = input.kind == UploadType::AdsCampaignCreative
            || input.sub_type == "adsCampaignCreative";
        if targets_ads_creative {
            let permissions = serde_json::from_str::<Value>(permissions).unwrap_or(Value::Null);
            if permissions.get("superAdmin") != Some(&Value::Bool(true)) {
                return exception(StatusCode::BAD_REQUEST, SUPER_ADMIN_REQUIRED);
            }
        }
    } else {
        if input.kind == UploadType::Import {
            return error(
                StatusCode::BAD_REQUEST,
                "workspaceId is required for import uploads",
            );
        }
        let own_avatar = input
            .path
            .as_deref()
            .is_some_and(|path| path.starts_with("avatar/"));
        if own_avatar {
            if !input.mime_type.starts_with("image/") {
                return error(StatusCode::BAD_REQUEST, "Avatar uploads must be an image");
            }
        } else {
            match is_platform_admin(api, session.user_id, state.is_cloud).await {
                Ok(true) => {}
                Ok(false) => return error(StatusCode::FORBIDDEN, "Forbidden"),
                Err(e) => return internal(e),
            }
        }
    }
    let storage_url = crate::handlers::contacts::tenant_urls(api).storage_url;
    let handler_input = UploadHandlerInput {
        workspace_id: input.workspace_id.clone(),
        user_id: Some(session.user_id.to_string()),
        file_name: input.file_name.clone(),
        mime_type: input.mime_type.clone(),
        sub_type: input.sub_type.clone(),
        path: input.path.clone(),
    };
    let path = match upload_path(input.kind, &handler_input) {
        Ok(path) => path,
        Err(refusal) => {
            let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::BAD_REQUEST);
            return error(status, &refusal.error);
        }
    };
    let Some(files) = api.files.as_deref() else {
        return internal("the file store is off");
    };
    let presigned_post_url = match presigned_upload_url(files, &path).await {
        Ok(url) => url,
        Err(e) => return internal(e),
    };
    let public_url = chatbotx_core::utils::storage::get_public_file_url(&path, &storage_url)
        .unwrap_or_else(|_| format!("{storage_url}{path}"));
    let context_type = if input.kind == UploadType::Import {
        "import"
    } else {
        "generic"
    };
    let patch: Patch = vec![
        (
            "workspaceId",
            input
                .workspace_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                .map_or(Sql::Null, Sql::Integer),
        ),
        ("userId", Sql::Integer(session.user_id)),
        ("contextType", Sql::Text(context_type.into())),
        ("subType", Sql::Text(input.sub_type.clone())),
        ("path", Sql::Text(path.clone())),
        ("fileName", Sql::Text(input.file_name.clone())),
        ("mimeType", Sql::Text(input.mime_type.clone())),
        ("status", Sql::Text("pending".into())),
    ];
    let file = match api
        .db
        .write(move |conn| {
            repo::file::create(conn, &patch).map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
    {
        Ok(file) => file,
        Err(e) => return internal(e),
    };
    crate::handlers::json(
        200,
        json!({
            "fileId": file.id.to_string(),
            "presignedPostUrl": presigned_post_url,
            "publicUrl": public_url,
            "path": path,
        }),
    )
}
