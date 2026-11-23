//! better-auth OAuth sign-in for the builder's providers (Google, Facebook and
//! the `genericOAuth` provider `oidc`):
//!
//! - `POST /api/auth/sign-in/social` `{ provider, callbackURL?, errorCallbackURL?,
//!   newUserCallbackURL?, disableRedirect?, scopes?, requestSignUp? }` -> `{ url, redirect }`
//! - `POST /api/auth/sign-in/oauth2` (same body with `providerId`)
//! - `GET|POST /api/auth/callback/{id}` -> code exchange, user info,
//!   `handleOAuthUserInfo` (account owner, email match, linking, sign-up),
//!   session cookie, 302 to the callback URL.
//!
//! State is database-backed (`Verification` `auth-state:<state>`, 10 minutes);
//! the state cookie check is skipped (`skipStateCookieCheck`, as the builder).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::email_password::{
    ROOT_TENANT_ID, body_json, coded, create_session, find_user_by_email, find_user_by_id,
    internal, json_response, new_id, on_user_created, query_value, random_string, redirect,
    session_cookie,
};
use super::{
    AuthApiError, AuthState, ClaimAdapter, ClaimUser, CreatedAt, ExistingAccount,
    GENERIC_OAUTH_PROVIDER_ID, LinkAccount, NewUser, claim_unverified_account_before_link,
    expose_auth_token, map_facebook_profile_to_user, user_create_before,
};

/// Lifetime of the OAuth state (`expiresAt: Date.now() + 6e5`).
const STATE_EXPIRES_IN_MS: i64 = 10 * 60 * 1000;
/// `FACEBOOK_SSO_SCOPES` (= `MESSENGER_SCOPES`).
pub const FACEBOOK_SSO_SCOPES: &[&str] = &[
    "email",
    "public_profile",
    "pages_manage_ads",
    "pages_manage_metadata",
    "pages_read_engagement",
    "pages_read_user_content",
    "pages_manage_posts",
    "pages_manage_engagement",
    "pages_messaging",
    "pages_show_list",
    "business_management",
    "pages_utility_messaging",
];

/// How the provider reads the user (`getUserInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthKind {
    /// The id token's claims.
    Google,
    /// Graph `/me` + `mapFacebookProfileToUser`.
    Facebook,
    /// The id token, else the userinfo endpoint (`genericOAuth`).
    Generic,
}

/// One configured provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProvider {
    pub id: String,
    pub kind: OAuthKind,
    pub client_id: String,
    pub client_secret: String,
    pub authorization_url: String,
    pub token_url: String,
    /// Facebook Graph `/me` or the generic userinfo endpoint.
    pub user_info_url: Option<String>,
    /// The default scopes plus the configured ones.
    pub scopes: Vec<String>,
    pub pkce: bool,
    /// The pinned redirect URI (`<broker origin>/api/auth/callback/<id>`).
    pub redirect_uri: String,
    /// Extra authorization URL parameters.
    pub extra_params: Vec<(String, String)>,
}

impl OAuthProvider {
    /// The `google()` provider of better-auth.
    pub fn google(client_id: &str, client_secret: &str, redirect_origin: &str) -> OAuthProvider {
        OAuthProvider {
            id: "google".into(),
            kind: OAuthKind::Google,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            user_info_url: None,
            scopes: vec!["email".into(), "profile".into(), "openid".into()],
            pkce: true,
            redirect_uri: format!("{redirect_origin}/api/auth/callback/google"),
            extra_params: vec![("include_granted_scopes".into(), "true".into())],
        }
    }

    /// The `facebook()` provider with the builder's SSO scopes.
    pub fn facebook(client_id: &str, client_secret: &str, redirect_origin: &str) -> OAuthProvider {
        let mut scopes: Vec<String> = vec!["email".into(), "public_profile".into()];
        scopes.extend(FACEBOOK_SSO_SCOPES.iter().map(|scope| (*scope).to_string()));
        OAuthProvider {
            id: "facebook".into(),
            kind: OAuthKind::Facebook,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            authorization_url: "https://www.facebook.com/v24.0/dialog/oauth".into(),
            token_url: "https://graph.facebook.com/v24.0/oauth/access_token".into(),
            user_info_url: Some(
                "https://graph.facebook.com/me?fields=id,name,email,picture".into(),
            ),
            scopes,
            pkce: false,
            redirect_uri: format!("{redirect_origin}/api/auth/callback/facebook"),
            extra_params: Vec::new(),
        }
    }
}

