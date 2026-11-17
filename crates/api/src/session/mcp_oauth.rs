//! The MCP OAuth authorization server (Q.A1): better-auth `mcp()` /
//! `oauthProvider()` as `packages/auth/src/mcp-config.ts` configures it, and
//! the builder routes around it.
//!
//! - `GET /.well-known/oauth-authorization-server`, `/.well-known/openid-configuration`
//! - `GET /.well-known/oauth-protected-resource/{*path}` (builder route)
//! - `POST /api/auth/oauth2/register` (dynamic, unauthenticated, 20 / window / IP)
//! - `GET /api/auth/oauth2/authorize` (code flow, PKCE S256, signed login / consent query)
//! - `POST /api/auth/oauth2/token` (`authorization_code`, `refresh_token`)
//! - `POST /api/auth/oauth2/consent` is 404 (consent only through the action)
//! - `GET /oauth/sign-in` (builder route)
//! - [`submit_oauth_consent`] / [`oauth_consent_page`]: the consent server
//!   action and page data, served as `/rpc/authActions/*`.
//!
//! Security: codes, refresh tokens and client secrets are stored as
//! SHA-256 (base64url) hashes; a code is deleted on first use (a replay
//! deletes the tokens it issued); redirect URIs match a registered one
//! exactly (loopback port aside, as the original); PKCE S256 is required;
//! the signed query is HMAC-SHA256 with `BETTER_AUTH_SECRET` and checked in
//! constant time; consent rows are bound to the session user.
//!
//! ponytail: not ported (no MCP client needs them): `request` / `request_uri`
//! objects, the `claims` parameter, `max_age`, DPoP, `private_key_jwt`,
//! client credentials, introspection, revocation, userinfo, end-session,
//! Client ID Metadata Documents and `OAuthClientResource` links
//! (`enforcePerClientResources: false`, one resource).

// ponytail: the early-return `Err` is the HTTP response itself, built once per
// request; boxing it buys nothing.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, assert_api_not_rate_limited};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256, Sha512};

use super::email_password::{hmac_sha256, new_id, random_string};
use super::{
    AuthState, MCP_ACCESS_TOKEN_TTL_SECONDS, MCP_OAUTH_SCOPES, MCP_REFRESH_TOKEN_TTL_SECONDS,
    McpResourceRewrite, apply_bearer, builder_origin, default_mcp_resource, request_cookie,
    sign_jwt, signing_key, symmetric_decrypt, symmetric_encrypt, verify_signed_cookie,
};

const CODE_TTL_SECONDS: i64 = 600;
const ID_TOKEN_TTL_SECONDS: i64 = 36_000;
const REFRESH_REUSE_SECONDS: i64 = 30;
const GRANT_TYPES: [&str; 2] = ["authorization_code", "refresh_token"];
const AUTH_METHODS: [&str; 4] = [
    "none",
    "client_secret_basic",
    "client_secret_post",
    "private_key_jwt",
];
/// Request limit of `/oauth2/register` per client IP (`mcp-client-registration`).
pub const MCP_REGISTRATION_LIMIT: u64 = 20;
/// The `MCP_CORS_HEADERS` of the auth route.
const CORS: [(&str, &str); 3] = [
    ("access-control-allow-origin", "*"),
    ("access-control-allow-methods", "GET,POST,OPTIONS"),
    (
        "access-control-allow-headers",
        "Content-Type, Authorization, mcp-protocol-version",
    ),
];

// ---------------------------------------------------------------- config

/// The router state: the auth state and the kv (rate-limit counter).
pub struct McpOAuth {
    pub auth: Arc<AuthState>,
    pub kv: Arc<chatbotx_kv::Kv>,
}

impl std::ops::Deref for McpOAuth {
    type Target = AuthState;

    fn deref(&self) -> &AuthState {
        &self.auth
    }
}

/// better-auth `baseURL` (`<BETTER_AUTH_URL>/api/auth`).
fn auth_base(state: &AuthState) -> String {
    format!("{}/api/auth", state.settings.base_url.trim_end_matches('/'))
}

/// `getMcpResourceUrl()`: `MCP_PUBLIC_URL`, else `<builder origin>/mcp`, no trailing `/`.
pub fn mcp_resource_url(state: &AuthState) -> String {
    std::env::var("MCP_PUBLIC_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| format!("{}/mcp", builder_origin(state)))
        .trim_end_matches('/')
        .to_string()
}

/// `defaultHasher`: SHA-256, base64url without padding.
pub fn store_hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

pub(super) fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn now_s(state: &AuthState) -> i64 {
    (state.now_ms)() / 1000
}

// ---------------------------------------------------------------- responses

fn with_cors(mut response: Response) -> Response {
    for (name, value) in CORS {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

fn json_with(
    status: StatusCode,
    body: &Value,
    headers: &[(&'static str, &'static str)],
) -> Response {
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response();
    for (name, value) in headers {
        response
            .headers_mut()
            .insert(*name, HeaderValue::from_static(value));
    }
    response
}

/// An `APIError` with an OAuth body (`{ error, error_description }`).
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    json_with(
        status,
        &json!({ "error": error, "error_description": description }),
        &[("cache-control", "no-store")],
    )
}

fn bad_request(error: &str, description: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, error, description)
}

fn server_error(cause: impl std::fmt::Display) -> Response {
    tracing::error!(cause = %cause, "MCP OAuth endpoint failed");
    oauth_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "internal server error",
    )
}

/// `formatErrorURL(url, error, description, state, iss)`.
fn error_url(
    url: &str,
    error: &str,
    description: &str,
    state: Option<&str>,
    iss: Option<&str>,
) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("error", error);
    params.append_pair("error_description", description);
    if let Some(state) = state.filter(|s| !s.is_empty()) {
        params.append_pair("state", state);
    }
    if let Some(iss) = iss {
        params.append_pair("iss", iss);
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{}", params.finish())
}

/// `handleRedirect`: JSON `{ redirect, url }` for a fetch / JSON caller, else 302.
fn handle_redirect(headers: &HeaderMap, url: &str) -> Response {
    let fetch = headers
        .get("sec-fetch-mode")
        .is_some_and(|mode| mode.as_bytes() == b"cors");
    let json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    if fetch || json {
        return json_with(
            StatusCode::OK,
            &json!({ "redirect": true, "url": url }),
            &[],
        );
    }
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(location) = HeaderValue::from_str(url) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

// ---------------------------------------------------------------- metadata

fn authorization_server_metadata(state: &AuthState) -> Value {
    let base = auth_base(state);
    json!({
        "scopes_supported": MCP_OAUTH_SCOPES,
        "issuer": builder_origin(state),
        "authorization_endpoint": format!("{base}/oauth2/authorize"),
        "token_endpoint": format!("{base}/oauth2/token"),
        "jwks_uri": format!("{base}/jwks"),
        "registration_endpoint": format!("{base}/oauth2/register"),
        "introspection_endpoint": format!("{base}/oauth2/introspect"),
        "revocation_endpoint": format!("{base}/oauth2/revoke"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": GRANT_TYPES,
        "token_endpoint_auth_methods_supported": AUTH_METHODS,
        "token_endpoint_auth_signing_alg_values_supported": PRIVATE_KEY_JWT_ALGS,
        "introspection_endpoint_auth_methods_supported": &AUTH_METHODS[1..],
        "introspection_endpoint_auth_signing_alg_values_supported": PRIVATE_KEY_JWT_ALGS,
        "revocation_endpoint_auth_methods_supported": &AUTH_METHODS[1..],
        "revocation_endpoint_auth_signing_alg_values_supported": PRIVATE_KEY_JWT_ALGS,
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true,
        "dpop_signing_alg_values_supported": ["EdDSA", "ES256", "ES512", "PS256", "RS256"],
        "backchannel_logout_supported": true,
        "backchannel_logout_session_supported": true,
        "claims_supported": [
            "sub", "iss", "aud", "exp", "iat", "sid", "scope", "azp",
            "name", "picture", "given_name", "family_name", "email", "email_verified",
        ],
        "claims_parameter_supported": true,
        "userinfo_endpoint": format!("{base}/oauth2/userinfo"),
        "subject_types_supported": ["public"],
        "acr_values_supported": ["0"],
        "id_token_signing_alg_values_supported": ["EdDSA"],
        "end_session_endpoint": format!("{base}/oauth2/end-session"),
        "request_parameter_supported": false,
        "request_uri_parameter_supported": false,
        "prompt_values_supported": ["login", "consent", "create", "select_account", "none"],
    })
}

const PRIVATE_KEY_JWT_ALGS: [&str; 10] = [
    "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384", "ES512", "EdDSA",
];

/// `getMcpAuthorizationServerMetadata(auth)` (both well-known paths).
async fn server_metadata(State(state): State<Arc<McpOAuth>>) -> Response {
    json_with(
        StatusCode::OK,
        &authorization_server_metadata(&state),
        &[
            ("access-control-allow-origin", "*"),
            ("cache-control", "no-store"),
        ],
    )
}

/// `/.well-known/oauth-protected-resource/[...path]` (builder route).
async fn protected_resource(State(state): State<Arc<McpOAuth>>, uri: Uri) -> Response {
    let resource_path = uri
        .path()
        .strip_prefix("/.well-known/oauth-protected-resource")
        .unwrap_or_default();
    let mcp = mcp_resource_url(&state);
    let Ok(mcp_url) = url::Url::parse(&mcp) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mcp_path = mcp_url.path().trim_end_matches('/');
    let same_origin = mcp_url.origin().ascii_serialization() == builder_origin(&state);
    if mcp_path.is_empty()
        || !same_origin
        || !(resource_path == mcp_path || resource_path.starts_with(&format!("{mcp_path}/")))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    json_with(
        StatusCode::OK,
        &json!({
            "resource": mcp,
            "authorization_servers": [state.settings.base_url],
            "bearer_methods_supported": ["header"],
        }),
        &[("cache-control", "no-store")],
    )
}

// ---------------------------------------------------------------- signed query

const SIGNED_EXTRA: [&str; 5] = ["sig", "exp", "ba_iat", "ba_pl", "ba_param"];

fn encode(params: &[(String, String)]) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        out.append_pair(key, value);
    }
    out.finish()
}

