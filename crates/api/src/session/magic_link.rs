//! better-auth `magicLink()` and the email/password reset endpoints:
//!
//! - `POST /api/auth/sign-in/magic-link` -> `{ status: true }`, mails the link
//! - `GET  /api/auth/magic-link/verify`  -> session cookie + redirect to `callbackURL`
//!   (JSON `{ token, user, session }` without `callbackURL`)
//! - `POST /api/auth/request-password-reset` -> `{ status, message }`, mails the link
//! - `GET  /api/auth/reset-password/{token}` -> redirect to `callbackURL?token=` (or `?error=INVALID_TOKEN`)
//! - `POST /api/auth/reset-password` -> `{ status: true }`; revokes mobile refresh tokens
//!
//! Tokens live in `"Verification"` as better-auth stores them:
//! `magic-link:<token>` (value `{ type, email, name }`, 5 minutes) and
//! `reset-password:<token>` (value = user id, 1 hour).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::email_password::{
    MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH, ROOT_TENANT_ID, body_json, coded, create_session,
    find_user_by_email, find_user_by_id, internal, json_response, new_id, public_origin,
    query_value, redirect, session_cookie,
};
use super::{
    AuthApiError, AuthMail, AuthMailDeps, AuthMailRequest, AuthState, MailUser, NewUser,
    SmtpResolution, TenantBrand, expose_auth_token, generate_session_token, hash_password,
    is_trusted_url, send_magic_link, send_reset_password, user_create_before,
};

/// better-auth magic link lifetime (`expiresIn` 300 s).
const MAGIC_LINK_EXPIRES_IN_MS: i64 = 300 * 1000;
/// better-auth `resetPasswordTokenExpiresIn` default (1 hour).
const RESET_PASSWORD_EXPIRES_IN_MS: i64 = 3600 * 1000;
const RESET_MESSAGE: &str =
    "If this email exists in our system, check your email for the reset link";

impl AuthMailDeps for AuthState {
    async fn tenant_settings(&self, _domain: &str) -> TenantBrand {
        // One tenant (root): the brand is the root tenant's name.
        let name = self
            .db
            .read(|conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "name" FROM "Tenant" WHERE "id" = ?1"#,
                        [ROOT_TENANT_ID],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten())
            })
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "ChatbotX".to_string());
        TenantBrand {
            name,
            ..TenantBrand::default()
        }
    }

    async fn smtp_for_tenant(&self) -> SmtpResolution {
        SmtpResolution::Default
    }

    async fn find_tenant_user(&self, email: &str) -> Option<MailUser> {
        let email = email.to_string();
        self.db
            .read(move |conn| find_user_by_email(conn, &email))
            .await
            .ok()
            .flatten()
            .map(|user| MailUser {
                name: user.name,
                email: user.email,
            })
    }

    async fn send_magic_link(&self, to: &str, mail: AuthMail, _transport: Option<&str>) {
        let subject = mail.subject.replace("{{brandName}}", &mail.brand_name);
        let text = format!(
            "Hi {},\n\nOpen this link to sign in to {}:\n\n{}\n",
            mail.user_name, mail.brand_name, mail.url
        );
        if let Err(error) = self.mailer.send(to, &subject, &text).await {
            tracing::warn!(error = %error, "magic link mail not sent");
        }
    }

    async fn send_reset_password(&self, to: &str, mail: AuthMail, _transport: Option<&str>) {
        let text = format!(
            "Hi {},\n\nOpen this link to reset your password:\n\n{}\n",
            mail.user_name, mail.url
        );
        if let Err(error) = self.mailer.send(to, &mail.subject, &text).await {
            tracing::warn!(error = %error, "reset password mail not sent");
        }
    }
}

fn mail_request(state: &AuthState, headers: &HeaderMap) -> AuthMailRequest {
    AuthMailRequest {
        public_origin: public_origin(&state.settings, headers),
        domain: headers
            .get("x-domain")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
    }
}

/// `generateId(n)` / `generateRandomString(n, "a-z", "A-Z")`.
fn random_token(length: usize, digits: bool) -> String {
    let token = generate_session_token();
    let mut chars: Vec<char> = token
        .chars()
        .filter(|c| digits || c.is_ascii_alphabetic())
        .collect();
    while chars.len() < length {
        chars.extend(
            generate_session_token()
                .chars()
                .filter(|c| digits || c.is_ascii_alphabetic()),
        );
    }
    chars.truncate(length);
    chars.into_iter().collect()
}