/// Root-tenant platform credentials for the social providers (cloud only):
/// `PlatformCredential` rows without a user, `google` and `messenger`.
pub struct PlatformSocial {
    pub encryption: Arc<Encryption>,
    /// `getBrokerOrigin()`.
    pub redirect_origin: String,
}

/// State of the OAuth router.
pub struct OAuthState {
    pub auth: Arc<AuthState>,
    pub http: reqwest::Client,
    /// Providers known at start (the generic provider, tests).
    pub providers: Vec<OAuthProvider>,
    /// Read per request, so a credential saved in the admin UI applies at once.
    pub platform: Option<PlatformSocial>,
    /// `trustedProviders` (`google`, `facebook`, plus `oidc` with
    /// `AUTH_OAUTH_TRUST_EMAIL`).
    pub trusted_providers: Vec<String>,
    /// `AUTH_OAUTH_TRUST_EMAIL` (claim hook).
    pub trust_oauth_email: bool,
}

impl OAuthState {
    async fn provider(&self, id: &str) -> Option<OAuthProvider> {
        if let Some(provider) = self.providers.iter().find(|p| p.id == id) {
            return Some(provider.clone());
        }
        let platform = self.platform.as_ref()?;
        let credential_type = match id {
            "google" => "google",
            "facebook" => "messenger",
            _ => return None,
        };
        let (client_id, client_secret) =
            platform_credential(&self.auth.db, &platform.encryption, credential_type).await?;
        let origin = &platform.redirect_origin;
        Some(match id {
            "google" => OAuthProvider::google(&client_id, &client_secret, origin),
            _ => OAuthProvider::facebook(&client_id, &client_secret, origin),
        })
    }
}

/// `platformCredentialService.findDecryptedPlatform({ type })`: `(clientId, clientSecret)`.
pub(crate) async fn platform_credential(
    db: &Db,
    encryption: &Encryption,
    credential_type: &'static str,
) -> Option<(String, String)> {
    let value: Option<String> = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "value" FROM "PlatformCredential"
                       WHERE "userId" IS NULL AND "type" = ?1 LIMIT 1"#,
                    [credential_type],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .await
        .ok()
        .flatten();
    let blob: EncryptedData = serde_json::from_str(&value?).ok()?;
    let aad = chatbotx_db::repo::partials::credential::platform_credential_aad(credential_type);
    match encryption.decrypt_object::<Value>(&blob, Some(&aad)) {
        Ok(config) => {
            let text = |name: &str| config[name].as_str().map(str::to_string);
            Some((text("clientId")?, text("clientSecret")?))
                .filter(|(id, secret)| !id.is_empty() && !secret.is_empty())
        }
        Err(error) => {
            tracing::error!(error = %error, credential_type, "Failed to decrypt platform credential");
            None
        }
    }
}

/// The router of the OAuth sign-in endpoints.
pub fn oauth_router(state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/api/auth/sign-in/social", post(sign_in_social))
        .route("/api/auth/sign-in/oauth2", post(sign_in_oauth2))
        .route("/api/auth/callback/{id}", any(callback))
        .with_state(state)
}

