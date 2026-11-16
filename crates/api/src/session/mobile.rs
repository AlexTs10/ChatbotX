//! `mobileAuth()` (`packages/auth/src/mobile-auth.ts`) and the refresh-token
//! service (`business/src/mobile-auth/service.ts`):
//!
//! - `POST /api/auth/mobile/token`  grants `password`, `exchange_code`, `refresh_token`
//! - `GET  /api/auth/mobile/callback` end of the browser social login: a one-time
//!   code, 302 to `<scheme>://...?code=&state=`, the browser session is dropped
//! - `POST /api/auth/mobile/revoke` revokes a refresh-token family
//! - `GET  /api/auth/jwks`
//! - [`mobile_change_password`]: the server-only `mobileChangePassword`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_db::repo::mobile_refresh_token as tokens;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::email_password::{
    MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH, SESSION_COOKIE, body_json, cleared_session_cookie,
    coded, find_user_by_email, find_user_by_id, internal, json_response, new_id, query_value,
    random_bytes, redirect,
};
use super::{
    AuthApiError, AuthState, apply_bearer, hash_password, jwks, request_cookie, sign_jwt,
    signing_key, verify_password, verify_signed_cookie,
};
use crate::auth::hash_token;

pub const MOBILE_ACCESS_TOKEN_TTL_SECONDS: i64 = 10 * 60;
pub const MOBILE_EXCHANGE_CODE_TTL_MS: i64 = 60 * 1000;
pub const MOBILE_REFRESH_TOKEN_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const SPENT_TOKEN_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
const ROTATION_GRACE_MS: i64 = 30 * 1000;
const MOBILE_CODE_IDENTIFIER_PREFIX: &str = "mobile-auth-code:";
const DEFAULT_MOBILE_APP_SCHEMES: [&str; 2] = ["chatconnectxapp", "chatbotxmobileapp"];

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `getBuilderOrigin()` (`NEXT_PUBLIC_BUILDER_URL`, else the auth base URL).
pub fn builder_origin(state: &AuthState) -> String {
    let builder = &state.settings.trusted_origins.builder_url;
    let url = if builder.is_empty() {
        &state.settings.base_url
    } else {
        builder
    };
    url::Url::parse(url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| url.trim_end_matches('/').to_string())
}

/// `getMobileApiAudience()`: `<builder origin>/api`.
pub fn mobile_api_audience(state: &AuthState) -> String {
    format!("{}/api", builder_origin(state))
}

/// `getMobileAppSchemes()`: the defaults plus `MOBILE_APP_SCHEMES`.
fn mobile_app_schemes() -> Vec<String> {
    let mut schemes: Vec<String> = DEFAULT_MOBILE_APP_SCHEMES.map(String::from).to_vec();
    for scheme in std::env::var("MOBILE_APP_SCHEMES")
        .unwrap_or_default()
        .split(',')
    {
        let scheme = scheme
            .trim()
            .to_lowercase()
            .trim_end_matches(':')
            .to_string();
        if !scheme.is_empty() && !schemes.contains(&scheme) {
            schemes.push(scheme);
        }
    }
    schemes
}

fn valid_device_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn unauthorized(code: &str, message: &str) -> AuthApiError {
    coded("UNAUTHORIZED", code, message)
}

fn invalid_body() -> AuthApiError {
    coded("BAD_REQUEST", "VALIDATION_ERROR", "Invalid body parameters")
}

// ---------------------------------------------------------------- refresh tokens

/// An issued refresh token.
pub struct IssuedRefreshToken {
    pub token: String,
    pub expires_at: i64,
}

/// `mobileRefreshTokenService.issue` inside a write.
pub fn issue_refresh_token(
    conn: &Connection,
    user_id: i64,
    device_id: &str,
    family_id: Option<&str>,
    now: i64,
) -> Result<IssuedRefreshToken, chatbotx_db::DbError> {
    let other = |error: chatbotx_db::repo::Error| chatbotx_db::DbError::other(error.to_string());
    if family_id.is_none() {
        tokens::delete_expired_for_user(conn, user_id, now).map_err(other)?;
    }
    let token = format!("cbx_mrt_{}", b64(&random_bytes(32)));
    let expires_at = now + MOBILE_REFRESH_TOKEN_TTL_MS;
    tokens::insert(
        conn,
        &tokens::NewMobileRefreshToken {
            user_id,
            device_id: device_id.to_string(),
            family_id: family_id.map_or_else(chatbotx_core::utils::id::create_id, str::to_string),
            token_hash: hash_token(&token),
            expires_at,
            rotated_at: None,
            revoked: None,
        },
    )
    .map_err(other)?;
    Ok(IssuedRefreshToken { token, expires_at })
}