fn decode(query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// `canonicalizeOAuthQueryParams`: sorted by key, then value.
fn canonical(params: &[(String, String)]) -> String {
    let mut sorted = params.to_vec();
    sorted.sort();
    encode(&sorted)
}

/// `signParams`: the query plus `exp`, `ba_iat`, `ba_param` names and `sig`.
fn sign_params(state: &AuthState, query: &[(String, String)]) -> String {
    let mut params: Vec<(String, String)> = query
        .iter()
        .filter(|(k, _)| !SIGNED_EXTRA.contains(&k.as_str()))
        .cloned()
        .collect();
    params.push(("exp".into(), (now_s(state) + CODE_TTL_SECONDS).to_string()));
    params.push(("ba_iat".into(), (state.now_ms)().to_string()));
    let mut names: Vec<String> = params.iter().map(|(k, _)| k.clone()).collect();
    names.push("ba_param".into());
    names.sort();
    names.dedup();
    params.extend(names.into_iter().map(|n| ("ba_param".to_string(), n)));
    let sig = STANDARD.encode(hmac_sha256(
        &state.settings.secret,
        canonical(&params).as_bytes(),
    ));
    params.push(("sig".into(), sig));
    encode(&params)
}

/// `verifyOAuthQueryParams(query, secret)`: one `sig`, valid, `exp` not past.
pub fn verify_oauth_query_params(query: &str, secret: &str, now_ms: i64) -> bool {
    let params = decode(query);
    let sigs: Vec<&String> = params
        .iter()
        .filter(|(k, _)| k == "sig")
        .map(|(_, v)| v)
        .collect();
    let [sig] = sigs.as_slice() else {
        return false;
    };
    let exp: f64 = params
        .iter()
        .find(|(k, _)| k == "exp")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(f64::NAN);
    let rest: Vec<(String, String)> = params.iter().filter(|(k, _)| k != "sig").cloned().collect();
    let expected = STANDARD.encode(hmac_sha256(secret, canonical(&rest).as_bytes()));
    !sig.is_empty() && constant_time_eq(sig, &expected) && exp * 1000.0 >= now_ms as f64
}

/// The authorization query of a signed one (the signature fields dropped).
fn unsigned(params: &[(String, String)]) -> Vec<(String, String)> {
    params
        .iter()
        .filter(|(k, _)| !SIGNED_EXTRA.contains(&k.as_str()))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- rows

/// The `OAuthClient` fields this server reads.
#[derive(Debug, Clone)]
struct Client {
    client_id: String,
    client_secret: Option<String>,
    disabled: bool,
    skip_consent: bool,
    scopes: Option<Vec<String>>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
    require_pkce: Option<bool>,
}

fn strings(value: Option<String>) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(&value?).ok()?;
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

fn find_client(
    conn: &Connection,
    client_id: &str,
) -> chatbotx_db::rusqlite::Result<Option<Client>> {
    conn.query_row(
        r#"SELECT "clientId", "clientSecret", "disabled", "skipConsent", "scopes", "redirectUris",
                  "tokenEndpointAuthMethod", "grantTypes", "requirePKCE"
           FROM "OAuthClient" WHERE "clientId" = ?1"#,
        [client_id],
        |r| {
            Ok(Client {
                client_id: r.get(0)?,
                client_secret: r.get(1)?,
                disabled: r.get::<_, Option<bool>>(2)?.unwrap_or(false),
                skip_consent: r.get::<_, Option<bool>>(3)?.unwrap_or(false),
                scopes: strings(r.get(4)?),
                redirect_uris: strings(r.get(5)?).unwrap_or_default(),
                token_endpoint_auth_method: r.get(6)?,
                grant_types: strings(r.get(7)?),
                require_pkce: r.get(8)?,
            })
        },
    )
    .optional()
}

/// `clientAllowsGrant`.
fn allows_grant(client: &Client, grant: &str) -> bool {
    let grants = client
        .grant_types
        .clone()
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| vec!["authorization_code".into()]);
    (grant == "refresh_token" && grants.iter().any(|g| g == "authorization_code"))
        || grants.iter().any(|g| g == grant)
}

/// `isPKCERequired`: the reason, or `None`.
fn pkce_required(client: &Client, scopes: &[String], nonce: Option<&str>) -> Option<&'static str> {
    if client.token_endpoint_auth_method.as_deref() == Some("none") {
        return Some("pkce is required for public clients");
    }
    let oidc_nonce = scopes.iter().any(|s| s == "openid") && nonce.is_some_and(|n| !n.is_empty());
    if scopes.iter().any(|s| s == "offline_access") && !oidc_nonce {
        return Some("pkce or OIDC nonce is required when requesting offline_access scope");
    }
    client
        .require_pkce
        .unwrap_or(true)
        .then_some("pkce is required for this client")
}

/// A browser session (`Session` row of the signed cookie).
#[derive(Debug, Clone)]
struct BrowserSession {
    id: i64,
    user_id: i64,
    created_at: i64,
}

