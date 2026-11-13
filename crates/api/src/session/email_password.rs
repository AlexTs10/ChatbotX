//! better-auth email/password endpoints as the builder configures them
//! (`packages/auth/src/server.ts`: `emailAndPassword.requireEmailVerification`,
//! `emailVerification.sendVerificationEmail`, the sign-up gate hooks):
//!
//! - `POST /api/auth/sign-up/email` -> `{ token: null, user }`, sends the verification mail
//! - `POST /api/auth/sign-in/email` -> `{ redirect: false, token, user }` + session cookie
//! - `GET  /api/auth/get-session`   -> `{ session, user }` or `null`
//! - `GET  /api/auth/verify-email`  -> marks the email verified (302 to `callbackURL`, else `{ status: true }`)
//! - `POST /api/auth/sign-out`      -> `{ success: true }`, clears the cookie
//!
//! Wire formats follow better-auth: the session cookie
//! `better-auth.session_token` is `<token>.<base64 HMAC-SHA256(secret, token)>`
//! (URL-encoded), passwords are scrypt `salt:key` hex (N=16384, r=16, p=1,
//! 64 bytes), the verification token is an HS256 JWT `{ email, iat, exp }`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_db::{Db, DbError};
use hmac::{Hmac, KeyInit, Mac};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use sha2::Sha256;

use super::{
    AuthApiError, CustomDomainSource, NewUser, OriginCheckRequest, SignUpGateDeps,
    TrustedOriginSettings, apply_bearer, context_trusted_origins, email_request_before,
    expose_auth_token, form_csrf_check, origin_check, request_cookie, trusted_origins,
    user_create_before,
};
use crate::auth::decode_query_component;
use crate::iso8601;
use crate::mail::Mailer;

/// The session cookie name (`better-auth.session_token`).
pub const SESSION_COOKIE: &str = "better-auth.session_token";
/// better-auth default session lifetime (7 days).
pub const SESSION_EXPIRES_IN_SECS: i64 = 7 * 24 * 60 * 60;
/// better-auth default verification token lifetime (1 hour).
pub const VERIFICATION_EXPIRES_IN_SECS: i64 = 60 * 60;
pub(super) const MIN_PASSWORD_LENGTH: usize = 8;
pub(super) const MAX_PASSWORD_LENGTH: usize = 128;
pub(super) const ROOT_TENANT_ID: i64 = 1;
/// `DEFAULT_SIGNUP_SUBJECT` of `@chatbotx.io/mail`.
const SIGNUP_SUBJECT: &str = "Verify your email address";

/// Settings of the auth endpoints.
#[derive(Debug, Clone)]
pub struct AuthSettings {
    /// `BETTER_AUTH_SECRET`.
    pub secret: String,
    /// `BETTER_AUTH_URL` (no trailing slash).
    pub base_url: String,
    /// `AUTH_DISABLE_SIGNUP`.
    pub sign_up_disabled: bool,
    /// The environment of the `trustedOrigins` thunk.
    pub trusted_origins: TrustedOriginSettings,
    /// `BETTER_AUTH_TRUSTED_ORIGINS` (comma list).
    pub extra_trusted_origins: Option<String>,
}

/// State of the auth router.
pub struct AuthState {
    pub db: Arc<Db>,
    pub settings: AuthSettings,
    pub mailer: Arc<dyn Mailer>,
    pub now_ms: fn() -> i64,
}

impl SignUpGateDeps for AuthState {
    fn is_sign_up_disabled(&self) -> bool {
        self.settings.sign_up_disabled
    }

    fn tenant_id(&self) -> String {
        ROOT_TENANT_ID.to_string()
    }