enum Rotation {
    Rotated {
        user_id: i64,
        device_id: String,
        refresh: IssuedRefreshToken,
    },
    Invalid,
    Reused,
}

/// `mobileRefreshTokenService.rotate`.
async fn rotate(state: &AuthState, token: &str) -> Result<Rotation, AuthApiError> {
    let token_hash = hash_token(token);
    let now = (state.now_ms)();
    state
        .db
        .write(move |tx| {
            let other =
                |error: chatbotx_db::repo::Error| chatbotx_db::DbError::other(error.to_string());
            let Some(existing) = tokens::lock_family_by_hash(tx, &token_hash).map_err(other)?
            else {
                return Ok(Rotation::Invalid);
            };
            if let Some(spent) = tokens::spend_active(tx, &token_hash, now).map_err(other)? {
                let refresh = issue_refresh_token(
                    tx,
                    spent.user_id,
                    &spent.device_id,
                    Some(&spent.family_id),
                    now,
                )?;
                tokens::delete_spent_in_family(
                    tx,
                    &spent.family_id,
                    now - SPENT_TOKEN_RETENTION_MS,
                )
                .map_err(other)?;
                return Ok(Rotation::Rotated {
                    user_id: spent.user_id,
                    device_id: spent.device_id,
                    refresh,
                });
            }
            if existing
                .rotated_at
                .is_some_and(|rotated| now - rotated <= ROTATION_GRACE_MS)
            {
                return Ok(Rotation::Invalid);
            }
            if existing.rotated_at.is_some() || existing.revoked.is_some() {
                tokens::revoke_family_by_hash(tx, &token_hash, now).map_err(other)?;
                tracing::warn!(
                    user_id = existing.user_id,
                    device_id = %existing.device_id,
                    "Mobile refresh token reuse detected; token family revoked"
                );
                return Ok(Rotation::Reused);
            }
            Ok(Rotation::Invalid)
        })
        .await
        .map_err(internal)
}

// ---------------------------------------------------------------- tokens

/// `buildTokenResponse` (+ `mintAccessToken`).
async fn token_response(
    state: &AuthState,
    user_id: i64,
    must_change_password: bool,
    device_id: &str,
    refresh: &IssuedRefreshToken,
) -> Result<Value, AuthApiError> {
    let now = (state.now_ms)();
    let key = signing_key(&state.db, &state.settings.secret, now)
        .await
        .map_err(internal)?;
    let issued_at = now / 1000;
    let expires_at = issued_at + MOBILE_ACCESS_TOKEN_TTL_SECONDS;
    let access_token = sign_jwt(
        &key,
        &json!({
            "sub": user_id.to_string(),
            "iat": issued_at,
            "exp": expires_at,
            "jti": chatbotx_core::utils::id::create_id(),
            "deviceId": device_id,
            "mustChangePassword": must_change_password,
            "iss": builder_origin(state),
            "aud": mobile_api_audience(state),
        }),
    );
    Ok(json!({
        "tokenType": "Bearer",
        "accessToken": access_token,
        "expiresIn": MOBILE_ACCESS_TOKEN_TTL_SECONDS,
        "expiresAt": expires_at * 1000,
        "refreshToken": refresh.token,
        "refreshTokenExpiresAt": refresh.expires_at,
        "deviceId": device_id,
        "mustChangePassword": must_change_password,
    }))
}

fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

pub(super) async fn mobile_token(State(state): State<Arc<AuthState>>, body: Bytes) -> Response {
    let result = mobile_token_inner(&state, &body).await;
    no_store(match result {
        Ok(body) => json_response(StatusCode::OK, &body),
        Err(error) => error.into_response(),
    })
}

/// A new refresh-token family for `user_id`, then the token response.
async fn issue_tokens(
    state: &AuthState,
    user_id: i64,
    must_change_password: bool,
    device_id: String,
) -> Result<Value, AuthApiError> {
    let now = (state.now_ms)();
    let device = device_id.clone();
    let refresh = state
        .db
        .write(move |tx| issue_refresh_token(tx, user_id, &device, None, now))
        .await
        .map_err(internal)?;
    token_response(state, user_id, must_change_password, &device_id, &refresh).await
}