async fn browser_session(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<Option<BrowserSession>, Response> {
    let headers = apply_bearer(headers, &state.settings.secret);
    let Some(token) = request_cookie(&headers, super::SESSION_COOKIE)
        .and_then(|raw| verify_signed_cookie(&raw, &state.settings.secret))
    else {
        return Ok(None);
    };
    let now = (state.now_ms)();
    state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "id", "userId", "createdAt" FROM "Session" WHERE "token" = ?1 AND "expiresAt" > ?2"#,
                    params![token, now],
                    |r| {
                        Ok(BrowserSession {
                            id: r.get(0)?,
                            user_id: r.get(1)?,
                            created_at: r.get(2)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
        .map_err(server_error)
}

// ---------------------------------------------------------------- redirect URIs

fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host == "[::1]"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `stripLoopbackRedirectPort`: an http loopback URI without its port.
fn strip_loopback_port(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "http" || !is_loopback_host(host) {
        return None;
    }
    let start = uri.find("://")? + 3;
    let end = uri[start..]
        .find(['/', '?', '#'])
        .map_or(uri.len(), |i| start + i);
    let authority = &uri[start..end];
    let host_part = match authority.rfind(':') {
        Some(i) if !authority[i..].contains(']') => &authority[..i],
        _ => authority,
    };
    Some(format!("{}{}{}", &uri[..start], host_part, &uri[end..]))
}

/// `findRegisteredRedirectUri`: exact match, or the same loopback URI on another port.
fn registered_redirect(registered: &[String], requested: &str) -> bool {
    let Ok(url) = url::Url::parse(requested) else {
        return false;
    };
    if requested.contains('#') || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let stripped = strip_loopback_port(requested);
    registered
        .iter()
        .any(|uri| uri == requested || stripped.is_some() && strip_loopback_port(uri) == stripped)
}

/// `isExactLoopbackHttpRedirectUri`.
fn is_exact_loopback_http(uri: &str) -> bool {
    let Some(rest) = uri
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("http://"))
        .map(|_| &uri[7..])
    else {
        return false;
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host_port = &authority[authority.rfind('@').map_or(0, |i| i + 1)..];
    let host = if host_port.starts_with('[') {
        host_port.find(']').map_or("", |i| &host_port[..=i])
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]"
    )
}

fn is_non_http(uri: &str) -> bool {
    url::Url::parse(uri).is_ok_and(|u| u.scheme() != "http" && u.scheme() != "https")
}

const RESERVED_SCHEMES: [&str; 9] = [
    "javascript",
    "data",
    "vbscript",
    "file",
    "blob",
    "about",
    "ftp",
    "ws",
    "wss",
];

/// `validateClientRedirectUri(uri, applicationType)`; `Err(description)`.
fn validate_redirect_uri(uri: &str, application_type: &str) -> Result<(), String> {
    let url =
        url::Url::parse(uri).map_err(|_| format!("redirect URI must be an absolute URI: {uri}"))?;
    if uri.contains('#') || !url.username().is_empty() || url.password().is_some() {
        return Err(format!(
            "redirect URI must not include credentials or a fragment: {uri}"
        ));
    }
    let host = url.host_str().unwrap_or_default();
    let loopback = is_loopback_host(host);
    match (application_type, url.scheme()) {
        ("web", "https") if !loopback => Ok(()),
        ("web", _) => Err(format!(
            "web clients require https redirect URIs on non-loopback hosts: {uri}"
        )),
        (_, "https") if loopback => Err(format!(
            "native clients must not use https loopback redirect URIs: {uri}"
        )),
        (_, "https") => Ok(()),
        (_, "http") if is_exact_loopback_http(uri) => Ok(()),
        (_, "http") => Err(format!(
            "native clients may use http only on the exact loopback hosts localhost, 127.0.0.1, or [::1]: {uri}"
        )),
        (_, scheme) => {
            let reverse_domain = scheme.contains('.') && host.is_empty();
            let host_bearing = !host.is_empty() && url.path().len() > 1;
            if !RESERVED_SCHEMES.contains(&scheme) && (reverse_domain || host_bearing) {
                Ok(())
            } else {
                Err(format!(
                    "native private-use redirect URI schemes must not use a reserved scheme; they must be an authority-free reverse-domain URI or a custom-scheme URI with a host and a path: {uri}"
                ))
            }
        }
    }
}

// ---------------------------------------------------------------- register

fn str_array(body: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, Response> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(Some)
            .ok_or_else(|| {
                bad_request(
                    "invalid_client_metadata",
                    &format!("{key} must be an array of strings"),
                )
            }),
        Some(_) => Err(bad_request(
            "invalid_client_metadata",
            &format!("{key} must be an array of strings"),
        )),
    }
}

fn opt_str(body: &Map<String, Value>, key: &str) -> Result<Option<String>, Response> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(bad_request(
            "invalid_client_metadata",
            &format!("{key} must be a string"),
        )),
    }
}