    async fn find_active_invitation(&self, code: &str, tenant_id: &str) -> bool {
        let code = code.to_string();
        let tenant_id = tenant_id.parse::<i64>().unwrap_or(ROOT_TENANT_ID);
        let now = (self.now_ms)();
        self.db
            .read(move |conn| {
                let found: Option<i64> = conn
                    .query_row(
                        r#"SELECT i."id" FROM "Invitation" i
                           JOIN "User" u ON u."id" = i."invitedBy"
                           WHERE i."code" = ?1 AND i."expiresAt" > ?2 AND u."tenantId" = ?3
                           LIMIT 1"#,
                        params![code, now, tenant_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(found.is_some())
            })
            .await
            .unwrap_or(false)
    }
}

impl CustomDomainSource for AuthState {
    async fn list_active_domains(&self) -> Vec<String> {
        self.db
            .read(|conn| {
                let mut statement = conn.prepare_cached(
                    r#"SELECT "domain" FROM "CustomDomain" WHERE "status" = 'active'"#,
                )?;
                let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
            .unwrap_or_else(|error| {
                tracing::error!(error = %error, "cannot list active custom domains");
                Vec::new()
            })
    }
}

impl AuthState {
    /// `ctx.context.trustedOrigins` of one request (better-auth resolves the
    /// `trustedOrigins` thunk per request).
    pub async fn request_trusted_origins(&self) -> Vec<String> {
        let app = trusted_origins(&self.settings.trusted_origins, self).await;
        context_trusted_origins(
            &self.settings.base_url,
            app,
            self.settings.extra_trusted_origins.as_deref(),
        )
    }

    /// `originCheckMiddleware`, plus `formCsrfMiddleware` when `form_csrf`.
    pub(super) async fn check_origins(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        body: Option<&Value>,
        form_csrf: bool,
    ) -> Result<(), AuthApiError> {
        let trusted = self.request_trusted_origins().await;
        let query_callback_url = query_value(uri, "callbackURL");
        origin_check(
            &OriginCheckRequest {
                method: "POST",
                headers,
                body,
                query_callback_url: query_callback_url.as_deref(),
            },
            &trusted,
        )?;
        if form_csrf {
            form_csrf_check(headers, &trusted)?;
        }
        Ok(())
    }
}

/// The router of `/api/auth/*` (email/password and sessions).
pub fn auth_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/api/auth/sign-up/email", post(sign_up_email))
        .route("/api/auth/sign-in/email", post(sign_in_email))
        .route("/api/auth/get-session", get(get_session))
        .route("/api/auth/verify-email", get(verify_email))
        .route("/api/auth/sign-out", post(sign_out))
        .route(
            "/api/auth/sign-in/magic-link",
            post(super::magic_link::sign_in_magic_link),
        )
        .route(
            "/api/auth/magic-link/verify",
            get(super::magic_link::magic_link_verify),
        )
        .route(
            "/api/auth/request-password-reset",
            post(super::magic_link::request_password_reset),
        )
        .route(
            "/api/auth/reset-password/{token}",
            get(super::magic_link::reset_password_callback),
        )
        .route(
            "/api/auth/reset-password",
            post(super::magic_link::reset_password),
        )
        .with_state(state)
}

// ---------------------------------------------------------------- responses

pub(super) fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

impl IntoResponse for AuthApiError {
    fn into_response(self) -> Response {
        let mut body = serde_json::Map::new();
        if let Some(code) = &self.code {
            body.insert("code".into(), json!(code));
        }
        body.insert("message".into(), json!(self.message));
        json_response(
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            &Value::Object(body),
        )
    }
}

pub(super) fn coded(status_name: &str, code: &str, message: &str) -> AuthApiError {
    let mut error = AuthApiError::new(status_name, message);
    error.code = Some(code.to_string());
    error
}

pub(super) fn internal(error: impl std::fmt::Display) -> AuthApiError {
    tracing::error!(error = %error, "auth endpoint failed");
    AuthApiError::new("INTERNAL_SERVER_ERROR", "Internal Server Error")
}

// ---------------------------------------------------------------- crypto

/// `count` random bytes from the OS.
fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0; count];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("OS randomness is available");
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// better-auth `generateId(32)`: 32 characters of `a-zA-Z0-9`.
pub fn generate_session_token() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    random_bytes(32)
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect()
}

fn scrypt_key(password: &str, salt: &str) -> Vec<u8> {
    // better-auth hashes `password.normalize("NFKC")`.
    let normalized = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc().normalize(password);
    let params = scrypt::Params::new(14, 16, 1).expect("valid scrypt params");
    let mut key = vec![0; 64];
    scrypt::scrypt(normalized.as_bytes(), salt.as_bytes(), &params, &mut key)
        .expect("64-byte scrypt output");
    key
}

/// better-auth `hashPassword`: `<hex salt>:<hex scrypt key>`.
pub fn hash_password(password: &str) -> String {
    let salt = hex(&random_bytes(16));
    let key = scrypt_key(password, &salt);
    format!("{salt}:{}", hex(&key))
}

/// better-auth `verifyPassword({ hash, password })` (constant-time compare).
pub fn verify_password(hash: &str, password: &str) -> bool {
    let Some((salt, key_hex)) = hash.split_once(':') else {
        return false;
    };
    let key = hex(&scrypt_key(password, salt));
    key.len() == key_hex.len()
        && key
            .bytes()
            .zip(key_hex.bytes())
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

fn hmac_sha256(secret: &str, value: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .expect("HMAC takes any key length");
    mac.update(value);
    mac.finalize().into_bytes().to_vec()
}

fn encode_uri_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// `signCookieValue(token, secret)`: URL-encoded `<token>.<base64 HMAC>`.
pub fn sign_cookie_value(token: &str, secret: &str) -> String {
    let signature = STANDARD.encode(hmac_sha256(secret, token.as_bytes()));
    encode_uri_component(&format!("{token}.{signature}"))
}

/// The token of a signed cookie value, when the signature holds.
pub fn verify_signed_cookie(raw: &str, secret: &str) -> Option<String> {
    let decoded = decode_query_component(&raw.replace('+', "%2B"));
    let (token, signature) = decoded.rsplit_once('.')?;
    let expected = STANDARD.encode(hmac_sha256(secret, token.as_bytes()));
    (signature == expected).then(|| token.to_string())
}

fn jwt_hs256(claims: &Value, secret: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signing_input = format!("{header}.{payload}");
    let signature = URL_SAFE_NO_PAD.encode(hmac_sha256(secret, signing_input.as_bytes()));
    format!("{signing_input}.{signature}")
}

/// Claims of a valid, unexpired HS256 JWT.
fn verify_jwt_hs256(token: &str, secret: &str, now_secs: i64) -> Option<Value> {
    let mut parts = token.split('.');
    let (header, payload, signature) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let expected = URL_SAFE_NO_PAD.encode(hmac_sha256(
        secret,
        format!("{header}.{payload}").as_bytes(),
    ));
    if signature != expected {
        return None;
    }
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let exp = claims["exp"].as_i64()?;
    (exp > now_secs).then_some(claims)
}

/// `createEmailVerificationToken(secret, email)`.
pub fn email_verification_token(secret: &str, email: &str, now_secs: i64) -> String {
    jwt_hs256(
        &json!({
            "email": email.to_lowercase(),
            "iat": now_secs,
            "exp": now_secs + VERIFICATION_EXPIRES_IN_SECS,
        }),
        secret,
    )
}

// ---------------------------------------------------------------- rows

#[derive(Debug, Clone)]
pub(super) struct UserRow {
    pub(super) id: i64,
    pub(super) name: Option<String>,
    pub(super) email: String,
    pub(super) email_verified: bool,
    image: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn iso(ms: i64) -> String {
    iso8601::format_millis(u64::try_from(ms).unwrap_or(0))
}

impl UserRow {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "id": self.id.to_string(),
            "name": self.name,
            "email": self.email,
            "emailVerified": self.email_verified,
            "image": self.image,
            "createdAt": iso(self.created_at),
            "updatedAt": iso(self.updated_at),
        })
    }
}

