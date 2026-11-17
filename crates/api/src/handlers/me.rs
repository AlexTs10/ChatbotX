//! `features/me/api/public.ts` (`mePublicRouter`, Q.A1 §3): the mobile-user
//! routes. Only a mobile access token reaches them
//! ([`crate::auth::authenticate_mobile_user`]); they read and write the
//! signed-in user's own rows only.

use axum::http::{HeaderMap, HeaderValue, header};
use chatbotx_business::BusinessError;
use chatbotx_db::repo::now_ms;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::{ApiState, Call, LISTING, MUTATING, Reply, business_error, json};
use crate::auth::MobileUser;
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "me.get",
    "me.listWorkspaces",
    "me.registerDeviceToken",
    "me.unregisterDeviceToken",
    "me.changePassword",
];

/// `changePasswordFailed`.
const CHANGE_PASSWORD_FAILED: &[(&str, u16)] = &[("changePasswordFailed", 400)];

/// `z.string().min(min).max(max)` (no trim).
fn length(min: usize, max: Option<usize>) -> Str {
    Str {
        min: Some(min),
        max,
        ..Str::default()
    }
}

fn forbidden(message: &str) -> OrpcError {
    business_error(
        BusinessError::exception(message, "forbidden", 403),
        MUTATING,
    )
}

/// `parseOptionalWorkspaceHeader`.
fn optional_workspace(headers: &HeaderMap) -> Result<Option<i64>, OrpcError> {
    let Some(raw) = headers
        .get("x-workspace-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let valid = raw.len() <= 20 && raw.bytes().all(|b| b.is_ascii_digit());
    match raw.parse::<i64>() {
        Ok(id) if valid => Ok(Some(id)),
        _ => Err(OrpcError::new(
            "BAD_REQUEST",
            Some(400),
            Some("x-workspace-id must be a numeric workspace identifier"),
        )),
    }
}

/// Runs one of [`ROUTES`] for the mobile `user`.
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    user: &MobileUser,
    headers: &HeaderMap,
) -> Reply {
    let db = &state.db;
    match route_id {
        "me.get" => Ok(json(
            200,
            json!({
                "id": user.id.to_string(),
                "name": user.name.clone().unwrap_or_default(),
                "email": user.email,
                "image": user.image,
                "mustChangePassword": user.must_change_password,
            }),
        )),
        "me.listWorkspaces" => {
            let rows = chatbotx_business::workspace::list_for_member(db, user.id)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let mut meta = Vec::new();
            let list: Vec<Value> = rows
                .iter()
                .map(|w| crate::rpc::workspace::workspace_resource(w, &[], &mut meta))
                .collect();
            Ok(json(200, json!({ "workspaces": list })))
        }
        "me.registerDeviceToken" => {
            let mut input = Input::from_body(call.params, call.body);
            let raw = input.get("platform").cloned();
            let platform = input.enum_at(&key("platform"), raw.as_ref(), &["ios", "android"], true);
            let token = input.string("token", length(1, None));
            input.finish()?;
            let workspace_id = optional_workspace(headers)?;
            let (Some(platform), Some(token)) = (platform, token) else {
                return Err(OrpcError::new("BAD_REQUEST", Some(400), None));
            };
            let user_id = user.id;
            let registered = db
                .write(move |tx| {
                    if let Some(ws) = workspace_id {
                        let member: bool = tx.query_row(
                            r#"SELECT EXISTS (SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2)"#,
                            [ws, user_id],
                            |r| r.get(0),
                        )?;
                        if !member {
                            return Ok(false);
                        }
                    }
                    let now = now_ms();
                    // `deviceTokenService.upsert` (conflict on `token`).
                    tx.execute(
                        r#"INSERT INTO "UserDeviceToken" ("id", "createdAt", "updatedAt", "userId", "workspaceId", "platform", "token", "lastSeenAt")
                           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?2)
                           ON CONFLICT ("token") DO UPDATE SET "userId" = excluded."userId",
                             "workspaceId" = excluded."workspaceId", "platform" = excluded."platform",
                             "lastSeenAt" = excluded."lastSeenAt", "updatedAt" = excluded."updatedAt""#,
                        chatbotx_db::rusqlite::params![
                            chatbotx_db::repo::new_id(),
                            now,
                            user_id,
                            workspace_id,
                            platform,
                            token
                        ],
                    )?;
                    Ok(true)
                })
                .await
                .map_err(|e| business_error(BusinessError::from(e), MUTATING))?;
            if !registered {
                return Err(forbidden("Not a member of this workspace"));
            }
            Ok(json(200, json!({ "success": true })))
        }
        "me.unregisterDeviceToken" => {
            let mut input = Input::from_body(call.params, call.body);
            let token = input.string("token", length(1, None));
            input.finish()?;
            let token = token.unwrap_or_default();
            let user_id = user.id;
            db.write(move |tx| {
                tx.execute(
                    r#"DELETE FROM "UserDeviceToken" WHERE "token" = ?1 AND "userId" = ?2"#,
                    chatbotx_db::rusqlite::params![token, user_id],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| business_error(BusinessError::from(e), MUTATING))?;
            Ok(json(200, json!({ "success": true })))
        }
        "me.changePassword" => change_password(state, call, user).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

async fn change_password(state: &ApiState, call: Call<'_>, user: &MobileUser) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let current = input.string("currentPassword", length(1, Some(100)));
    let new = input.string("newPassword", length(8, Some(100)));
    let device = input.opt_string("deviceId", length(1, Some(128)));
    input.finish()?;
    let Some(sessions) = state.sessions.as_deref() else {
        return Err(crate::routes::not_ported_error());
    };
    let device_id = device
        .or_else(|| user.device_id.clone())
        .unwrap_or_else(chatbotx_core::utils::id::create_id);
    let result = crate::session::mobile_change_password(
        sessions,
        user.id,
        &current.unwrap_or_default(),
        &new.unwrap_or_default(),
        &device_id,
    )
    .await;
    let mut response = match result {
        Ok(body) => json(200, body),
        Err(error) if (400..500).contains(&error.status) => {
            return Err(business_error(
                BusinessError::exception(error.message, "changePasswordFailed", error.status),
                CHANGE_PASSWORD_FAILED,
            ));
        }
        Err(error) => {
            tracing::error!(message = %error.message, "Mobile password change failed");
            return Err(OrpcError::new("INTERNAL_SERVER_ERROR", Some(500), None));
        }
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    Ok(response)
}