/// `registerEndpoint` (dynamic, `allowUnauthenticatedClientRegistration`).
async fn register_inner(
    state: &McpOAuth,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Response, Response> {
    let client_ip = crate::auth::guest_client_ip(headers);
    if let Err(limited) = assert_api_not_rate_limited(
        ApiRateLimitInput {
            scope: "mcp-client-registration",
            key: &client_ip,
            store: &state.kv.mem,
            now_ms: (state.now_ms)(),
            limit: Some(MCP_REGISTRATION_LIMIT),
        },
        None,
    ) {
        let mut response = json_with(
            StatusCode::TOO_MANY_REQUESTS,
            &json!({ "error": limited.to_string() }),
            &[],
        );
        if let Ok(value) = HeaderValue::from_str(&limited.retry_after.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return Err(response);
    }
    let body: Map<String, Value> = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => map,
        _ => return Err(bad_request("invalid_request", "body must be a JSON object")),
    };
    let session = browser_session(state, headers).await?;
    let requested_grants = str_array(&body, "grant_types")?;
    let grant_types = match &requested_grants {
        Some(grants) if !grants.is_empty() => {
            let mutual: Vec<String> = grants
                .iter()
                .filter(|g| GRANT_TYPES.contains(&g.as_str()))
                .cloned()
                .collect();
            if mutual.is_empty() {
                return Err(bad_request(
                    "invalid_client_metadata",
                    &format!(
                        "no mutually supported grant_type among {}",
                        grants.join(", ")
                    ),
                ));
            }
            mutual
        }
        Some(_) => {
            return Err(bad_request(
                "invalid_client_metadata",
                "grant_types must contain at least one grant type",
            ));
        }
        None => vec!["authorization_code".to_string()],
    };
    let dropped_code = requested_grants
        .as_ref()
        .is_some_and(|g| g.iter().any(|g| g == "authorization_code"))
        && !grant_types.iter().any(|g| g == "authorization_code");
    let response_types = if dropped_code {
        None
    } else {
        str_array(&body, "response_types")?
    }
    .or_else(|| {
        grant_types
            .iter()
            .any(|g| g == "authorization_code")
            .then(|| vec!["code".to_string()])
    });
    let redirect_uris = str_array(&body, "redirect_uris")?.unwrap_or_default();
    let auth_method = opt_str(&body, "token_endpoint_auth_method")?
        .unwrap_or_else(|| "client_secret_basic".to_string());
    if !["none", "client_secret_basic", "client_secret_post"].contains(&auth_method.as_str()) {
        return Err(bad_request(
            "invalid_client_metadata",
            &format!("unsupported token_endpoint_auth_method {auth_method}"),
        ));
    }
    let application_type = match opt_str(&body, "application_type")? {
        Some(kind) if kind == "web" || kind == "native" => kind,
        Some(_) => {
            return Err(bad_request(
                "invalid_client_metadata",
                "application_type must be web or native",
            ));
        }
        // `clientRegistrationDefaultApplicationType: "infer"`.
        None if redirect_uris
            .iter()
            .any(|u| is_non_http(u) || is_exact_loopback_http(u)) =>
        {
            "native".to_string()
        }
        None => "web".to_string(),
    };
    let has_code = grant_types.iter().any(|g| g == "authorization_code");
    if has_code && redirect_uris.is_empty() {
        return Err(bad_request(
            "invalid_redirect_uri",
            "Redirect URIs are required for authorization_code and implicit grant types",
        ));
    }
    for uri in &redirect_uris {
        validate_redirect_uri(uri, &application_type)
            .map_err(|d| bad_request("invalid_redirect_uri", &d))?;
    }
    let code_response = response_types
        .as_ref()
        .is_some_and(|r| r.iter().any(|t| t == "code"));
    if has_code && !code_response {
        return Err(bad_request(
            "invalid_client_metadata",
            "When 'authorization_code' grant type is used, 'code' response type must be included",
        ));
    }
    if !has_code && code_response {
        return Err(bad_request(
            "invalid_client_metadata",
            "When 'code' response type is used, 'authorization_code' grant type must be included",
        ));
    }
    for scope in opt_str(&body, "scope")?
        .unwrap_or_default()
        .split(' ')
        .filter(|s| !s.is_empty())
    {
        if !MCP_OAUTH_SCOPES.contains(&scope) {
            return Err(bad_request(
                "invalid_scope",
                &format!("cannot request scope {scope}"),
            ));
        }
    }
    if body.get("require_pkce") == Some(&Value::Bool(false)) {
        return Err(bad_request(
            "invalid_client_metadata",
            "pkce is required for registered clients.",
        ));
    }
    let resource = mcp_resource_url(state);
    for requested in str_array(&body, "resources")?.unwrap_or_default() {
        if !requested.is_empty() && requested != resource {
            return Err(bad_request(
                "invalid_target",
                &format!("requested resource {requested} is not allowed for client registration"),
            ));
        }
    }
    let client_id = random_letters(32);
    let secret = (auth_method != "none").then(|| random_letters(32));
    let iat = now_s(state);
    let names = [
        ("client_name", "name"),
        ("client_uri", "uri"),
        ("logo_uri", "icon"),
        ("tos_uri", "tos"),
        ("policy_uri", "policy"),
        ("software_id", "softwareId"),
        ("software_version", "softwareVersion"),
        ("software_statement", "softwareStatement"),
    ];
    let mut text_fields = Vec::new();
    for (key, column) in names {
        text_fields.push((key, column, opt_str(&body, key)?));
    }
    let contacts = str_array(&body, "contacts")?;
    let user_id = session.as_ref().map(|s| s.user_id);
    let row = (
        client_id.clone(),
        secret.as_deref().map(store_hash),
        redirect_uris.clone(),
        auth_method.clone(),
        grant_types.clone(),
        response_types.clone(),
        application_type.clone(),
        text_fields.clone(),
        contacts.clone(),
    );
    state
        .db
        .write(move |tx| {
            let (client_id, secret_hash, redirects, method, grants, responses, app_type, texts, contacts) = row;
            let now = iat * 1000;
            tx.execute(
                r#"INSERT INTO "OAuthClient" ("id", "createdAt", "updatedAt", "clientId", "clientSecret",
                       "disabled", "scopes", "clientCredentialsScopes", "userId", "redirectUris",
                       "tokenEndpointAuthMethod", "grantTypes", "responseTypes", "applicationType",
                       "contacts", "dpopBoundAccessTokens")
                   VALUES (?1, ?2, ?2, ?3, ?4, 0, ?5, '[]', ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)"#,
                params![
                    new_id(),
                    now,
                    client_id,
                    secret_hash,
                    json!(MCP_OAUTH_SCOPES).to_string(),
                    user_id,
                    json!(redirects).to_string(),
                    method,
                    json!(grants).to_string(),
                    responses.map(|r| json!(r).to_string()),
                    app_type,
                    contacts.map(|c| json!(c).to_string()),
                ],
            )?;
            for (_, column, value) in texts {
                if let Some(value) = value {
                    tx.execute(
                        &format!(r#"UPDATE "OAuthClient" SET "{column}" = ?1 WHERE "clientId" = ?2"#),
                        params![value, client_id],
                    )?;
                }
            }
            Ok(())
        })
        .await
        .map_err(server_error)?;
    // `schemaToOAuth` of the stored row (undefined fields left out).
    let mut out = Map::new();
    out.insert("client_id".into(), json!(client_id));
    if let Some(secret) = &secret {
        out.insert("client_secret".into(), json!(secret));
        out.insert("client_secret_expires_at".into(), json!(0));
    }
    out.insert("scope".into(), json!(MCP_OAUTH_SCOPES.join(" ")));
    if let Some(user_id) = user_id {
        out.insert("user_id".into(), json!(user_id.to_string()));
    }
    out.insert("client_id_issued_at".into(), json!(iat));
    for (key, _, value) in text_fields {
        if let Some(value) = value {
            out.insert(key.into(), json!(value));
        }
    }
    if let Some(contacts) = contacts {
        out.insert("contacts".into(), json!(contacts));
    }
    out.insert("redirect_uris".into(), json!(redirect_uris));
    out.insert("token_endpoint_auth_method".into(), json!(auth_method));
    out.insert("grant_types".into(), json!(grant_types));
    if let Some(response_types) = response_types {
        out.insert("response_types".into(), json!(response_types));
    }
    out.insert("application_type".into(), json!(application_type));
    out.insert("disabled".into(), json!(false));
    out.insert("dpop_bound_access_tokens".into(), json!(false));
    out.insert("resources".into(), json!([resource]));
    Ok(json_with(
        StatusCode::CREATED,
        &Value::Object(out),
        &[("cache-control", "no-store"), ("pragma", "no-cache")],
    ))
}

/// `generateRandomString(32, "a-z", "A-Z")`.
fn random_letters(length: usize) -> String {
    random_string(length * 2)
        .chars()
        .filter(char::is_ascii_alphabetic)
        .chain(std::iter::repeat_with(|| {
            random_letters(1).chars().next().unwrap_or('a')
        }))
        .take(length)
        .collect()
}

async fn register(State(state): State<Arc<McpOAuth>>, headers: HeaderMap, body: Bytes) -> Response {
    with_cors(
        register_inner(&state, &headers, &body)
            .await
            .unwrap_or_else(|e| e),
    )
}

// ---------------------------------------------------------------- authorize

/// One authorization request (the parsed query).
struct AuthorizeQuery {
    params: Vec<(String, String)>,
}

impl AuthorizeQuery {
    fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn count(&self, key: &str) -> usize {
        self.params.iter().filter(|(k, _)| k == key).count()
    }

    fn set(&mut self, key: &str, value: &str) {
        self.params.retain(|(k, _)| k != key);
        self.params.push((key.to_string(), value.to_string()));
    }
}

/// `parsePrompt`.
fn prompts(query: &AuthorizeQuery) -> Vec<&str> {
    query
        .get("prompt")
        .unwrap_or_default()
        .split(' ')
        .map(str::trim)
        .filter(|p| ["login", "consent", "create", "select_account", "none"].contains(p))
        .collect()
}

/// `authorizeEndpoint` for `query`; the response (redirect or `{ redirect, url }`).
async fn authorize_flow(
    state: &AuthState,
    headers: &HeaderMap,
    mut query: AuthorizeQuery,
    session: Option<BrowserSession>,
) -> Response {
    let base = auth_base(state);
    let iss = builder_origin(state);
    let error_page = |error: &str, description: &str| {
        handle_redirect(
            headers,
            &error_url(&format!("{base}/error"), error, description, None, None),
        )
    };
    // `defaultMcpResource`: fill in / canonicalize `resource`.
    let resource = mcp_resource_url(state);
    let mut as_map = Map::new();
    if let Some(r) = query.get("resource") {
        as_map.insert("resource".into(), json!(r));
    }
    if query.count("resource") <= 1
        && let Some(McpResourceRewrite::Query(rewritten)) =
            default_mcp_resource("/oauth2/authorize", Some(&as_map), None, &resource)
        && let Some(r) = rewritten.get("resource").and_then(Value::as_str)
    {
        query.set("resource", r);
    }
    for key in [
        "client_id",
        "redirect_uri",
        "response_type",
        "state",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "nonce",
        "prompt",
    ] {
        if query.count(key) > 1 {
            return error_page("invalid_request", &format!("{key} must be provided once"));
        }
    }
    if query.get("request").is_some() {
        return error_page("request_not_supported", "request object not supported");
    }
    if query.get("request_uri").is_some() {
        return error_page("request_uri_not_supported", "request_uri not supported");
    }
    let Some(response_type) = query.get("response_type").map(str::to_string) else {
        return error_page("invalid_request", "response_type is required");
    };
    let prompt = prompts(&query)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let has = |p: &str| prompt.iter().any(|x| x == p);
    if has("select_account") {
        return error_page(
            "unsupported_prompt_select_account",
            "unsupported prompt type",
        );
    }
    if response_type != "code" {
        return error_page("unsupported_response_type", "unsupported response type");
    }
    let client_id = query.get("client_id").unwrap_or_default().to_string();
    let lookup = client_id.clone();
    let client = match state.db.read(move |c| Ok(find_client(c, &lookup)?)).await {
        Ok(Some(client)) => client,
        Ok(None) => return error_page("invalid_client", "client not found"),
        Err(e) => return server_error(e),
    };
    if client.disabled {
        return error_page("client_disabled", "client is disabled");
    }
    if !allows_grant(&client, "authorization_code") {
        return error_page(
            "unauthorized_client",
            "client is not authorized to use the authorization_code grant",
        );
    }
    let redirect_uri = query.get("redirect_uri").unwrap_or_default().to_string();
    if redirect_uri.is_empty() || !registered_redirect(&client.redirect_uris, &redirect_uri) {
        return error_page("invalid_redirect", "invalid redirect uri");
    }
    let rp_state = query.get("state").map(str::to_string);
    let to_rp = |error: &str, description: &str| {
        handle_redirect(
            headers,
            &error_url(
                &redirect_uri,
                error,
                description,
                rp_state.as_deref(),
                Some(&iss),
            ),
        )
    };
    let allowed = client
        .scopes
        .clone()
        .unwrap_or_else(|| MCP_OAUTH_SCOPES.iter().map(|s| s.to_string()).collect());
    let mut scopes: Vec<String> = query
        .get("scope")
        .map(|s| {
            s.split(' ')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if query.get("scope").is_some() {
        let invalid: Vec<&str> = scopes
            .iter()
            .filter(|s| !allowed.contains(s))
            .map(String::as_str)
            .collect();
        if !invalid.is_empty() {
            return to_rp(
                "invalid_scope",
                &format!("The following scopes are invalid: {}", invalid.join(", ")),
            );
        }
    } else {
        scopes = allowed.clone();
        query.set("scope", &scopes.join(" "));
    }
    if query.get("claims").is_some() {
        // ponytail: the OIDC `claims` parameter is not supported.
        return to_rp(
            "invalid_request",
            "claims must be a valid Claims request object",
        );
    }
    if query.get("max_age").is_some() {
        // ponytail: `max_age` is not supported; ask for a fresh login instead.
        return to_rp("invalid_request", "max_age is not supported");
    }
    let resources: Vec<String> = query
        .params
        .iter()
        .filter(|(k, _)| k == "resource")
        .map(|(_, v)| v.clone())
        .collect();
    if resources.iter().any(|r| r != &resource) {
        return to_rp("invalid_target", "requested resource invalid");
    }
    if let Some(reason) = pkce_required(&client, &scopes, query.get("nonce"))
        && (query.get("code_challenge").is_none() || query.get("code_challenge_method").is_none())
    {
        return to_rp("invalid_request", reason);
    }
    if query.get("code_challenge").is_some() || query.get("code_challenge_method").is_some() {
        if query.get("code_challenge").is_none() || query.get("code_challenge_method").is_none() {
            return to_rp(
                "invalid_request",
                "code_challenge and code_challenge_method must both be provided",
            );
        }
        if query.get("code_challenge_method") != Some("S256") {
            return to_rp(
                "invalid_request",
                "invalid code_challenge method, only S256 is supported",
            );
        }
    }
    let Some(session) = session.filter(|_| !has("login") && !has("create")) else {
        if has("none") {
            return to_rp("login_required", "authentication required");
        }
        return handle_redirect(
            headers,
            &format!("/oauth/sign-in?{}", sign_params(state, &query.params)),
        );
    };
    let consent_page = || {
        handle_redirect(
            headers,
            &format!("/oauth/consent?{}", sign_params(state, &query.params)),
        )
    };
    if has("consent") {
        return consent_page();
    }
    if !client.skip_consent {
        let (user, cid) = (session.user_id, client.client_id.clone());
        let consent = state
            .db
            .read(move |c| {
                Ok(c.query_row(
                    r#"SELECT "scopes", "resources" FROM "OAuthConsent"
                       WHERE "clientId" = ?1 AND "userId" = ?2 AND ("referenceId" IS NULL OR "referenceId" = '')
                       ORDER BY "id" LIMIT 1"#,
                    params![cid, user],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()?)
            })
            .await;
        let consent = match consent {
            Ok(consent) => consent,
            Err(e) => return server_error(e),
        };
        let covered = consent.is_some_and(|(granted, granted_resources)| {
            let granted = strings(granted).unwrap_or_default();
            let granted_resources = strings(granted_resources).unwrap_or_default();
            scopes.iter().all(|s| granted.contains(s))
                && resources.iter().all(|r| granted_resources.contains(r))
        });
        if !covered {
            if has("none") {
                return to_rp("consent_required", "End-User consent is required");
            }
            return consent_page();
        }
    }
    // `redirectWithAuthorizationCode`.
    let code = random_string(32);
    let iat = now_s(state);
    let value = json!({
        "type": "authorization_code",
        "query": query.params.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
        "userId": session.user_id.to_string(),
        "sessionId": session.id.to_string(),
        "authTime": session.created_at,
        "resource": resources,
    })
    .to_string();
    let identifier = store_hash(&code);
    let stored = state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Verification" ("id", "identifier", "value", "expiresAt", "createdAt", "updatedAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?5)"#,
                params![new_id(), identifier, value, (iat + CODE_TTL_SECONDS) * 1000, iat * 1000],
            )?;
            Ok(())
        })
        .await;
    if let Err(e) = stored {
        return server_error(e);
    }
    let mut target = url::Url::parse(&redirect_uri).expect("registered redirect URIs parse");
    target.query_pairs_mut().append_pair("code", &code);
    if let Some(rp_state) = &rp_state {
        target.query_pairs_mut().append_pair("state", rp_state);
    }
    target.query_pairs_mut().append_pair("iss", &iss);
    handle_redirect(headers, target.as_str())
}