async fn mobile_token_inner(state: &AuthState, body: &Bytes) -> Result<Value, AuthApiError> {
    let body = body_json(body)?;
    let text = |name: &str| {
        body[name]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let device_id = match body.get("deviceId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if valid_device_id(id) => Some(id.trim().to_string()),
        Some(_) => return Err(invalid_body()),
    };
    match body["grantType"].as_str() {
        Some("refresh_token") => {
            let token = text("refreshToken").ok_or_else(invalid_body)?;
            let (user_id, device_id, refresh) = match rotate(state, &token).await? {
                Rotation::Rotated {
                    user_id,
                    device_id,
                    refresh,
                } => (user_id, device_id, refresh),
                Rotation::Reused => {
                    return Err(unauthorized(
                        "REFRESH_TOKEN_REUSED",
                        "Refresh token was already used",
                    ));
                }
                Rotation::Invalid => {
                    return Err(unauthorized(
                        "INVALID_REFRESH_TOKEN",
                        "Invalid or expired refresh token",
                    ));
                }
            };
            let user = state
                .db
                .read(move |conn| find_user_by_id(conn, user_id))
                .await
                .map_err(internal)?;
            let Some(user) = user else {
                let now = (state.now_ms)();
                let _ = state
                    .db
                    .write(move |tx| {
                        tokens::revoke_all_for_user(tx, user_id, now)
                            .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
                    })
                    .await;
                return Err(unauthorized(
                    "INVALID_REFRESH_TOKEN",
                    "Invalid or expired refresh token",
                ));
            };
            token_response(
                state,
                user.id,
                user.must_change_password,
                &device_id,
                &refresh,
            )
            .await
        }
        Some("exchange_code") => {
            let code = text("code").ok_or_else(invalid_body)?;
            let verifier = text("codeVerifier")
                .filter(|v| {
                    (43..=128).contains(&v.len())
                        && v.chars().all(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-')
                        })
                })
                .ok_or_else(invalid_body)?;
            let identifier = format!(
                "{MOBILE_CODE_IDENTIFIER_PREFIX}{}",
                b64(&Sha256::digest(code.as_bytes()))
            );
            let now = (state.now_ms)();
            // `consumeVerificationValue`: single use, also under races (one writer).
            let record: Option<String> = state
                .db
                .write(move |tx| {
                    let row: Option<(i64, String)> = tx
                        .query_row(
                            r#"SELECT rowid, "value" FROM "Verification"
                               WHERE "identifier" = ?1 AND "expiresAt" > ?2 LIMIT 1"#,
                            params![identifier, now],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()?;
                    tx.execute(
                        r#"DELETE FROM "Verification" WHERE "identifier" = ?1"#,
                        [&identifier],
                    )?;
                    Ok(row.map(|(_, value)| value))
                })
                .await
                .map_err(internal)?;
            let invalid_code = || unauthorized("INVALID_EXCHANGE_CODE", "Invalid or expired code");
            let record: Value = record
                .and_then(|value| serde_json::from_str(&value).ok())
                .ok_or_else(invalid_code)?;
            let (Some(user_id), Some(challenge)) =
                (record["userId"].as_str(), record["codeChallenge"].as_str())
            else {
                return Err(invalid_code());
            };
            if b64(&Sha256::digest(verifier.as_bytes())) != challenge {
                return Err(unauthorized(
                    "INVALID_CODE_VERIFIER",
                    "Code verifier does not match",
                ));
            }
            let user_id: i64 = user_id.parse().map_err(|_| invalid_code())?;
            let user = state
                .db
                .read(move |conn| find_user_by_id(conn, user_id))
                .await
                .map_err(internal)?
                .ok_or_else(invalid_code)?;
            let device_id = device_id
                .or_else(|| record["deviceId"].as_str().map(str::to_string))
                .unwrap_or_else(chatbotx_core::utils::id::create_id);
            issue_tokens(state, user.id, user.must_change_password, device_id).await
        }
        Some("password") => {
            let (Some(email), Some(password)) = (text("email"), text("password")) else {
                return Err(invalid_body());
            };
            let user = authenticate_with_password(state, &email, &password).await?;
            let device_id = device_id.unwrap_or_else(chatbotx_core::utils::id::create_id);
            issue_tokens(state, user.0, user.1, device_id).await
        }
        _ => Err(invalid_body()),
    }
}

/// `authenticateWithPassword`: `(userId, mustChangePassword)`.
async fn authenticate_with_password(
    state: &AuthState,
    email: &str,
    password: &str,
) -> Result<(i64, bool), AuthApiError> {
    let email = email.trim().to_lowercase();
    let valid_email = email
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if !valid_email {
        return Err(coded("BAD_REQUEST", "INVALID_EMAIL", "Invalid email"));
    }
    if password.encode_utf16().count() > MAX_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_LONG",
            "Password too long",
        ));
    }
    let found = state
        .db
        .read(move |conn| {
            let Some(user) = find_user_by_email(conn, &email)? else {
                return Ok(None);
            };
            let hash: Option<String> = conn
                .query_row(
                    r#"SELECT "password" FROM "Account"
                       WHERE "userId" = ?1 AND "providerId" = 'credential' AND "accountId" = ?2 LIMIT 1"#,
                    params![user.id, user.id.to_string()],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            Ok(Some((user, hash)))
        })
        .await
        .map_err(internal)?;
    let password = password.to_string();
    let invalid = || unauthorized("INVALID_EMAIL_OR_PASSWORD", "Invalid email or password");
    let Some((user, Some(hash))) = found else {
        let _ = tokio::task::spawn_blocking(move || hash_password(&password)).await;
        return Err(invalid());
    };
    let matches = tokio::task::spawn_blocking(move || verify_password(&hash, &password))
        .await
        .map_err(internal)?;
    if !matches {
        return Err(invalid());
    }
    if !user.email_verified {
        return Err(coded(
            "FORBIDDEN",
            "EMAIL_NOT_VERIFIED",
            "Email not verified",
        ));
    }
    Ok((user.id, user.must_change_password))
}