fn encode_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// `new URL(path, ctx.context.baseURL)` with query parameters set.
fn absolute_url(state: &AuthState, target: &str, query: &[(&str, &str)]) -> String {
    let base = format!("{}/api/auth", state.settings.base_url);
    let Ok(mut url) = url::Url::parse(&base).and_then(|base| base.join(target)) else {
        return target.to_string();
    };
    for (name, value) in query {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| key != name)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair(name, value);
    }
    url.to_string()
}

// ---------------------------------------------------------------- magic link

pub(super) async fn sign_in_magic_link(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match magic_link_request(&state, &uri, &headers, &body).await {
        Ok(()) => json_response(StatusCode::OK, &json!({ "status": true })),
        Err(error) => error.into_response(),
    }
}

async fn magic_link_request(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), AuthApiError> {
    let body = body_json(body)?;
    state.check_origins(headers, uri, Some(&body), true).await?;
    let email = body["email"].as_str().unwrap_or_default().to_lowercase();
    if !email.contains('@') {
        return Err(coded("BAD_REQUEST", "INVALID_EMAIL", "Invalid email"));
    }
    let name = body["name"].as_str().map(str::to_string);
    let token = random_token(32, false);
    let value = json!({ "type": "magic-link", "email": email, "name": name }).to_string();
    let (identifier, expires_at, id) = (
        format!("magic-link:{token}"),
        (state.now_ms)() + MAGIC_LINK_EXPIRES_IN_MS,
        new_id(),
    );
    state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id.to_string(), identifier, value, expires_at],
            )?;
            Ok(())
        })
        .await
        .map_err(internal)?;
    let callback = body["callbackURL"]
        .as_str()
        .filter(|url| !url.is_empty())
        .unwrap_or("/");
    let mut url = format!(
        "{}/api/auth/magic-link/verify?token={token}&callbackURL={}",
        state.settings.base_url,
        encode_component(callback)
    );
    for name in ["newUserCallbackURL", "errorCallbackURL"] {
        if let Some(value) = body[name].as_str().filter(|value| !value.is_empty()) {
            url.push_str(&format!("&{name}={}", encode_component(value)));
        }
    }
    send_magic_link(state, &email, &url, Some(&mail_request(state, headers))).await
}

/// `consumeVerificationValue(identifier)`: the live value, deleted.
async fn consume_verification(
    state: &AuthState,
    identifier: String,
) -> Result<Option<String>, AuthApiError> {
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            let value: Option<(i64, String, i64)> = tx
                .query_row(
                    r#"SELECT rowid, "value", "expiresAt" FROM "Verification"
                       WHERE "identifier" = ?1 ORDER BY "createdAt" DESC LIMIT 1"#,
                    [&identifier],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((id, value, expires_at)) = value else {
                return Ok(None);
            };
            tx.execute(r#"DELETE FROM "Verification" WHERE rowid = ?1"#, [id])?;
            Ok((expires_at > now).then_some(value))
        })
        .await
        .map_err(internal)
}