const USER_COLUMNS: &str =
    r#""id", "name", "email", "emailVerified", "image", "createdAt", "updatedAt""#;

fn user_from_row(row: &chatbotx_db::rusqlite::Row<'_>) -> chatbotx_db::rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        email_verified: row.get(3)?,
        image: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(super) fn find_user_by_email(
    conn: &Connection,
    email: &str,
) -> Result<Option<UserRow>, DbError> {
    Ok(conn
        .query_row(
            &format!(
                r#"SELECT {USER_COLUMNS} FROM "User" WHERE "email" = ?1 AND "tenantId" = ?2 LIMIT 1"#
            ),
            params![email, ROOT_TENANT_ID],
            user_from_row,
        )
        .optional()?)
}

pub(super) fn find_user_by_id(conn: &Connection, id: i64) -> Result<Option<UserRow>, DbError> {
    Ok(conn
        .query_row(
            &format!(r#"SELECT {USER_COLUMNS} FROM "User" WHERE "id" = ?1"#),
            [id],
            user_from_row,
        )
        .optional()?)
}

pub(super) fn new_id() -> i64 {
    chatbotx_core::utils::id::create_id()
        .parse()
        .expect("snowflake ids are integers")
}

// ---------------------------------------------------------------- helpers

pub(super) fn body_json(body: &Bytes) -> Result<Value, AuthApiError> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| AuthApiError::new("BAD_REQUEST", "Invalid request body"))
}

