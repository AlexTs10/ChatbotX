//! better-auth account endpoints the builder UI calls, and the
//! `forceChangePasswordAction` server action:
//!
//! - `POST /api/auth/update-user`     `{ name?, image? }` -> `{ status: true }`
//! - `POST /api/auth/change-password` `{ currentPassword, newPassword, revokeOtherSessions? }`
//!   -> `{ token, user }` (a new session cookie when `revokeOtherSessions`);
//!   the `mobileAuth` after hook revokes the user's mobile refresh tokens
//! - `force_change_password`: `features/auth/actions/force-change-password.ts`
//!   (served as `/rpc/authActions/forceChangePassword`, see `rpc`).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::email_password::{
    MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH, SESSION_COOKIE, UserRow, body_json, coded,
    create_session, find_user_by_id, internal, json_response, session_cookie,
};
use super::{
    AuthApiError, AuthState, apply_bearer, expose_auth_token, hash_password, request_cookie,
    verify_password, verify_signed_cookie,
};

/// The live session of a request (`getSessionFromCtx`).
pub(super) struct CurrentSession {
    pub(super) user: UserRow,
}

/// The session of the request's cookie (after the bearer rewrite).
pub(super) async fn current_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<Option<CurrentSession>, AuthApiError> {
    let Some(token) = request_cookie(headers, SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret))
    else {
        return Ok(None);
    };
    let now = (state.now_ms)();
    state
        .db
        .read(move |conn| {
            let user_id: Option<i64> = conn
                .query_row(
                    r#"SELECT "userId" FROM "Session" WHERE "token" = ?1 AND "expiresAt" > ?2"#,
                    params![token, now],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(user_id) = user_id else {
                return Ok(None);
            };
            Ok(find_user_by_id(conn, user_id)?.map(|user| CurrentSession { user }))
        })
        .await
        .map_err(internal)
}

/// `sessionMiddleware`: 401 `UNAUTHORIZED` without a live session.
async fn require_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<CurrentSession, AuthApiError> {
    current_session(state, headers)
        .await?
        .ok_or_else(|| coded("UNAUTHORIZED", "UNAUTHORIZED", "Unauthorized"))
}

pub(super) async fn update_user(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let headers = apply_bearer(&headers, &state.settings.secret);
    match update_user_inner(&state, &uri, &headers, &body).await {
        Ok(()) => json_response(StatusCode::OK, &json!({ "status": true })),
        Err(error) => error.into_response(),
    }
}

async fn update_user_inner(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), AuthApiError> {
    let body = body_json(body)?;
    state
        .check_origins(headers, uri, Some(&body), false)
        .await?;
    let session = require_session(state, headers).await?;
    if body.get("email").is_some_and(is_truthy) {
        return Err(coded(
            "BAD_REQUEST",
            "EMAIL_CAN_NOT_BE_UPDATED",
            "Email can not be updated",
        ));
    }
    // `parseUserInput(..., "update")`: the declared additional fields
    // (`tenantId`, `mustChangePassword`) are `input: false`.
    for field in ["tenantId", "mustChangePassword", "isAnonymous"] {
        if body.get(field).is_some() {
            return Err(AuthApiError::new(
                "BAD_REQUEST",
                &format!("{field} is not allowed to be set"),
            ));
        }
    }
    let name = body.get("name").cloned();
    let image = body.get("image").cloned();
    if name.is_none() && image.is_none() {
        return Err(AuthApiError::new("BAD_REQUEST", "No fields to update"));
    }
    let as_text = |value: &Option<Value>| -> Result<Option<Option<String>>, AuthApiError> {
        match value {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(text)) => Ok(Some(Some(text.clone()))),
            Some(_) => Err(AuthApiError::new("BAD_REQUEST", "Invalid body parameters")),
        }
    };
    let (name, image) = (as_text(&name)?, as_text(&image)?);
    let user_id = session.user.id;
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            if let Some(name) = name {
                tx.execute(
                    r#"UPDATE "User" SET "name" = ?1, "updatedAt" = ?3 WHERE "id" = ?2"#,
                    params![name, user_id, now],
                )?;
            }
            if let Some(image) = image {
                tx.execute(
                    r#"UPDATE "User" SET "image" = ?1, "updatedAt" = ?3 WHERE "id" = ?2"#,
                    params![image, user_id, now],
                )?;
            }
            Ok(())
        })
        .await
        .map_err(internal)
}

/// JS truthiness of a JSON value (`if (body.email)`).
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

/// The input of `/change-password`.
pub struct ChangePasswordInput {
    pub current_password: String,
    pub new_password: String,
    pub revoke_other_sessions: bool,
}

/// The result of a password change: the new session token (when the other
/// sessions were revoked) and the user before the change.
pub(super) struct ChangedPassword {
    token: Option<String>,
    user: UserRow,
}

pub(super) async fn change_password_endpoint(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let headers = apply_bearer(&headers, &state.settings.secret);
    let result = async {
        let body = body_json(&body)?;
        state
            .check_origins(&headers, &uri, Some(&body), false)
            .await?;
        let input = change_password_body(&body)?;
        change_password(&state, &headers, input).await
    }
    .await;
    match result {
        Ok(changed) => {
            let mut response = json_response(
                StatusCode::OK,
                &json!({ "token": changed.token, "user": changed.user.to_json() }),
            );
            if let Some(token) = &changed.token {
                response.headers_mut().append(
                    header::SET_COOKIE,
                    session_cookie(token, &state.settings.secret),
                );
                expose_auth_token(response.headers_mut());
            }
            response
        }
        Err(error) => error.into_response(),
    }
}