async fn authorize(State(state): State<Arc<McpOAuth>>, uri: Uri, headers: HeaderMap) -> Response {
    let params = unsigned(&decode(uri.query().unwrap_or_default()));
    let session = match browser_session(&state, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    authorize_flow(&state, &headers, AuthorizeQuery { params }, session).await
}

/// `GET /oauth/sign-in`: a signed-in user goes back to authorize, else to the sign-in page.
async fn oauth_sign_in(
    State(state): State<Arc<McpOAuth>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let query = uri.query().unwrap_or_default();
    let params = decode(query);
    let fresh = params.iter().any(|(k, v)| {
        k == "max_age" || k == "prompt" && v.split(' ').any(|p| p == "login" || p == "create")
    });
    let session = browser_session(&state, &headers).await.ok().flatten();
    let target = if session.is_none() || fresh {
        format!("/auth/sign-in?{query}")
    } else {
        format!("/api/auth/oauth2/authorize?{query}")
    };
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(location) = HeaderValue::from_str(&target) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

// ---------------------------------------------------------------- consent

/// A failed consent action (`ChatbotXException` code, status, message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

fn consent_error(status: u16, code: &str, message: &str) -> ConsentError {
    ConsentError {
        status,
        code: code.to_string(),
        message: message.to_string(),
    }
}

/// `verifyMcpOAuthQuery`: `(client_id, scopes)` of a valid signed query.
fn verified_query(state: &AuthState, query: &str) -> Option<(String, Vec<String>)> {
    if !verify_oauth_query_params(query, &state.settings.secret, (state.now_ms)()) {
        return None;
    }
    let params = decode(query);
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let client_id = get("client_id").filter(|id| !id.is_empty())?;
    let scopes = get("scope")
        .map(|s| {
            s.split(' ')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some((client_id, scopes))
}

async fn session_user(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<BrowserSession, ConsentError> {
    browser_session(state, headers)
        .await
        .map_err(|_| consent_error(500, "INTERNAL_SERVER_ERROR", "Internal server error"))?
        .ok_or_else(|| consent_error(401, "unauthorized", "Not authenticated"))
}

/// The consent page data (`app/oauth/consent/page.tsx`): `notFound` (404)
/// for an unsigned / expired query.
pub async fn oauth_consent_page(
    state: &AuthState,
    headers: &HeaderMap,
    input: &Value,
) -> Result<Value, ConsentError> {
    let session = session_user(state, headers).await?;
    let query = input
        .get("oauthQuery")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let not_found = || consent_error(404, "notFound", "Not found");
    let (client_id, scopes) = verified_query(state, query).ok_or_else(not_found)?;
    let workspaces =
        chatbotx_business::mcp_oauth::list_authorizable_workspaces(&state.db, session.user_id)
            .await
            .map_err(|e| consent_error(500, "INTERNAL_SERVER_ERROR", &e.message()))?;
    let (lookup, user) = (client_id.clone(), session.user_id);
    let (client_name, email) = state
        .db
        .read(move |c| {
            let name: Option<String> = c
                .query_row(
                    r#"SELECT "name" FROM "OAuthClient" WHERE "clientId" = ?1"#,
                    [&lookup],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let email: Option<String> = c
                .query_row(
                    r#"SELECT "email" FROM "User" WHERE "id" = ?1"#,
                    [user],
                    |r| r.get(0),
                )
                .optional()?;
            Ok((name, email))
        })
        .await
        .map_err(|e| consent_error(500, "INTERNAL_SERVER_ERROR", &e.to_string()))?;
    Ok(json!({
        "clientName": client_name.filter(|n| !n.is_empty()).unwrap_or(client_id),
        "oauthQuery": query,
        "scopes": scopes,
        "userEmail": email,
        "workspaces": workspaces.into_iter().map(|(id, name)| json!({ "id": id.to_string(), "name": name })).collect::<Vec<_>>(),
    }))
}

/// `submitOAuthConsentAction` (`{ accept, oauthQuery, workspaceId? }`) +
/// better-auth `oauth2Consent`: `{ redirect: true, url }`.
pub async fn submit_oauth_consent(
    state: &AuthState,
    headers: &HeaderMap,
    input: &Value,
) -> Result<Value, ConsentError> {
    let invalid = || consent_error(400, "BAD_REQUEST", "Input validation failed");
    let accept = input
        .get("accept")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let query = input
        .get("oauthQuery")
        .and_then(Value::as_str)
        .filter(|q| !q.is_empty())
        .ok_or_else(invalid)?;
    let workspace_id = if accept {
        let text = input
            .get("workspaceId")
            .and_then(Value::as_str)
            .filter(|w| !w.is_empty())
            .ok_or_else(invalid)?;
        // A non-numeric id is no workspace the user administers.
        Some(text.parse::<i64>().unwrap_or(-1))
    } else {
        None
    };
    let session = session_user(state, headers).await?;
    let mut json_headers = headers.clone();
    json_headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    let redirect_of = |response: Response| async move {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap_or_default();
        serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null)
    };
    let Some(workspace_id) = workspace_id else {
        // Deny: the better-auth hook still checks the signature.
        if !verify_oauth_query_params(query, &state.settings.secret, (state.now_ms)()) {
            return Err(consent_error(400, "BAD_REQUEST", "invalid_signature"));
        }
        let params = decode(query);
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        let url = error_url(
            &get("redirect_uri").unwrap_or_default(),
            "access_denied",
            "User denied access",
            get("state").as_deref(),
            Some(&builder_origin(state)),
        );
        return Ok(json!({ "redirect": true, "url": url }));
    };
    let (client_id, _) = verified_query(state, query).ok_or_else(|| {
        consent_error(
            403,
            "forbidden",
            "OAuth consent request is invalid or expired",
        )
    })?;
    let business = |e: chatbotx_business::BusinessError| match e {
        chatbotx_business::BusinessError::Exception {
            message,
            code,
            status,
            ..
        } => ConsentError {
            status,
            code,
            message,
        },
        other => consent_error(500, "INTERNAL_SERVER_ERROR", &other.message()),
    };
    chatbotx_business::mcp_oauth::assert_authorizable_workspace(
        &state.db,
        session.user_id,
        workspace_id,
    )
    .await
    .map_err(business)?;
    // `consentEndpoint`: store the consent of the originally requested scopes.
    let params = unsigned(&decode(query));
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let scopes: Vec<String> = get("scope")
        .map(|s| s.split(' ').map(str::to_string).collect())
        .unwrap_or_default();
    let resources: Vec<String> = params
        .iter()
        .filter(|(k, _)| k == "resource")
        .map(|(_, v)| v.clone())
        .collect();
    let (user, cid, now) = (session.user_id, client_id.clone(), (state.now_ms)());
    state
        .db
        .write(move |tx| {
            let found: Option<i64> = tx
                .query_row(
                    r#"SELECT "id" FROM "OAuthConsent" WHERE "clientId" = ?1 AND "userId" = ?2
                       AND ("referenceId" IS NULL OR "referenceId" = '') ORDER BY "id" LIMIT 1"#,
                    params![cid, user],
                    |r| r.get(0),
                )
                .optional()?;
            let resources = (!resources.is_empty()).then(|| json!(resources).to_string());
            match found {
                Some(id) => tx.execute(
                    r#"UPDATE "OAuthConsent" SET "scopes" = ?1, "resources" = ?2, "updatedAt" = ?3 WHERE "id" = ?4"#,
                    params![json!(scopes).to_string(), resources, now, id],
                )?,
                None => tx.execute(
                    r#"INSERT INTO "OAuthConsent" ("id", "createdAt", "updatedAt", "clientId", "userId", "scopes", "resources")
                       VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)"#,
                    params![new_id(), now, cid, user, json!(scopes).to_string(), resources],
                )?,
            };
            Ok(())
        })
        .await
        .map_err(|e| consent_error(500, "INTERNAL_SERVER_ERROR", &e.to_string()))?;
    let mut params = params.clone();
    // `removePromptFromQuery(query, "consent")`.
    if let Some((_, prompt)) = params.iter_mut().find(|(k, _)| k == "prompt") {
        *prompt = prompt
            .split(' ')
            .filter(|p| *p != "consent")
            .collect::<Vec<_>>()
            .join(" ");
    }
    params.retain(|(k, v)| k != "prompt" || !v.is_empty());
    let response = authorize_flow(
        state,
        &json_headers,
        AuthorizeQuery { params },
        Some(session.clone()),
    )
    .await;
    let result = redirect_of(response).await;
    if let Err(error) = chatbotx_business::mcp_oauth::bind_consent_workspace(
        &state.db,
        session.user_id,
        &client_id,
        workspace_id,
    )
    .await
    {
        tracing::error!(
            client_id,
            user_id = session.user_id,
            "Failed to bind MCP OAuth consent to workspace; revoking consent"
        );
        let _ = chatbotx_business::mcp_oauth::revoke_grant(&state.db, session.user_id, &client_id)
            .await;
        return Err(business(error));
    }
    Ok(result)
}

// ---------------------------------------------------------------- token

/// The token request body: form fields (or a JSON object); repeated `resource`.
fn token_body(headers: &HeaderMap, body: &Bytes) -> Vec<(String, String)> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    if is_json {
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (key, value) in map {
            match value {
                Value::String(s) => out.push((key, s)),
                Value::Array(items) => out.extend(
                    items
                        .into_iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .map(|v| (key.clone(), v)),
                ),
                _ => {}
            }
        }
        return out;
    }
    decode(std::str::from_utf8(body).unwrap_or_default())
}

struct TokenRequest {
    body: Vec<(String, String)>,
    basic: Option<(String, String)>,
}

impl TokenRequest {
    fn get(&self, key: &str) -> Option<&str> {
        self.body
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn all(&self, key: &str) -> Vec<String> {
        self.body
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// `extractClientCredentials` + `validateClientCredentials`.
async fn client_of(state: &AuthState, req: &TokenRequest, grant: &str) -> Result<Client, Response> {
    let (client_id, secret, method) = match &req.basic {
        Some((id, secret)) => (id.clone(), Some(secret.clone()), "client_secret_basic"),
        None => match (req.get("client_id"), req.get("client_secret")) {
            (Some(id), Some(secret)) => (
                id.to_string(),
                Some(secret.to_string()),
                "client_secret_post",
            ),
            (Some(id), None) => (id.to_string(), None, "none"),
            (None, _) => return Err(bad_request("invalid_request", "client_id is required")),
        },
    };
    let invalid = |d: &str| {
        let status = if method == "client_secret_basic" {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::BAD_REQUEST
        };
        let mut response = oauth_error(status, "invalid_client", d);
        if method == "client_secret_basic" {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic"));
        }
        response
    };
    let lookup = client_id.clone();
    let client = state
        .db
        .read(move |c| Ok(find_client(c, &lookup)?))
        .await
        .map_err(server_error)?
        .ok_or_else(|| invalid("client not found"))?;
    if client.disabled {
        return Err(invalid("client is disabled"));
    }
    let registered = client
        .token_endpoint_auth_method
        .clone()
        .unwrap_or_else(|| "client_secret_basic".into());
    if registered != method {
        return Err(invalid(&format!(
            "client registered for {registered} cannot use {method}"
        )));
    }
    if registered != "none" && secret.is_none() {
        return Err(invalid("client secret must be provided"));
    }
    if let Some(secret) = &secret {
        let Some(stored) = &client.client_secret else {
            return Err(invalid(
                "public client, client secret should not be received",
            ));
        };
        if !constant_time_eq(stored, &store_hash(secret)) {
            return Err(invalid("invalid client_secret"));
        }
    }
    if !allows_grant(&client, grant) {
        return Err(bad_request(
            "unauthorized_client",
            &format!("client is not authorized to use grant type {grant}"),
        ));
    }
    Ok(client)
}

/// `validateClientScopes`.
fn check_client_scopes(client: &Client, scopes: &[String]) -> Result<(), Response> {
    if let Some(allowed) = &client.scopes
        && let Some(scope) = scopes.iter().find(|s| !allowed.contains(s))
    {
        return Err(bad_request(
            "invalid_scope",
            &format!("client does not allow scope {scope}"),
        ));
    }
    Ok(())
}

/// What `createUserTokens` needs.
struct Issue {
    user_id: i64,
    session_id: Option<i64>,
    auth_time: Option<i64>,
    scopes: Vec<String>,
    resources: Vec<String>,
    nonce: Option<String>,
    authorization_code_id: Option<String>,
    rotate: Option<i64>,
}

/// `computeOidcHash` for EdDSA (SHA-512, left half).
fn at_hash(token: &str) -> String {
    let digest = Sha512::digest(token.as_bytes());
    URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2])
}

/// `createUserTokens`: a JWT access token for the MCP resource, a refresh
/// token for `offline_access`, an ID token for `openid`.
async fn issue_tokens(state: &AuthState, client: &Client, issue: Issue) -> Result<Value, Response> {
    let iat = now_s(state);
    let exp = iat + MCP_ACCESS_TOKEN_TTL_SECONDS as i64;
    let iss = builder_origin(state);
    let key = signing_key(&state.db, &state.settings.secret, (state.now_ms)())
        .await
        .map_err(server_error)?;
    let audience = match issue.resources.as_slice() {
        [] => Value::Null,
        [one] => json!(one),
        many => json!(many),
    };
    // `isJwtAccessToken`: the default resource always applies here.
    let access_token = sign_jwt(
        &key,
        &json!({
            "sub": issue.user_id.to_string(),
            "aud": audience,
            "client_id": client.client_id,
            "azp": client.client_id,
            "scope": issue.scopes.join(" "),
            "sid": issue.session_id.map(|s| s.to_string()),
            "iss": iss,
            "iat": iat,
            "exp": exp,
            "jti": random_string(32),
        }),
    );
    let wants_refresh =
        allows_grant(client, "refresh_token") && issue.scopes.iter().any(|s| s == "offline_access");
    let refresh_token = if wants_refresh {
        let token = random_letters(32);
        let row = (
            store_hash(&token),
            client.client_id.clone(),
            issue.session_id,
            issue.user_id,
            issue.authorization_code_id.clone(),
            json!(issue.scopes).to_string(),
            (!issue.resources.is_empty()).then(|| json!(issue.resources).to_string()),
            issue.auth_time,
            issue.rotate,
        );
        let rotated = state
            .db
            .write(move |tx| {
                let (hash, client_id, session_id, user_id, code_id, scopes, resources, auth_time, rotate) = row;
                if let Some(old) = rotate {
                    let changed = tx.execute(
                        r#"UPDATE "OAuthRefreshToken" SET "revoked" = ?1, "rotatedAt" = ?1, "rotationReplayExpiresAt" = ?2
                           WHERE "id" = ?3 AND "revoked" IS NULL"#,
                        params![iat * 1000, (iat + REFRESH_REUSE_SECONDS) * 1000, old],
                    )?;
                    if changed == 0 {
                        return Ok(false);
                    }
                }
                tx.execute(
                    r#"INSERT INTO "OAuthRefreshToken" ("id", "createdAt", "updatedAt", "token", "clientId", "sessionId",
                           "userId", "authorizationCodeId", "scopes", "resources", "authTime", "expiresAt")
                       VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                    params![
                        new_id(), iat * 1000, hash, client_id, session_id, user_id, code_id, scopes, resources,
                        auth_time, (iat + MCP_REFRESH_TOKEN_TTL_SECONDS as i64) * 1000
                    ],
                )?;
                Ok(true)
            })
            .await
            .map_err(server_error)?;
        if !rotated {
            return Err(bad_request("invalid_grant", "invalid refresh token"));
        }
        Some(token)
    } else {
        None
    };
    let id_token = issue.scopes.iter().any(|s| s == "openid").then(|| {
        sign_jwt(
            &key,
            &json!({
                "auth_time": issue.auth_time.map(|ms| ms / 1000),
                "acr": "0",
                "at_hash": at_hash(&access_token),
                "iss": builder_origin(state),
                "sub": issue.user_id.to_string(),
                "aud": client.client_id,
                "nonce": issue.nonce,
                "iat": iat,
                "exp": iat + ID_TOKEN_TTL_SECONDS,
            }),
        )
    });
    let mut out = Map::new();
    out.insert("access_token".into(), json!(access_token));
    out.insert("expires_in".into(), json!(exp - iat));
    out.insert("expires_at".into(), json!(exp));
    out.insert("token_type".into(), json!("Bearer"));
    if let Some(refresh) = refresh_token {
        out.insert("refresh_token".into(), json!(refresh));
    }
    out.insert("scope".into(), json!(issue.scopes.join(" ")));
    if let Some(id_token) = id_token {
        out.insert("id_token".into(), json!(id_token));
    }
    let out = Value::Object(out);
    if let Some(old) = issue.rotate {
        // `storeRefreshTokenRotationReplay`: a retried refresh in the reuse window gets this answer.
        let sealed = symmetric_encrypt(&state.settings.secret, &out.to_string());
        let _ = state
            .db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE "OAuthRefreshToken" SET "rotationReplayResponse" = ?1 WHERE "id" = ?2"#,
                    params![sealed, old],
                )?;
                Ok(())
            })
            .await;
    }
    Ok(out)
}