async fn sign_in_social(
    State(state): State<Arc<OAuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    sign_in(&state, &uri, &headers, &body, "provider")
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn sign_in_oauth2(
    State(state): State<Arc<OAuthState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    sign_in(&state, &uri, &headers, &body, "providerId")
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn sign_in(
    state: &OAuthState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
    provider_field: &str,
) -> Result<Response, AuthApiError> {
    let body = body_json(body)?;
    state
        .auth
        .check_origins(headers, uri, Some(&body), false)
        .await?;
    let provider_id = body[provider_field].as_str().unwrap_or_default();
    let Some(provider) = state.provider(provider_id).await else {
        tracing::error!(provider = provider_id, "Provider not found");
        return Err(coded(
            "NOT_FOUND",
            "PROVIDER_NOT_FOUND",
            "Provider not found",
        ));
    };
    if body.get("idToken").is_some_and(|token| !token.is_null()) {
        // ponytail: id-token sign-in (native Google/Facebook SDKs) is not
        // ported; add it with the mobile app port.
        return Err(coded(
            "NOT_FOUND",
            "ID_TOKEN_NOT_SUPPORTED",
            "id token not supported",
        ));
    }
    let text = |name: &str| {
        body[name]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let callback_url = text("callbackURL").unwrap_or_else(|| state.auth.settings.base_url.clone());
    let code_verifier = random_string(128);
    let oauth_state = random_string(32);
    let now = (state.auth.now_ms)();
    let mut data = json!({
        "callbackURL": callback_url,
        "codeVerifier": code_verifier,
        "expiresAt": now + STATE_EXPIRES_IN_MS,
        "oauthState": oauth_state,
    });
    for (field, key) in [
        ("errorCallbackURL", "errorURL"),
        ("newUserCallbackURL", "newUserURL"),
    ] {
        if let Some(value) = text(field) {
            data[key] = json!(value);
        }
    }
    if let Some(flag) = body["requestSignUp"].as_bool() {
        data["requestSignUp"] = json!(flag);
    }
    let identifier = format!("auth-state:{oauth_state}");
    let verification_id = new_id();
    state
        .auth
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![
                    verification_id,
                    identifier,
                    data.to_string(),
                    now + STATE_EXPIRES_IN_MS
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(internal)?;
    let mut scopes = provider.scopes.clone();
    if let Some(extra) = body["scopes"].as_array() {
        scopes.extend(extra.iter().filter_map(Value::as_str).map(str::to_string));
    }
    let url = authorization_url(&provider, &oauth_state, &code_verifier, &scopes);
    let disable_redirect = body["disableRedirect"].as_bool().unwrap_or(false);
    let mut response = json_response(
        StatusCode::OK,
        &json!({ "url": url, "redirect": !disable_redirect }),
    );
    if !disable_redirect && let Ok(location) = url.parse() {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

/// `createAuthorizationURL`.
fn authorization_url(
    provider: &OAuthProvider,
    state: &str,
    code_verifier: &str,
    scopes: &[String],
) -> String {
    let mut url = url::Url::parse(&provider.authorization_url)
        .unwrap_or_else(|_| url::Url::parse("http://invalid/").expect("static url"));
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &provider.client_id);
        query.append_pair("state", state);
        if !scopes.is_empty() {
            query.append_pair("scope", &scopes.join(" "));
        }
        query.append_pair("redirect_uri", &provider.redirect_uri);
        if provider.pkce {
            query.append_pair("code_challenge_method", "S256");
            query.append_pair("code_challenge", &code_challenge(code_verifier));
        }
        for (key, value) in &provider.extra_params {
            query.append_pair(key, value);
        }
    }
    url.to_string()
}

/// `generateCodeChallenge`: base64url(SHA-256(verifier)).
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

// ---------------------------------------------------------------- callback

/// The parsed state (`stateDataSchema`).
struct StateData {
    callback_url: String,
    code_verifier: String,
    error_url: Option<String>,
    new_user_url: Option<String>,
}

fn error_redirect(base: &str, error: &str, description: Option<&str>) -> Response {
    let mut url = base.to_string();
    url.push(if url.contains('?') { '&' } else { '?' });
    let mut pairs = url::form_urlencoded::Serializer::new(String::new());
    pairs.append_pair("error", error);
    if let Some(description) = description {
        pairs.append_pair("error_description", description);
    }
    url.push_str(&pairs.finish());
    redirect(&url)
}

async fn callback(
    State(state): State<Arc<OAuthState>>,
    Path(id): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let default_error_url = format!("{}/api/auth/error", state.auth.settings.base_url);
    if method == Method::POST {
        // `response_mode=form_post`: re-issue as a GET with the merged params.
        let mut params: Vec<(String, String)> =
            url::form_urlencoded::parse(&body).into_owned().collect();
        params.extend(
            url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned(),
        );
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        return redirect(&format!(
            "{}/api/auth/callback/{id}?{query}",
            state.auth.settings.base_url
        ));
    }
    let Some(oauth_state) = query_value(&uri, "state").filter(|s| !s.is_empty()) else {
        return error_redirect(&default_error_url, "state_not_found", None);
    };
    let data = match parse_state(&state.auth, &oauth_state).await {
        Ok(data) => data,
        Err((code, error_url)) => {
            return error_redirect(
                error_url.as_deref().unwrap_or(&default_error_url),
                code,
                None,
            );
        }
    };
    let error_url = data.error_url.clone().unwrap_or(default_error_url);
    if let Some(error) = query_value(&uri, "error").filter(|e| !e.is_empty()) {
        let description = query_value(&uri, "error_description");
        return error_redirect(&error_url, &error, description.as_deref());
    }
    let Some(code) = query_value(&uri, "code").filter(|c| !c.is_empty()) else {
        return error_redirect(&error_url, "no_code", None);
    };
    let Some(provider) = state.provider(&id).await else {
        return error_redirect(&error_url, "oauth_provider_not_found", None);
    };
    let now = (state.auth.now_ms)();
    let tokens = match exchange_code(&state.http, &provider, &code, &data.code_verifier, now).await
    {
        Some(tokens) => tokens,
        None => return error_redirect(&error_url, "invalid_code", None),
    };
    let Some(info) = user_info(&state.http, &provider, &tokens).await else {
        return error_redirect(&error_url, "unable_to_get_user_info", None);
    };
    let Some(email) = info.email.clone().filter(|e| !e.is_empty()) else {
        return error_redirect(&error_url, "email_not_found", None);
    };
    match handle_user_info(&state, &headers, &provider, &tokens, &info, &email).await {
        Ok(Ok((user_id, is_register))) => {
            let token = match create_session(&state.auth, &headers, user_id).await {
                Ok(token) => token,
                Err(error) => return error.into_response(),
            };
            let target = if is_register {
                data.new_user_url.as_deref().unwrap_or(&data.callback_url)
            } else {
                &data.callback_url
            };
            let mut response = redirect(target);
            response.headers_mut().append(
                header::SET_COOKIE,
                session_cookie(&token, &state.auth.settings.secret),
            );
            expose_auth_token(response.headers_mut());
            response
        }
        Ok(Err(error)) => error_redirect(&error_url, &error.replace(' ', "_"), None),
        Err(error) => error.into_response(),
    }
}

/// `parseGenericState` (database strategy). `Err((code, errorURL))`.
async fn parse_state(
    auth: &AuthState,
    oauth_state: &str,
) -> Result<StateData, (&'static str, Option<String>)> {
    let identifier = format!("auth-state:{oauth_state}");
    let now = (auth.now_ms)();
    let stored: Option<String> = auth
        .db
        .write(move |tx| {
            let value: Option<String> = tx
                .query_row(
                    r#"SELECT "value" FROM "Verification" WHERE "identifier" = ?1 AND "expiresAt" > ?2 LIMIT 1"#,
                    params![identifier, now],
                    |row| row.get(0),
                )
                .optional()?;
            if value.is_some() {
                tx.execute(r#"DELETE FROM "Verification" WHERE "identifier" = ?1"#, [identifier])?;
            }
            Ok(value)
        })
        .await
        .map_err(|_| ("internal_server_error", None))?;
    let value: Value = stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or(("state_mismatch", None))?;
    let text = |name: &str| value[name].as_str().map(str::to_string);
    let error_url = text("errorURL");
    if text("oauthState").is_some_and(|stored| stored != oauth_state) {
        return Err(("state_mismatch", error_url));
    }
    let (Some(callback_url), Some(code_verifier), Some(expires_at)) = (
        text("callbackURL"),
        text("codeVerifier"),
        value["expiresAt"].as_i64(),
    ) else {
        return Err(("state_mismatch", error_url));
    };
    if expires_at < now {
        return Err(("state_mismatch", error_url));
    }
    Ok(StateData {
        callback_url,
        code_verifier,
        error_url,
        new_user_url: text("newUserURL"),
    })
}

/// `getOAuth2Tokens` of the token response.
struct Tokens {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    access_token_expires_at: Option<i64>,
    scope: Option<String>,
}

/// `validateAuthorizationCode` (client secret in the body).
async fn exchange_code(
    http: &reqwest::Client,
    provider: &OAuthProvider,
    code: &str,
    code_verifier: &str,
    now: i64,
) -> Option<Tokens> {
    // The serializer is not `Send`: finish it before the request.
    let form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", code);
        if provider.pkce {
            form.append_pair("code_verifier", code_verifier);
        }
        form.append_pair("redirect_uri", &provider.redirect_uri);
        form.append_pair("client_id", &provider.client_id);
        form.append_pair("client_secret", &provider.client_secret);
        form.finish()
    };
    let response = http
        .post(&provider.token_url)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/json")
        .body(form)
        .send()
        .await
        .inspect_err(|error| tracing::error!(error = %error, "OAuth token request failed"))
        .ok()?;
    if !response.status().is_success() {
        tracing::error!(status = %response.status(), provider = %provider.id, "OAuth token request refused");
        return None;
    }
    let data: Value = serde_json::from_slice(&response.bytes().await.ok()?).ok()?;
    let text = |name: &str| data[name].as_str().map(str::to_string);
    Some(Tokens {
        access_token: text("access_token"),
        refresh_token: text("refresh_token"),
        id_token: text("id_token"),
        access_token_expires_at: data["expires_in"].as_i64().map(|secs| now + secs * 1000),
        scope: text("scope").map(|scope| scope.split(' ').collect::<Vec<_>>().join(",")),
    })
}

/// The user of the provider (`getUserInfo` + `accountKey`).
struct UserInfo {
    account_id: String,
    email: Option<String>,
    email_verified: bool,
    name: String,
    image: Option<String>,
}

/// The unverified claims of a JWT (`decodeJwt`).
fn jwt_claims(token: &str) -> Option<Value> {
    let segment = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segment.trim_end_matches('=')).ok()?).ok()
}

fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

async fn user_info(
    http: &reqwest::Client,
    provider: &OAuthProvider,
    tokens: &Tokens,
) -> Option<UserInfo> {
    // The generic provider reads the id token when it has `sub` and `email`.
    let id_claims = tokens
        .id_token
        .as_deref()
        .and_then(jwt_claims)
        .filter(|claims| claims["sub"].is_string() && claims["email"].is_string());
    let profile = match (provider.kind, id_claims) {
        (OAuthKind::Google, _) => jwt_claims(tokens.id_token.as_deref()?)?,
        (OAuthKind::Generic, Some(claims)) => claims,
        _ => {
            let response = http
                .get(provider.user_info_url.as_deref()?)
                .bearer_auth(tokens.access_token.as_deref()?)
                .header(header::ACCEPT, "application/json")
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            serde_json::from_slice(&response.bytes().await.ok()?).ok()?
        }
    };
    let text = |name: &str| {
        profile[name]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let mut info = match provider.kind {
        OAuthKind::Google => UserInfo {
            account_id: id_text(&profile["sub"])?,
            email: text("email"),
            email_verified: profile["email_verified"].as_bool().unwrap_or(false),
            name: text("name").unwrap_or_default(),
            image: text("picture"),
        },
        OAuthKind::Facebook => UserInfo {
            account_id: id_text(&profile["id"])?,
            email: text("email"),
            email_verified: profile["email_verified"].as_bool().unwrap_or(false),
            name: text("name").unwrap_or_default(),
            image: profile["picture"]["data"]["url"]
                .as_str()
                .map(str::to_string),
        },
        OAuthKind::Generic => {
            let joined = [text("given_name"), text("family_name")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            UserInfo {
                account_id: id_text(&profile["id"]).or_else(|| id_text(&profile["sub"]))?,
                email: text("email"),
                email_verified: profile["email_verified"].as_bool().unwrap_or(false),
                name: text("name")
                    .or_else(|| text("preferred_username"))
                    .unwrap_or(joined),
                image: text("picture"),
            }
        }
    };
    if provider.kind == OAuthKind::Facebook {
        let mapped = map_facebook_profile_to_user(&profile);
        if let Some(email) = mapped["email"].as_str() {
            info.email = Some(email.to_string());
            info.email_verified = false;
        }
    }
    Some(info)
}

/// The claim hook over the database.
struct DbClaimAdapter<'a> {
    db: &'a Db,
    now_ms: i64,
    trust_oauth_email: bool,
}

impl ClaimAdapter for DbClaimAdapter<'_> {
    async fn find_user_by_id(&self, id: &str) -> Result<Option<ClaimUser>, String> {
        let id: i64 = id.parse().map_err(|_| "invalid user id".to_string())?;
        self.db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "email", "emailVerified", "createdAt" FROM "User" WHERE "id" = ?1"#,
                        [id],
                        |row| {
                            Ok(ClaimUser {
                                id: id.to_string(),
                                email: row.get(0)?,
                                email_verified: row.get(1)?,
                                created_at: Some(CreatedAt::Millis(row.get(2)?)),
                            })
                        },
                    )
                    .optional()?)
            })
            .await
            .map_err(|e| e.to_string())
    }

    async fn find_accounts(&self, user_id: &str) -> Result<Vec<ExistingAccount>, String> {
        let user_id: i64 = user_id.parse().map_err(|_| "invalid user id".to_string())?;
        self.db
            .read(move |conn| {
                let mut statement = conn
                    .prepare(r#"SELECT "id", "providerId" FROM "Account" WHERE "userId" = ?1"#)?;
                let rows = statement.query_map([user_id], |row| {
                    Ok(ExistingAccount {
                        id: row.get::<_, i64>(0)?.to_string(),
                        provider_id: row.get(1)?,
                    })
                })?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete_account(&self, id: &str) -> Result<(), String> {
        let id: i64 = id.parse().map_err(|_| "invalid account id".to_string())?;
        self.db
            .write(move |tx| {
                tx.execute(r#"DELETE FROM "Account" WHERE "id" = ?1"#, [id])?;
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())
    }

    async fn mark_email_verified(&self, user_id: &str) -> Result<(), String> {
        let user_id: i64 = user_id.parse().map_err(|_| "invalid user id".to_string())?;
        set_email_verified(self.db, user_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete_sessions(&self, user_id: &str) -> Result<u64, String> {
        let user_id: i64 = user_id.parse().map_err(|_| "invalid user id".to_string())?;
        self.db
            .write(move |tx| {
                Ok(tx.execute(r#"DELETE FROM "Session" WHERE "userId" = ?1"#, [user_id])? as u64)
            })
            .await
            .map_err(|e| e.to_string())
    }

    fn trust_oauth_email(&self) -> bool {
        self.trust_oauth_email
    }

    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

async fn set_email_verified(db: &Db, user_id: i64) -> Result<(), chatbotx_db::DbError> {
    db.write(move |tx| {
        tx.execute(
            r#"UPDATE "User" SET "emailVerified" = 1,
                   "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
               WHERE "id" = ?1"#,
            [user_id],
        )?;
        Ok(())
    })
    .await
}

/// `internalAdapter.createAccount` / `linkAccount` (after the claim hook).
async fn insert_account(
    state: &OAuthState,
    provider_id: &str,
    account_id: &str,
    user_id: i64,
    tokens: &Tokens,
) -> Result<(), String> {
    let adapter = DbClaimAdapter {
        db: &state.auth.db,
        now_ms: (state.auth.now_ms)(),
        trust_oauth_email: state.trust_oauth_email,
    };
    claim_unverified_account_before_link(
        &LinkAccount {
            user_id: user_id.to_string(),
            provider_id: provider_id.to_string(),
            id_token: tokens.id_token.clone(),
        },
        Some(&adapter),
    )
    .await?;
    let row = (
        new_id(),
        account_id.to_string(),
        provider_id.to_string(),
        tokens.access_token.clone(),
        tokens.refresh_token.clone(),
        tokens.id_token.clone(),
        tokens.access_token_expires_at,
        tokens.scope.clone(),
    );
    state
        .auth
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Account" ("id", "accountId", "providerId", "accessToken", "refreshToken",
                       "idToken", "accessTokenExpiresAt", "scope", "userId", "tenantId")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
                params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, user_id, ROOT_TENANT_ID],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

/// `handleOAuthUserInfo`: `Ok(Ok((userId, isRegister)))`, `Ok(Err(error))`
/// for a redirect error, `Err` for an API error response.
async fn handle_user_info(
    state: &OAuthState,
    headers: &HeaderMap,
    provider: &OAuthProvider,
    tokens: &Tokens,
    info: &UserInfo,
    email: &str,
) -> Result<Result<(i64, bool), &'static str>, AuthApiError> {
    let email = email.to_lowercase();
    let (provider_id, account_id) = (provider.id.clone(), info.account_id.clone());
    let lookup_email = email.clone();
    let found = state
        .auth
        .db
        .read(move |conn| {
            let owner: Option<(i64, i64)> = conn
                .query_row(
                    r#"SELECT "id", "userId" FROM "Account"
                       WHERE "providerId" = ?1 AND "accountId" = ?2 AND "tenantId" = ?3 LIMIT 1"#,
                    params![provider_id, account_id, ROOT_TENANT_ID],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((account_row, user_id)) = owner {
                return Ok(Some((find_user_by_id(conn, user_id)?, Some(account_row))));
            }
            Ok(find_user_by_email(conn, &lookup_email)?.map(|user| (Some(user), None)))
        })
        .await
        .map_err(internal)?;
    let user_id = match found {
        // `accountOwner.kind === "orphaned"`.
        Some((None, _)) => return Ok(Err("unable to link account")),
        Some((Some(user), Some(account_row))) => {
            let (access, refresh, id_token, expires) = (
                tokens.access_token.clone(),
                tokens.refresh_token.clone(),
                tokens.id_token.clone(),
                tokens.access_token_expires_at,
            );
            state
                .auth
                .db
                .write(move |tx| {
                    // `updateAccountOnSignIn`: only the defined token fields.
                    tx.execute(
                        r#"UPDATE "Account" SET
                               "accessToken" = COALESCE(?2, "accessToken"),
                               "refreshToken" = COALESCE(?3, "refreshToken"),
                               "idToken" = COALESCE(?4, "idToken"),
                               "accessTokenExpiresAt" = COALESCE(?5, "accessTokenExpiresAt"),
                               "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                           WHERE "id" = ?1"#,
                        params![account_row, access, refresh, id_token, expires],
                    )?;
                    Ok(())
                })
                .await
                .map_err(internal)?;
            if info.email_verified && !user.email_verified && email == user.email {
                set_email_verified(&state.auth.db, user.id)
                    .await
                    .map_err(internal)?;
            }
            return Ok(Ok((user.id, false)));
        }
        Some((Some(user), None)) => {
            let trusted = state.trusted_providers.contains(&provider.id);
            // `requireLocalEmailVerified: false`.
            if !trusted && !info.email_verified {
                return Ok(Err("account not linked"));
            }
            if let Err(error) =
                insert_account(state, &provider.id, &info.account_id, user.id, tokens).await
            {
                tracing::error!(error = %error, "Unable to link account");
                return Ok(Err("unable to link account"));
            }
            if info.email_verified && !user.email_verified && email == user.email {
                set_email_verified(&state.auth.db, user.id)
                    .await
                    .map_err(internal)?;
            }
            user.id
        }
        None => {
            user_create_before(
                state.auth.as_ref(),
                &NewUser {
                    email: Some(email.clone()),
                    is_anonymous: Some(false),
                },
                Some(headers),
            )
            .await?;
            let user_id = new_id();
            let (name, image, verified, insert_email) = (
                info.name.clone(),
                info.image.clone(),
                info.email_verified,
                email.clone(),
            );
            state
                .auth
                .db
                .write(move |tx| {
                    tx.execute(
                        r#"INSERT INTO "User" ("id", "name", "email", "emailVerified", "image", "tenantId")
                           VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                        params![user_id, name, insert_email, verified, image, ROOT_TENANT_ID],
                    )?;
                    on_user_created(tx, &insert_email, false)?;
                    Ok(())
                })
                .await
                .map_err(internal)?;
            if let Err(error) =
                insert_account(state, &provider.id, &info.account_id, user_id, tokens).await
            {
                tracing::error!(error = %error, "Unable to create OAuth user");
                let _ = state
                    .auth
                    .db
                    .write(move |tx| {
                        tx.execute(r#"DELETE FROM "User" WHERE "id" = ?1"#, [user_id])?;
                        Ok(())
                    })
                    .await;
                return Ok(Err("unable to create user"));
            }
            return Ok(Ok((user_id, true)));
        }
    };
    Ok(Ok((user_id, false)))
}

/// The generic provider's endpoints after discovery (`discoveryUrl`).
pub async fn discover_generic_provider(
    http: &reqwest::Client,
    config: &super::GenericOAuthProvider,
) -> Option<OAuthProvider> {
    let (authorization_url, token_url, user_info_url) = match &config.discovery_url {
        Some(discovery_url) => {
            let response = http.get(discovery_url).send().await.ok()?;
            let document: Value = serde_json::from_slice(&response.bytes().await.ok()?).ok()?;
            let text = |name: &str| document[name].as_str().map(str::to_string);
            (
                text("authorization_endpoint")?,
                text("token_endpoint")?,
                text("userinfo_endpoint").or_else(|| config.user_info_url.clone()),
            )
        }
        None => (
            config.authorization_url.clone()?,
            config.token_url.clone()?,
            config.user_info_url.clone(),
        ),
    };
    Some(OAuthProvider {
        id: GENERIC_OAUTH_PROVIDER_ID.to_string(),
        kind: OAuthKind::Generic,
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        authorization_url,
        token_url,
        user_info_url,
        scopes: config.scopes.clone(),
        pkce: config.pkce,
        redirect_uri: config.redirect_uri.clone(),
        extra_params: Vec::new(),
    })
}

/// The `AUTH_OAUTH_*` variables.
pub(crate) fn generic_oauth_env() -> super::GenericOAuthEnv {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    super::GenericOAuthEnv {
        enabled: var("AUTH_OAUTH_ENABLED"),
        client_id: var("AUTH_OAUTH_CLIENT_ID"),
        client_secret: var("AUTH_OAUTH_CLIENT_SECRET"),
        auth_url: var("AUTH_OAUTH_AUTH_URL"),
        token_url: var("AUTH_OAUTH_TOKEN_URL"),
        discovery_url: var("AUTH_OAUTH_DISCOVERY_URL"),
        userinfo_url: var("AUTH_OAUTH_USERINFO_URL"),
        scopes: var("AUTH_OAUTH_SCOPES"),
        trust_email: var("AUTH_OAUTH_TRUST_EMAIL"),
    }
}

/// `resolveAuthPageOptions` minus the sign-up flag: the social providers with a
/// root-tenant credential (cloud only) and the generic IdP button (cloud /
/// enterprise with `AUTH_OAUTH_*`).
pub(crate) async fn auth_page_providers(
    auth: &AuthState,
    edition: &str,
) -> (Vec<&'static str>, Value) {
    let mut enabled = Vec::new();
    if edition == "cloud"
        && let Ok(keys) = chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv)
    {
        let encryption = Encryption::new(keys);
        for (provider, credential_type) in [("google", "google"), ("facebook", "messenger")] {
            if platform_credential(&auth.db, &encryption, credential_type)
                .await
                .is_some()
            {
                enabled.push(provider);
            }
        }
    }
    let broker_url = auth
        .settings
        .trusted_origins
        .broker_url
        .clone()
        .unwrap_or_else(|| auth.settings.trusted_origins.builder_url.clone());
    let generic = matches!(edition, "cloud" | "enterprise")
        && super::generic_oauth_provider(&generic_oauth_env(), &broker_url).is_some();
    let button = if generic {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        serde_json::json!({
            "displayName": var("NEXT_PUBLIC_AUTH_OAUTH_DISPLAY_NAME").unwrap_or_else(|| "SSO".into()),
            "logoUrl": var("NEXT_PUBLIC_AUTH_OAUTH_LOGO_URL"),
        })
    } else {
        Value::Null
    };
    (enabled, button)
}

/// The OAuth router state of the server, from the environment:
/// - the generic provider (`AUTH_OAUTH_*`) when the edition is `cloud` or
///   `enterprise` (`hasEnterpriseFeatures`; the license check is not ported);
/// - the root-tenant social credentials (`PlatformCredential`) on `cloud`
///   (`resolveCredentialForTenant`: community has no social providers).
pub async fn oauth_state_from_env(auth: Arc<AuthState>, edition: &str) -> OAuthState {
    let env = generic_oauth_env();
    let builder_url = auth.settings.trusted_origins.builder_url.clone();
    let broker_url = auth
        .settings
        .trusted_origins
        .broker_url
        .clone()
        .unwrap_or_else(|| builder_url.clone());
    let broker_origin = url::Url::parse(&broker_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| auth.settings.base_url.clone());
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();
    let licensed = matches!(edition, "cloud" | "enterprise");
    let mut providers = Vec::new();
    if licensed && let Some(config) = super::generic_oauth_provider(&env, &broker_url) {
        match discover_generic_provider(&http, &config).await {
            Some(provider) => providers.push(provider),
            None => tracing::error!("generic OAuth provider discovery failed"),
        }
    }
    let platform = if edition == "cloud" {
        match chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv) {
            Ok(keys) => Some(PlatformSocial {
                encryption: Arc::new(Encryption::new(keys)),
                redirect_origin: broker_origin,
            }),
            Err(error) => {
                tracing::error!(error = %error, "social sign-in off: ENCRYPTION_KEY is not valid");
                None
            }
        }
    } else {
        None
    };
    OAuthState {
        trusted_providers: super::trusted_providers(&env),
        trust_oauth_email: super::social::env_bool(env.trust_email.as_deref()),
        auth,
        http,
        providers,
        platform,
    }
}

/// `bootstrapPlatformAdminAtStartup`.
pub async fn bootstrap_platform_admin_at_startup(db: &Db) {
    let Some(email) = super::platform_admin_email() else {
        return;
    };
    let provisioned = db
        .write(move |tx| {
            chatbotx_db::repo::platform_admin::bootstrap_root_admin(tx, &email)
                .map_err(|error| chatbotx_db::DbError::other(error.to_string()))
        })
        .await;
    match provisioned {
        Ok(true) => tracing::info!("Provisioned platform bootstrap administrator"),
        Ok(false) => {}
        Err(error) => tracing::warn!(error = %error, "platform admin bootstrap failed"),
    }
}