pub(super) fn query_value(uri: &Uri, name: &str) -> Option<String> {
    uri.query()?
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| decode_query_component(key) == name)
        .map(|(_, value)| decode_query_component(value))
}

fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(char::is_whitespace)
}

pub(super) fn session_cookie(token: &str, secret: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={}; Max-Age={SESSION_EXPIRES_IN_SECS}; Path=/; HttpOnly; SameSite=Lax",
        sign_cookie_value(token, secret)
    ))
    .expect("cookie is ASCII")
}

fn cleared_session_cookie() -> HeaderValue {
    HeaderValue::from_static(
        "better-auth.session_token=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax",
    )
}

/// The public origin of the request (`getPublicOriginFromRequest`): the
/// configured base URL with the request's host name.
fn verification_url(settings: &AuthSettings, headers: &HeaderMap, token: &str) -> String {
    let base = public_origin(settings, headers);
    format!("{base}/api/auth/verify-email?token={token}&callbackURL=%2F")
}

/// `getPublicOriginFromRequest`: the configured base URL with the request's host name.
pub(super) fn public_origin(settings: &AuthSettings, headers: &HeaderMap) -> String {
    let mut base = settings.base_url.clone();
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .map(|host| host.split(':').next().unwrap_or(host).to_string());
    if let (Some(host), Some(scheme_end)) = (host, base.find("://")) {
        let authority_start = scheme_end + 3;
        let authority_end = base[authority_start..]
            .find('/')
            .map_or(base.len(), |index| authority_start + index);
        let port = base[authority_start..authority_end]
            .split_once(':')
            .map(|(_, port)| format!(":{port}"))
            .unwrap_or_default();
        base.replace_range(authority_start..authority_end, &format!("{host}{port}"));
    }
    base
}

// ---------------------------------------------------------------- handlers

async fn sign_up_email(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match sign_up(&state, &uri, &headers, &body).await {
        Ok(user) => json_response(
            StatusCode::OK,
            &json!({ "token": null, "user": user.to_json() }),
        ),
        Err(error) => error.into_response(),
    }
}

async fn sign_up(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<UserRow, AuthApiError> {
    let body = body_json(body)?;
    state.check_origins(headers, uri, Some(&body), true).await?;
    let email = body["email"].as_str().unwrap_or_default().to_lowercase();
    email_request_before(state, "/sign-up/email", Some(&email), headers).await?;
    let password = body["password"].as_str().unwrap_or_default().to_string();
    let name = body["name"].as_str().map(str::to_string);
    if !is_valid_email(&email) {
        return Err(coded("BAD_REQUEST", "INVALID_EMAIL", "Invalid email"));
    }
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_SHORT",
            "Password too short",
        ));
    }
    if password.chars().count() > MAX_PASSWORD_LENGTH {
        return Err(coded(
            "BAD_REQUEST",
            "PASSWORD_TOO_LONG",
            "Password too long",
        ));
    }
    let lookup_email = email.clone();
    let existing = state
        .db
        .read(move |conn| find_user_by_email(conn, &lookup_email))
        .await
        .map_err(internal)?;
    if existing.is_some() {
        return Err(coded(
            "UNPROCESSABLE_ENTITY",
            "USER_ALREADY_EXISTS",
            "User already exists",
        ));
    }
    user_create_before(
        state,
        &NewUser {
            email: Some(email.clone()),
            is_anonymous: Some(false),
        },
        Some(headers),
    )
    .await?;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(internal)?;
    let (user_id, account_id) = (new_id(), new_id());
    let insert_email = email.clone();
    let user = state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "User" ("id", "name", "email", "emailVerified", "tenantId")
                   VALUES (?1, ?2, ?3, 0, ?4)"#,
                params![user_id, name, insert_email, ROOT_TENANT_ID],
            )?;
            tx.execute(
                r#"INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId", "tenantId")
                   VALUES (?1, ?2, 'credential', ?3, ?4, ?5)"#,
                params![account_id, user_id.to_string(), password_hash, user_id, ROOT_TENANT_ID],
            )?;
            find_user_by_id(tx, user_id)
        })
        .await
        .map_err(internal)?
        .ok_or_else(|| internal("user row missing after insert"))?;
    send_verification_mail(state, headers, &user).await;
    Ok(user)
}