/// `handleAuthorizationCodeGrant`.
async fn code_grant(state: &AuthState, req: &TokenRequest) -> Result<Value, Response> {
    if req.get("client_id").is_none() && req.basic.is_none() {
        return Err(bad_request("invalid_request", "client_id is required"));
    }
    let Some(code) = req.get("code").map(str::to_string) else {
        return Err(bad_request("invalid_request", "code is required"));
    };
    let verifier = req.get("code_verifier").map(str::to_string);
    let has_secret = req.basic.is_some() || req.get("client_secret").is_some();
    if !has_secret && verifier.is_none() {
        return Err(bad_request(
            "invalid_request",
            "Either code_verifier or client_secret is required",
        ));
    }
    let client = client_of(state, req, "authorization_code").await?;
    // `consumeVerificationValue`: single use, also under races (one writer).
    let code_id = store_hash(&code);
    let (lookup, now) = (code_id.clone(), (state.now_ms)());
    let value: Option<String> = state
        .db
        .write(move |tx| {
            let value: Option<String> = tx
                .query_row(
                    r#"SELECT "value" FROM "Verification" WHERE "identifier" = ?1 AND "expiresAt" > ?2 LIMIT 1"#,
                    params![lookup, now],
                    |r| r.get(0),
                )
                .optional()?;
            tx.execute(r#"DELETE FROM "Verification" WHERE "identifier" = ?1"#, [&lookup])?;
            if value.is_none() {
                // A replayed code: drop what it issued.
                tx.execute(r#"DELETE FROM "OAuthAccessToken" WHERE "authorizationCodeId" = ?1"#, [&lookup])?;
                tx.execute(r#"DELETE FROM "OAuthRefreshToken" WHERE "authorizationCodeId" = ?1"#, [&lookup])?;
            }
            Ok(value)
        })
        .await
        .map_err(server_error)?;
    let malformed = || bad_request("invalid_grant", "malformed verification value");
    let value: Value =
        serde_json::from_str(&value.ok_or_else(|| bad_request("invalid_grant", "invalid code"))?)
            .map_err(|_| malformed())?;
    if value["type"] != "authorization_code" {
        return Err(malformed());
    }
    let query = &value["query"];
    let q = |key: &str| query.get(key).and_then(Value::as_str);
    if q("client_id") != Some(client.client_id.as_str()) {
        return Err(bad_request("invalid_grant", "invalid client_id"));
    }
    match (q("redirect_uri"), req.get("redirect_uri")) {
        (Some(_), None) => return Err(bad_request("invalid_request", "redirect_uri is required")),
        (Some(bound), Some(sent)) if bound != sent => {
            return Err(bad_request("invalid_grant", "redirect_uri mismatch"));
        }
        (None, Some(_)) => return Err(bad_request("invalid_grant", "redirect_uri mismatch")),
        _ => {}
    }
    let stored: Vec<String> = value["resource"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let requested = req.all("resource");
    if !requested.is_empty() && !stored.is_empty() && requested.iter().any(|r| !stored.contains(r))
    {
        return Err(bad_request(
            "invalid_target",
            "requested resource not authorized",
        ));
    }
    let resources = if requested.is_empty() {
        stored
    } else {
        requested
    };
    let scopes: Vec<String> = q("scope")
        .ok_or_else(|| {
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_scope",
                "verification scope unset",
            )
        })?
        .split(' ')
        .map(str::to_string)
        .collect();
    check_client_scopes(&client, &scopes)?;
    if pkce_required(&client, &scopes, q("nonce")).is_some() && verifier.is_none() {
        return Err(bad_request(
            "invalid_request",
            "PKCE is required for this client",
        ));
    }
    match (q("code_challenge"), &verifier) {
        (Some(_), None) => {
            return Err(bad_request(
                "invalid_request",
                "code_verifier required because PKCE was used in authorization",
            ));
        }
        (None, Some(_)) => {
            return Err(bad_request(
                "invalid_request",
                "code_verifier provided but PKCE was not used in authorization",
            ));
        }
        (Some(challenge), Some(verifier)) => {
            let computed = (q("code_challenge_method") == Some("S256"))
                .then(|| URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())));
            if !computed.is_some_and(|c| constant_time_eq(&c, challenge)) {
                return Err(bad_request("invalid_grant", "code verification failed"));
            }
        }
        (None, None) => {}
    }
    let missing_user = || bad_request("invalid_user", "missing user, user may have been deleted");
    let user_id: i64 = value["userId"]
        .as_str()
        .and_then(|u| u.parse().ok())
        .ok_or_else(missing_user)?;
    let session_id: Option<i64> = value["sessionId"].as_str().and_then(|s| s.parse().ok());
    let now = (state.now_ms)();
    let (user_ok, session_ok) = state
        .db
        .read(move |c| {
            let user = c
                .query_row(r#"SELECT 1 FROM "User" WHERE "id" = ?1"#, [user_id], |_| {
                    Ok(())
                })
                .optional()?
                .is_some();
            let session = c
                .query_row(
                    r#"SELECT 1 FROM "Session" WHERE "id" = ?1 AND "expiresAt" >= ?2"#,
                    params![session_id, now],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            Ok((user, session))
        })
        .await
        .map_err(server_error)?;
    if !user_ok {
        return Err(missing_user());
    }
    if !session_ok {
        return Err(bad_request("invalid_request", "session no longer exists"));
    }
    issue_tokens(
        state,
        &client,
        Issue {
            user_id,
            session_id,
            auth_time: value["authTime"].as_i64(),
            scopes,
            resources,
            nonce: q("nonce").map(str::to_string),
            authorization_code_id: Some(code_id),
            rotate: None,
        },
    )
    .await
}

/// `handleRefreshTokenGrant` (rotation, 30 s replay window, family revoke on reuse).
async fn refresh_grant(state: &AuthState, req: &TokenRequest) -> Result<Value, Response> {
    if req.get("client_id").is_none() && req.basic.is_none() {
        return Err(bad_request("invalid_request", "Missing required client_id"));
    }
    let Some(token) = req.get("refresh_token").map(str::to_string) else {
        return Err(bad_request(
            "invalid_request",
            "Missing a required refresh_token for refresh_token grant",
        ));
    };
    let client = client_of(state, req, "refresh_token").await?;
    let hash = store_hash(&token);
    type Row = (
        i64,
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        i64,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    );
    let row: Option<Row> = state
        .db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT "id", "clientId", "userId", "sessionId", "scopes", "resources", "expiresAt",
                          "revoked", "rotationReplayResponse", "rotationReplayExpiresAt", "authTime", "authorizationCodeId"
                   FROM "OAuthRefreshToken" WHERE "token" = ?1"#,
                [hash],
                |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                        r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?,
                    ))
                },
            )
            .optional()?)
        })
        .await
        .map_err(server_error)?;
    let Some((
        id,
        client_id,
        user_id,
        session_id,
        scopes,
        resources,
        expires_at,
        revoked,
        replay,
        replay_until,
        auth_time,
        code_id,
    )) = row
    else {
        return Err(bad_request("invalid_grant", "session not found"));
    };
    let invalid = || bad_request("invalid_grant", "invalid refresh token");
    if client_id != client.client_id {
        return Err(invalid());
    }
    let now = (state.now_ms)();
    if expires_at < now {
        return Err(invalid());
    }
    let granted = strings(scopes).unwrap_or_default();
    let stored_resources = strings(resources);
    let requested_resources = req.all("resource");
    if !requested_resources.is_empty()
        && let Some(stored) = &stored_resources
        && requested_resources.iter().any(|r| !stored.contains(r))
    {
        return Err(bad_request("invalid_target", "requested resource invalid"));
    }
    let requested_scopes: Option<Vec<String>> = req
        .get("scope")
        .map(|s| s.split(' ').map(str::to_string).collect());
    if let Some(requested) = &requested_scopes
        && let Some(scope) = requested.iter().find(|s| !granted.contains(s))
    {
        return Err(bad_request(
            "invalid_scope",
            &format!("unable to issue scope {scope}"),
        ));
    }
    let scopes = requested_scopes.unwrap_or(granted.clone());
    check_client_scopes(&client, &scopes)?;
    let resources = if requested_resources.is_empty() {
        stored_resources.clone().unwrap_or_default()
    } else {
        requested_resources
    };
    if revoked.is_some() {
        if replay_until.is_some_and(|until| until > now) {
            // Same request inside the reuse window: the stored answer.
            let same = scopes == granted && Some(&resources) == stored_resources.as_ref();
            if same
                && let Some(sealed) = replay
                && let Some(plain) = symmetric_decrypt(&state.settings.secret, &sealed)
                && let Ok(answer) = serde_json::from_str::<Value>(&plain)
            {
                return Ok(answer);
            }
            return Err(invalid());
        }
        // `invalidateRefreshFamily`.
        let family_client = client.client_id.clone();
        state
            .db
            .write(move |tx| {
                tx.execute(
                    r#"DELETE FROM "OAuthAccessToken" WHERE "refreshId" IN
                       (SELECT "id" FROM "OAuthRefreshToken" WHERE "clientId" = ?1 AND "userId" = ?2)"#,
                    params![family_client, user_id],
                )?;
                tx.execute(
                    r#"DELETE FROM "OAuthRefreshToken" WHERE "clientId" = ?1 AND "userId" = ?2"#,
                    params![family_client, user_id],
                )?;
                Ok(())
            })
            .await
            .map_err(server_error)?;
        return Err(invalid());
    }
    let exists = state
        .db
        .read(move |c| {
            Ok(
                c.query_row(r#"SELECT 1 FROM "User" WHERE "id" = ?1"#, [user_id], |_| {
                    Ok(())
                })
                .optional()?
                .is_some(),
            )
        })
        .await
        .map_err(server_error)?;
    if !exists {
        return Err(bad_request("invalid_request", "user not found"));
    }
    issue_tokens(
        state,
        &client,
        Issue {
            user_id,
            session_id,
            auth_time,
            scopes,
            resources,
            nonce: None,
            authorization_code_id: code_id,
            rotate: Some(id),
        },
    )
    .await
}