pub(super) async fn magic_link_verify(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let callback_query = query_value(&uri, "callbackURL").filter(|url| !url.is_empty());
    let trusted = state.request_trusted_origins().await;
    if !is_trusted_url(callback_query.as_deref().unwrap_or("/"), &trusted, true) {
        let mut error = AuthApiError::new("FORBIDDEN", "Invalid callbackURL");
        error.code = Some("INVALID_CALLBACK_URL".to_string());
        return error.into_response();
    }
    let callback = absolute_url(&state, callback_query.as_deref().unwrap_or("/"), &[]);
    let error_target = query_value(&uri, "errorCallbackURL")
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| callback.clone());
    let redirect_error =
        |error: &str| redirect(&absolute_url(&state, &error_target, &[("error", error)]));
    let Some(token) = query_value(&uri, "token").filter(|token| !token.is_empty()) else {
        return redirect_error("INVALID_TOKEN");
    };
    let value = match consume_verification(&state, format!("magic-link:{token}")).await {
        Ok(Some(value)) => value,
        Ok(None) => return redirect_error("INVALID_TOKEN"),
        Err(error) => return error.into_response(),
    };
    let record: Value = serde_json::from_str(&value).unwrap_or(Value::Null);
    let Some(email) = record["email"].as_str().map(str::to_string) else {
        return redirect_error("INVALID_TOKEN");
    };
    let name = record["name"].as_str().unwrap_or_default().to_string();
    let lookup = email.clone();
    let existing = match state
        .db
        .read(move |conn| find_user_by_email(conn, &lookup))
        .await
    {
        Ok(user) => user,
        Err(error) => return internal(error).into_response(),
    };
    let (user_id, is_new_user) = match existing {
        Some(user) if user.email_verified => (user.id, false),
        Some(user) => match promote_unproven_user(&state, user.id).await {
            Ok(()) => (user.id, false),
            Err(error) => return error.into_response(),
        },
        None => {
            let new_user = NewUser {
                email: Some(email.clone()),
                is_anonymous: Some(false),
            };
            if let Err(error) = user_create_before(&*state, &new_user, Some(&headers)).await {
                let code = error.code.clone().unwrap_or_else(|| error.message.clone());
                return redirect_error(&code);
            }
            let id = new_id();
            let created = state
                .db
                .write(move |tx| {
                    tx.execute(
                        r#"INSERT INTO "User" ("id", "name", "email", "emailVerified", "tenantId")
                           VALUES (?1, ?2, ?3, 1, ?4)"#,
                        params![id, name, email, ROOT_TENANT_ID],
                    )?;
                    Ok(())
                })
                .await;
            if let Err(error) = created {
                return internal(error).into_response();
            }
            (id, true)
        }
    };
    let session_token = match create_session(&state, &headers, user_id).await {
        Ok(token) => token,
        Err(error) => return error.into_response(),
    };
    let mut response = if callback_query.is_none() {
        let user = state
            .db
            .read(move |conn| find_user_by_id(conn, user_id))
            .await;
        match user {
            Ok(Some(user)) => json_response(
                StatusCode::OK,
                &json!({ "token": session_token, "user": user.to_json() }),
            ),
            Ok(None) => return redirect_error("user_not_found"),
            Err(error) => return internal(error).into_response(),
        }
    } else if is_new_user {
        let target = query_value(&uri, "newUserCallbackURL")
            .filter(|url| !url.is_empty())
            .map_or_else(|| callback.clone(), |url| absolute_url(&state, &url, &[]));
        redirect(&target)
    } else {
        redirect(&callback)
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie(&session_token, &state.settings.secret),
    );
    expose_auth_token(response.headers_mut());
    response
}

/// `revokeUnprovenAccountAccess`: an unverified user proven by the mailbox
/// loses its other login methods and sessions and becomes verified.
async fn promote_unproven_user(state: &AuthState, user_id: i64) -> Result<(), AuthApiError> {
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            tx.execute(r#"DELETE FROM "Account" WHERE "userId" = ?1"#, [user_id])?;
            tx.execute(r#"DELETE FROM "Session" WHERE "userId" = ?1"#, [user_id])?;
            tx.execute(
                r#"UPDATE "User" SET "emailVerified" = 1, "updatedAt" = ?2 WHERE "id" = ?1"#,
                params![user_id, now],
            )?;
            Ok(())
        })
        .await
        .map_err(internal)
}

// ---------------------------------------------------------------- password reset

pub(super) async fn request_password_reset(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match password_reset_request(&state, &uri, &headers, &body).await {
        Ok(()) => json_response(
            StatusCode::OK,
            &json!({ "status": true, "message": RESET_MESSAGE }),
        ),
        Err(error) => error.into_response(),
    }
}

async fn password_reset_request(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), AuthApiError> {
    let body = body_json(body)?;
    state
        .check_origins(headers, uri, Some(&body), false)
        .await?;
    let email = body["email"].as_str().unwrap_or_default().to_lowercase();
    let lookup = email.clone();
    let user = state
        .db
        .read(move |conn| find_user_by_email(conn, &lookup))
        .await
        .map_err(internal)?;
    let token = random_token(24, true);
    let Some(user) = user else {
        tracing::warn!("Reset Password: User not found");
        return Ok(());
    };
    let (id, identifier, value, expires_at) = (
        new_id(),
        format!("reset-password:{token}"),
        user.id.to_string(),
        (state.now_ms)() + RESET_PASSWORD_EXPIRES_IN_MS,
    );
    state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id.to_string(), identifier, value, expires_at],
            )?;
            Ok(())
        })
        .await
        .map_err(internal)?;
    let callback = body["redirectTo"]
        .as_str()
        .map(encode_component)
        .unwrap_or_default();
    let url = format!(
        "{}/api/auth/reset-password/{token}?callbackURL={callback}",
        state.settings.base_url
    );
    let mail_user = MailUser {
        name: user.name.clone(),
        email: user.email.clone(),
    };
    send_reset_password(state, &mail_user, &url, Some(&mail_request(state, headers))).await
}