// ---------------------------------------------------------------- callback / revoke / jwks

pub(super) async fn mobile_callback(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let bad = |code: &str, message: &str| coded("BAD_REQUEST", code, message).into_response();
    let challenge = query_value(&uri, "code_challenge").filter(|c| {
        c.len() == 43
            && c.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    });
    let device_id = query_value(&uri, "device_id");
    let oauth_state = query_value(&uri, "state");
    if challenge.is_none()
        || device_id.as_deref().is_some_and(|id| !valid_device_id(id))
        || oauth_state.as_deref().is_some_and(|s| s.len() > 512)
    {
        return bad("VALIDATION_ERROR", "Invalid query parameters");
    }
    let target = query_value(&uri, "redirect_uri")
        .and_then(|value| url::Url::parse(&value).ok())
        .filter(|url| {
            let scheme = url.scheme().to_lowercase();
            scheme != "http" && scheme != "https" && mobile_app_schemes().contains(&scheme)
        });
    let Some(target) = target else {
        return bad(
            "INVALID_REDIRECT_URI",
            "redirect_uri is not an allowed app scheme",
        );
    };
    let to_app = |pairs: &[(&str, Option<&str>)]| {
        let mut url = target.clone();
        {
            let mut query = url.query_pairs_mut();
            for (key, value) in pairs {
                if let Some(value) = value {
                    query.append_pair(key, value);
                }
            }
        }
        url.to_string()
    };
    let headers = apply_bearer(&headers, &state.settings.secret);
    let session_token = request_cookie(&headers, SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret));
    let now = (state.now_ms)();
    let user_id: Option<i64> = match session_token.clone() {
        None => None,
        Some(token) => state
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "userId" FROM "Session" WHERE "token" = ?1 AND "expiresAt" > ?2"#,
                        params![token, now],
                        |row| row.get(0),
                    )
                    .optional()?)
            })
            .await
            .unwrap_or(None),
    };
    let (Some(user_id), Some(session_token)) = (user_id, session_token) else {
        return redirect(&to_app(&[
            ("error", Some("access_denied")),
            ("state", oauth_state.as_deref()),
        ]));
    };
    let code = b64(&random_bytes(32));
    let identifier = format!(
        "{MOBILE_CODE_IDENTIFIER_PREFIX}{}",
        b64(&Sha256::digest(code.as_bytes()))
    );
    let value = json!({
        "userId": user_id.to_string(),
        "deviceId": device_id,
        "codeChallenge": challenge,
    })
    .to_string();
    let id = new_id();
    let stored = state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, identifier, value, now + MOBILE_EXCHANGE_CODE_TTL_MS],
            )?;
            // The social callback minted a browser session; the app holds JWTs only.
            tx.execute(
                r#"DELETE FROM "Session" WHERE "token" = ?1"#,
                [session_token],
            )?;
            Ok(())
        })
        .await;
    if let Err(error) = stored {
        return internal(error).into_response();
    }
    let mut response = redirect(&to_app(&[
        ("code", Some(&code)),
        ("state", oauth_state.as_deref()),
    ]));
    response
        .headers_mut()
        .append(header::SET_COOKIE, cleared_session_cookie());
    response
}