fn basic_credentials(headers: &HeaderMap) -> Result<Option<(String, String)>, Response> {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(None);
    };
    let Some(encoded) = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))
    else {
        let mut response = oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "unsupported authorization scheme",
        );
        if let Some(scheme) = value
            .split(' ')
            .next()
            .and_then(|s| HeaderValue::from_str(s).ok())
        {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, scheme);
        }
        return Err(response);
    };
    let decoded = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| bad_request("invalid_client", "invalid authorization header format"))?;
    let (id, secret) = decoded
        .split_once(':')
        .ok_or_else(|| bad_request("invalid_client", "invalid authorization header format"))?;
    let unescape = |s: &str| {
        url::form_urlencoded::parse(format!("v={s}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    Ok(Some((unescape(id), unescape(secret))))
}

async fn token_inner(state: &AuthState, headers: &HeaderMap, body: &Bytes) -> Response {
    let basic = match basic_credentials(headers) {
        Ok(basic) => basic,
        Err(response) => return response,
    };
    let mut req = TokenRequest {
        body: token_body(headers, body),
        basic,
    };
    // `defaultMcpResource` on `/oauth2/token`.
    let resource = mcp_resource_url(state);
    let resources = req.all("resource");
    let mut as_map = Map::new();
    if let [one] = resources.as_slice() {
        as_map.insert("resource".into(), json!(one));
    } else if !resources.is_empty() {
        as_map.insert("resource".into(), json!(resources));
    }
    if let Some(McpResourceRewrite::Body(rewritten)) =
        default_mcp_resource("/oauth2/token", None, Some(&as_map), &resource)
        && let Some(r) = rewritten.get("resource").and_then(Value::as_str)
    {
        req.body.retain(|(k, _)| k != "resource");
        req.body.push(("resource".into(), r.to_string()));
    }
    let outcome = match req.get("grant_type") {
        None => Err(bad_request("invalid_request", "grant_type is required")),
        Some("authorization_code") => code_grant(state, &req).await,
        Some("refresh_token") => refresh_grant(state, &req).await,
        Some(_) => Err(bad_request(
            "unsupported_grant_type",
            "unsupported grant type",
        )),
    };
    match outcome {
        Ok(body) => json_with(
            StatusCode::OK,
            &body,
            &[("cache-control", "no-store"), ("pragma", "no-cache")],
        ),
        Err(response) => response,
    }
}

async fn token(State(state): State<Arc<McpOAuth>>, headers: HeaderMap, body: Bytes) -> Response {
    with_cors(token_inner(&state, &headers, &body).await)
}

async fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

async fn preflight(method: Method) -> Response {
    let _ = method;
    with_cors(StatusCode::NO_CONTENT.into_response())
}

/// The MCP OAuth routes.
pub fn mcp_oauth_router(state: Arc<McpOAuth>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata),
        )
        .route("/.well-known/openid-configuration", get(server_metadata))
        .route(
            "/.well-known/oauth-protected-resource/{*path}",
            get(protected_resource),
        )
        .route(
            "/api/auth/oauth2/register",
            post(register).options(preflight),
        )
        .route("/api/auth/oauth2/authorize", get(authorize))
        .route("/api/auth/oauth2/token", post(token).options(preflight))
        .route("/api/auth/oauth2/consent", post(not_found))
        .route("/oauth/sign-in", get(oauth_sign_in))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_rules() {
        assert!(validate_redirect_uri("https://app.example.com/cb", "web").is_ok());
        assert!(validate_redirect_uri("http://app.example.com/cb", "web").is_err());
        assert!(validate_redirect_uri("http://127.0.0.1:3000/cb", "native").is_ok());
        assert!(validate_redirect_uri("http://example.com/cb", "native").is_err());
        assert!(validate_redirect_uri("com.example.app:/cb", "native").is_ok());
        assert!(validate_redirect_uri("javascript://x/alert", "native").is_err());
        assert!(registered_redirect(
            &["http://127.0.0.1/cb".into()],
            "http://127.0.0.1:5555/cb"
        ));
        assert!(!registered_redirect(
            &["https://a.example/cb".into()],
            "https://a.example/cb/x"
        ));
        assert!(!registered_redirect(
            &["https://a.example/cb".into()],
            "https://a.example/cb#x"
        ));
    }

    #[test]
    fn hash_is_base64url_sha256() {
        assert_eq!(
            store_hash("abc"),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }
}