pub(super) async fn reset_password_callback(
    State(state): State<Arc<AuthState>>,
    Path(token): Path<String>,
    uri: Uri,
) -> Response {
    let callback = query_value(&uri, "callbackURL").filter(|url| !url.is_empty());
    let Some(callback) = callback else {
        return redirect(&absolute_url(
            &state,
            "/api/auth/error",
            &[("error", "INVALID_TOKEN")],
        ));
    };
    let identifier = format!("reset-password:{token}");
    let now = (state.now_ms)();
    let live = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT 1 FROM "Verification" WHERE "identifier" = ?1 AND "expiresAt" >= ?2"#,
                    params![identifier, now],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })
        .await;
    match live {
        Ok(true) => redirect(&absolute_url(&state, &callback, &[("token", &token)])),
        Ok(false) => redirect(&absolute_url(
            &state,
            &callback,
            &[("error", "INVALID_TOKEN")],
        )),
        Err(error) => internal(error).into_response(),
    }
}

fn invalid_token() -> AuthApiError {
    coded("BAD_REQUEST", "INVALID_TOKEN", "Invalid token")
}

pub(super) async fn reset_password(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match reset_password_inner(&state, &uri, &headers, &body).await {
        Ok(()) => json_response(StatusCode::OK, &json!({ "status": true })),
        Err(error) => error.into_response(),
    }
}

async fn reset_password_inner(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), AuthApiError> {
    let body = body_json(body)?;
    state
        .check_origins(headers, uri, Some(&body), false)
        .await?;
    let token = body["token"]
        .as_str()
        .map(str::to_string)
        .or_else(|| query_value(uri, "token"))
        .filter(|token| !token.is_empty())
        .ok_or_else(invalid_token)?;
    let password = body["newPassword"].as_str().unwrap_or_default().to_string();
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
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(internal)?;
    let identifier = format!("reset-password:{token}");
    let now = (state.now_ms)();
    let account_id = new_id();
    let outcome = state
        .db
        .write(move |tx| {
            let verification: Option<(i64, String)> = tx
                .query_row(
                    r#"SELECT rowid, "value" FROM "Verification"
                       WHERE "identifier" = ?1 AND "expiresAt" >= ?2 LIMIT 1"#,
                    params![identifier, now],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((verification_id, user_id)) = verification else {
                return Ok(Err("INVALID_TOKEN"));
            };
            let Ok(user_id) = user_id.parse::<i64>() else {
                return Ok(Err("USER_NOT_FOUND"));
            };
            if find_user_by_id(tx, user_id)?.is_none() {
                return Ok(Err("USER_NOT_FOUND"));
            }
            let updated = tx.execute(
                r#"UPDATE "Account" SET "password" = ?1, "updatedAt" = ?3
                   WHERE "userId" = ?2 AND "providerId" = 'credential'"#,
                params![password_hash, user_id, now],
            )?;
            if updated == 0 {
                tx.execute(
                    r#"INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId", "tenantId")
                       VALUES (?1, ?2, 'credential', ?3, ?4, ?5)"#,
                    params![account_id, user_id.to_string(), password_hash, user_id, ROOT_TENANT_ID],
                )?;
            }
            tx.execute(r#"DELETE FROM "Verification" WHERE rowid = ?1"#, [verification_id])?;
            // `onPasswordReset`: mobileRefreshTokenService.revokeAllForUser.
            chatbotx_db::repo::mobile_refresh_token::revoke_all_for_user(tx, user_id, now)
                .map_err(|error| chatbotx_db::DbError::other(error.to_string()))?;
            Ok(Ok(()))
        })
        .await
        .map_err(internal)?;
    match outcome {
        Ok(()) => Ok(()),
        Err("USER_NOT_FOUND") => Err(coded("BAD_REQUEST", "USER_NOT_FOUND", "User not found")),
        Err(_) => Err(invalid_token()),
    }
}