async fn send_verification_mail(state: &AuthState, headers: &HeaderMap, user: &UserRow) {
    if chatbotx_core::utils::placeholder_email::is_placeholder_email(Some(&user.email)) {
        return;
    }
    let token =
        email_verification_token(&state.settings.secret, &user.email, (state.now_ms)() / 1000);
    let url = verification_url(&state.settings, headers, &token);
    let greeting = user.name.as_deref().unwrap_or(&user.email);
    let text = format!(
        "Hi {greeting},\n\nPlease verify your email address by opening this link:\n\n{url}\n"
    );
    if let Err(error) = state.mailer.send(&user.email, SIGNUP_SUBJECT, &text).await {
        tracing::warn!(error = %error, "verification mail not sent");
    }
}

async fn sign_in_email(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match sign_in(&state, &uri, &headers, &body).await {
        Ok((token, user)) => {
            let mut response = json_response(
                StatusCode::OK,
                &json!({ "redirect": false, "token": token, "user": user.to_json() }),
            );
            response.headers_mut().append(
                header::SET_COOKIE,
                session_cookie(&token, &state.settings.secret),
            );
            expose_auth_token(response.headers_mut());
            response
        }
        Err(error) => error.into_response(),
    }
}

fn invalid_credentials() -> AuthApiError {
    coded(
        "UNAUTHORIZED",
        "INVALID_EMAIL_OR_PASSWORD",
        "Invalid email or password",
    )
}

async fn sign_in(
    state: &AuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(String, UserRow), AuthApiError> {
    let body = body_json(body)?;
    state.check_origins(headers, uri, Some(&body), true).await?;
    let email = body["email"].as_str().unwrap_or_default().to_lowercase();
    let password = body["password"].as_str().unwrap_or_default().to_string();
    email_request_before(state, "/sign-in/email", Some(&email), headers).await?;
    if !is_valid_email(&email) {
        return Err(coded("BAD_REQUEST", "INVALID_EMAIL", "Invalid email"));
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
                       WHERE "userId" = ?1 AND "providerId" = 'credential' LIMIT 1"#,
                    [user.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            Ok(Some((user, hash)))
        })
        .await
        .map_err(internal)?;
    let Some((user, Some(hash))) = found else {
        // Same cost as a real check, like better-auth's dummy hash.
        let _ = tokio::task::spawn_blocking(move || hash_password(&password)).await;
        return Err(invalid_credentials());
    };
    let valid = tokio::task::spawn_blocking(move || verify_password(&hash, &password))
        .await
        .map_err(internal)?;
    if !valid {
        return Err(invalid_credentials());
    }
    if !user.email_verified {
        return Err(coded(
            "FORBIDDEN",
            "EMAIL_NOT_VERIFIED",
            "Email not verified",
        ));
    }
    let token = create_session(state, headers, user.id).await?;
    Ok((token, user))
}

/// `internalAdapter.createSession(userId)`: a 7-day session with the
/// request's IP and user agent; the new session token.
pub(super) async fn create_session(
    state: &AuthState,
    headers: &HeaderMap,
    user_id: i64,
) -> Result<String, AuthApiError> {
    let token = generate_session_token();
    let now = (state.now_ms)();
    let (session_id, session_token) = (new_id(), token.clone());
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        });
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "expiresAt", "token", "ipAddress", "userAgent", "userId")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    session_id,
                    now + SESSION_EXPIRES_IN_SECS * 1000,
                    session_token,
                    ip,
                    user_agent,
                    user_id
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(internal)?;
    Ok(token)
}