pub(super) async fn mobile_revoke(State(state): State<Arc<AuthState>>, body: Bytes) -> Response {
    let result = async {
        let body = body_json(&body)?;
        let token = body["refreshToken"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(invalid_body)?;
        let token_hash = hash_token(token);
        let now = (state.now_ms)();
        state
            .db
            .write(move |tx| {
                tokens::revoke_family_by_hash(tx, &token_hash, now)
                    .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
            })
            .await
            .map_err(internal)
    }
    .await;
    no_store(match result {
        Ok(()) => json_response(StatusCode::OK, &json!({ "success": true })),
        Err(error) => error.into_response(),
    })
}

pub(super) async fn get_jwks(State(state): State<Arc<AuthState>>) -> Response {
    match jwks(&state.db, &state.settings.secret, (state.now_ms)()).await {
        Ok(keys) => json_response(StatusCode::OK, &keys),
        Err(error) => internal(error).into_response(),
    }
}

/// The server-only `mobileChangePassword` (used by `POST /v1/me/password`):
/// checks the current password, ends every session and mobile login, clears
/// `mustChangePassword` and returns a fresh token pair for `device_id`.
pub async fn mobile_change_password(
    state: &AuthState,
    user_id: i64,
    current_password: &str,
    new_password: &str,
    device_id: &str,
) -> Result<Value, AuthApiError> {
    let length = |text: &str| text.encode_utf16().count();
    if length(new_password) < MIN_PASSWORD_LENGTH
        || length(new_password) > MAX_PASSWORD_LENGTH
        || length(current_password) > MAX_PASSWORD_LENGTH
        || !valid_device_id(device_id)
    {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_LENGTH_INVALID",
            "Password length is not allowed",
        ));
    }
    let stored: Option<String> = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "password" FROM "Account" WHERE "userId" = ?1 AND "providerId" = 'credential' LIMIT 1"#,
                    [user_id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten())
        })
        .await
        .map_err(internal)?;
    let (current, new) = (current_password.to_string(), new_password.to_string());
    let checked = tokio::task::spawn_blocking(move || {
        let matches = stored.is_some_and(|hash| verify_password(&hash, &current));
        (matches, current == new, hash_password(&new))
    })
    .await
    .map_err(internal)?;
    let (matches, unchanged, new_hash) = checked;
    if !matches {
        return Err(coded("BAD_REQUEST", "INVALID_PASSWORD", "Invalid password"));
    }
    if unchanged {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_UNCHANGED",
            "New password must be different from the current one",
        ));
    }
    let now = (state.now_ms)();
    let device = device_id.to_string();
    let (refresh, user) = state
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Account" SET "password" = ?1, "updatedAt" = ?3
                   WHERE "userId" = ?2 AND "providerId" = 'credential'"#,
                params![new_hash, user_id, now],
            )?;
            tx.execute(r#"DELETE FROM "Session" WHERE "userId" = ?1"#, [user_id])?;
            tx.execute(
                r#"UPDATE "User" SET "mustChangePassword" = 0, "updatedAt" = ?2 WHERE "id" = ?1"#,
                params![user_id, now],
            )?;
            tokens::revoke_all_for_user(tx, user_id, now)
                .map_err(|e| chatbotx_db::DbError::other(e.to_string()))?;
            let user = find_user_by_id(tx, user_id)?;
            let refresh = issue_refresh_token(tx, user_id, &device, None, now)?;
            Ok((refresh, user))
        })
        .await
        .map_err(internal)?;
    let user = user.ok_or_else(|| unauthorized("INVALID_USER", "User not found"))?;
    token_response(state, user.id, false, device_id, &refresh).await
}