fn change_password_body(body: &Value) -> Result<ChangePasswordInput, AuthApiError> {
    let invalid = || AuthApiError::new("BAD_REQUEST", "Invalid body parameters");
    let text = |name: &str| body.get(name).and_then(Value::as_str).map(str::to_string);
    let revoke_other_sessions = match body.get("revokeOtherSessions") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(invalid()),
    };
    Ok(ChangePasswordInput {
        current_password: text("currentPassword").ok_or_else(invalid)?,
        new_password: text("newPassword").ok_or_else(invalid)?,
        revoke_other_sessions,
    })
}

fn check_password_length(password: &str) -> Result<(), AuthApiError> {
    let length = password.encode_utf16().count();
    if length < MIN_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_SHORT",
            "Password too short",
        ));
    }
    if length > MAX_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_LONG",
            "Password too long",
        ));
    }
    Ok(())
}

/// better-auth `changePassword` + the `mobileAuth` after hook.
pub(super) async fn change_password(
    state: &AuthState,
    headers: &HeaderMap,
    input: ChangePasswordInput,
) -> Result<ChangedPassword, AuthApiError> {
    let session = require_session(state, headers).await?;
    check_password_length(&input.new_password)?;
    if input.current_password.encode_utf16().count() > MAX_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_LONG",
            "Password too long",
        ));
    }
    let user_id = session.user.id;
    let stored: Option<(i64, Option<String>)> = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "id", "password" FROM "Account"
                       WHERE "userId" = ?1 AND "providerId" = 'credential' LIMIT 1"#,
                    [user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?)
        })
        .await
        .map_err(internal)?;
    let Some((account_id, Some(stored_hash))) = stored else {
        return Err(coded(
            "BAD_REQUEST",
            "CREDENTIAL_ACCOUNT_NOT_FOUND",
            "Credential account not found",
        ));
    };
    let ChangePasswordInput {
        current_password,
        new_password,
        revoke_other_sessions,
    } = input;
    let (new_hash, valid) = tokio::task::spawn_blocking(move || {
        (
            hash_password(&new_password),
            verify_password(&stored_hash, &current_password),
        )
    })
    .await
    .map_err(internal)?;
    if !valid {
        return Err(coded("BAD_REQUEST", "INVALID_PASSWORD", "Invalid password"));
    }
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            if revoke_other_sessions {
                tx.execute(r#"DELETE FROM "Session" WHERE "userId" = ?1"#, [user_id])?;
            }
            tx.execute(
                r#"UPDATE "Account" SET "password" = ?1, "updatedAt" = ?3 WHERE "id" = ?2"#,
                params![new_hash, account_id, now],
            )?;
            // `mobileAuth` after hook on `/change-password`.
            chatbotx_db::repo::mobile_refresh_token::revoke_all_for_user(tx, user_id, now)
                .map_err(|error| chatbotx_db::DbError::other(error.to_string()))?;
            Ok(())
        })
        .await
        .map_err(internal)?;
    let token = if revoke_other_sessions {
        Some(create_session(state, headers, user_id).await?)
    } else {
        None
    };
    Ok(ChangedPassword {
        token,
        user: session.user,
    })
}

/// The error of [`force_change_password`]: `ChatbotXException(message, code, status)`
/// or the input validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceChangePasswordError {
    pub code: String,
    pub status: u16,
    pub message: String,
}

/// `forceChangePasswordAction`: validates `changePasswordRequest`, changes the
/// password with `revokeOtherSessions` and clears `mustChangePassword`.
/// `Ok(Some(cookie))` = the rotated session cookie.
pub async fn force_change_password(
    state: &AuthState,
    headers: &HeaderMap,
    input: &Value,
) -> Result<Option<HeaderValue>, ForceChangePasswordError> {
    let headers = apply_bearer(headers, &state.settings.secret);
    let error = |code: &str, status: u16, message: &str| ForceChangePasswordError {
        code: code.to_string(),
        status,
        message: message.to_string(),
    };
    let field = |name: &str| {
        input
            .get(name)
            .and_then(Value::as_str)
            .filter(|text| (8..=100).contains(&text.chars().count()))
            .map(str::to_string)
    };
    let (Some(current), Some(new), Some(confirmation)) = (
        field("currentPassword"),
        field("newPassword"),
        field("passwordConfirmation"),
    ) else {
        return Err(error("BAD_REQUEST", 400, "Input validation failed"));
    };
    if new != confirmation || new == current {
        return Err(error("BAD_REQUEST", 400, "Input validation failed"));
    }
    let Some(session) = current_session(state, &headers)
        .await
        .map_err(|e| error("INTERNAL_SERVER_ERROR", 500, &e.message))?
    else {
        return Err(error("unauthorized", 401, "Not authenticated"));
    };
    let changed = change_password(
        state,
        &headers,
        ChangePasswordInput {
            current_password: current,
            new_password: new,
            revoke_other_sessions: true,
        },
    )
    .await
    .map_err(|e| match e.status {
        500 => error("INTERNAL_SERVER_ERROR", 500, &e.message),
        _ => error("changePasswordFailed", 400, &e.message),
    })?;
    let user_id = session.user.id;
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            // `userService.clearMustChangePassword`.
            tx.execute(
                r#"UPDATE "User" SET "mustChangePassword" = 0, "updatedAt" = ?2 WHERE "id" = ?1"#,
                params![user_id, now],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| error("INTERNAL_SERVER_ERROR", 500, &e.to_string()))?;
    Ok(changed
        .token
        .map(|token| session_cookie(&token, &state.settings.secret)))
}