async fn get_session(State(state): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    let headers = apply_bearer(&headers, &state.settings.secret);
    let Some(token) = request_cookie(&headers, SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret))
    else {
        return json_response(StatusCode::OK, &Value::Null);
    };
    let now = (state.now_ms)();
    let found = state
        .db
        .read(move |conn| {
            let session = conn
                .query_row(
                    r#"SELECT "id", "expiresAt", "token", "ipAddress", "userAgent", "userId",
                              "createdAt", "updatedAt"
                       FROM "Session" WHERE "token" = ?1 AND "expiresAt" > ?2"#,
                    params![token, now],
                    |row| {
                        Ok((
                            json!({
                                "id": row.get::<_, i64>(0)?.to_string(),
                                "expiresAt": iso(row.get(1)?),
                                "token": row.get::<_, String>(2)?,
                                "ipAddress": row.get::<_, Option<String>>(3)?,
                                "userAgent": row.get::<_, Option<String>>(4)?,
                                "userId": row.get::<_, i64>(5)?.to_string(),
                                "createdAt": iso(row.get(6)?),
                                "updatedAt": iso(row.get(7)?),
                            }),
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((session, user_id)) = session else {
                return Ok(None);
            };
            Ok(find_user_by_id(conn, user_id)?.map(|user| (session, user)))
        })
        .await;
    match found {
        Ok(Some((session, user))) => json_response(
            StatusCode::OK,
            &json!({ "session": session, "user": user.to_json() }),
        ),
        Ok(None) => json_response(StatusCode::OK, &Value::Null),
        Err(error) => internal(error).into_response(),
    }
}

async fn verify_email(State(state): State<Arc<AuthState>>, uri: Uri) -> Response {
    let callback = query_value(&uri, "callbackURL").filter(|url| !url.is_empty());
    let now_secs = (state.now_ms)() / 1000;
    let claims = query_value(&uri, "token")
        .and_then(|token| verify_jwt_hs256(&token, &state.settings.secret, now_secs));
    let Some(email) = claims.and_then(|claims| claims["email"].as_str().map(str::to_string)) else {
        return match callback {
            Some(url) => redirect(&format!("{url}?error=invalid_token")),
            None => coded("UNAUTHORIZED", "INVALID_TOKEN", "Invalid token").into_response(),
        };
    };
    let updated = state
        .db
        .write(move |tx| {
            Ok(tx.execute(
                r#"UPDATE "User" SET "emailVerified" = 1,
                       "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                   WHERE "email" = ?1 AND "tenantId" = ?2"#,
                params![email, ROOT_TENANT_ID],
            )?)
        })
        .await;
    match (updated, callback) {
        (Ok(0), Some(url)) => redirect(&format!("{url}?error=user_not_found")),
        (Ok(0), None) => coded("UNAUTHORIZED", "USER_NOT_FOUND", "User not found").into_response(),
        (Ok(_), Some(url)) => redirect(&url),
        (Ok(_), None) => json_response(StatusCode::OK, &json!({ "status": true })),
        (Err(error), _) => internal(error).into_response(),
    }
}

pub(super) fn redirect(location: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

async fn sign_out(
    State(state): State<Arc<AuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let headers = apply_bearer(&headers, &state.settings.secret);
    let body = serde_json::from_slice::<Value>(&body).ok();
    if let Err(error) = state
        .check_origins(&headers, &uri, body.as_ref(), false)
        .await
    {
        return error.into_response();
    }
    if let Some(token) = request_cookie(&headers, SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret))
    {
        let deleted = state
            .db
            .write(move |tx| {
                tx.execute(r#"DELETE FROM "Session" WHERE "token" = ?1"#, [token])?;
                Ok(())
            })
            .await;
        if let Err(error) = deleted {
            return internal(error).into_response();
        }
    }
    let mut response = json_response(StatusCode::OK, &json!({ "success": true }));
    response
        .headers_mut()
        .append(header::SET_COOKIE, cleared_session_cookie());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_round_trips_and_rejects_a_wrong_password() {
        let hash = hash_password("Parity#12345");
        assert!(verify_password(&hash, "Parity#12345"));
        assert!(!verify_password(&hash, "Wrong#12345"));
    }

    #[test]
    fn signed_cookie_round_trips() {
        let cookie = sign_cookie_value("abc", "secret");
        assert_eq!(
            verify_signed_cookie(&cookie, "secret"),
            Some("abc".to_string())
        );
        assert_eq!(verify_signed_cookie(&cookie, "other"), None);
    }

    #[test]
    fn verification_jwt_round_trips_and_expires() {
        let token = email_verification_token("secret", "A@B.test", 1_000);
        let claims = verify_jwt_hs256(&token, "secret", 1_001).expect("valid");
        assert_eq!(claims["email"], "a@b.test");
        assert!(verify_jwt_hs256(&token, "secret", 1_000 + VERIFICATION_EXPIRES_IN_SECS).is_none());
        assert!(verify_jwt_hs256(&token, "other", 1_001).is_none());
    }
}
