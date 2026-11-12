//! `workspaceTokenAuthMidddleware` (`apps/builder/src/middlewares/workspace-token-auth.ts`)
//! and `workspaceTokenAuthAPIForScope` (`requireTokenScope`, `apps/builder/src/orpc.ts`).
//!
//! The services the middleware calls are a trait ([`WorkspaceTokenDeps`]), so the
//! tests can stub them like the vitest mocks of the original.

use std::future::Future;

use axum::http::{HeaderMap, HeaderValue, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_kv::rate_limit::RateLimited;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    RequestApiToken, WorkspaceQuota, assert_workspace_owner_access_for_method,
    is_read_only_token_allowed_method, token_has_any_scope,
};
use crate::errors::ChatbotXException;
use crate::orpc::OrpcError;

/// Rate-limit scope of an authenticated workspace token (key: workspace id).
pub const WORKSPACE_TOKEN_RATE_LIMIT: &str = "workspace-token-rate-limit";
/// Rate-limit scope before the token lookup (key: client IP).
pub const WORKSPACE_TOKEN_PREAUTH_RATE_LIMIT: &str = "workspace-token-preauth-rate-limit";
/// Rate-limit scope of a mobile user (key: user id).
pub const MOBILE_USER_RATE_LIMIT: &str = "mobile-user-rate-limit";
/// Requests per 10 s window per client IP before the token lookup.
pub const PREAUTH_REQUEST_LIMIT: u64 = 600;
/// `UNKNOWN_CLIENT_IP` of `guest-rate-limit.ts`.
pub const UNKNOWN_CLIENT_IP: &str = "unknown";
/// The warn line of a `?token=` request.
pub const DEPRECATED_QUERY_TOKEN_WARNING: &str =
    "Workspace token authenticated via deprecated ?token= query param";
/// The warn line of a cached token whose workspace is purged.
pub const PURGED_WORKSPACE_TOKEN_WARNING: &str =
    "Workspace token cache pointed at a purged workspace";

/// The workspace fields the middleware and the routes read (`WorkspaceModel`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthWorkspace {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    /// `scheduledDeletionAt` (epoch ms); `Some` = deletion scheduled.
    pub scheduled_deletion_at: Option<i64>,
}

/// `workspaceApiTokenService.findWorkspaceByTokenHash` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTokenLookup {
    pub workspace: AuthWorkspace,
    pub api_token: RequestApiToken,
}

/// `verifyMcpAccessToken` result (a verified MCP OAuth access token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAccessToken {
    pub client_id: String,
    pub user_id: String,
}

/// `McpTokenVerificationUnavailableError` (the JWKS cannot be read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpTokenVerificationUnavailable;

/// A workspace that `mcpOAuthService.resolvePrincipal` offers for selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChoice {
    pub id: String,
    pub name: String,
}

/// `mcpOAuthService.resolvePrincipal` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthPrincipalResolution {
    InvalidToken,
    Forbidden,
    WorkspaceSelectionRequired(Vec<WorkspaceChoice>),
    Principal {
        workspace: AuthWorkspace,
        user_id: String,
        client_id: String,
    },
}

/// `resolveMobileWorkspacePrincipal` result (P3.5e fills the member fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileWorkspacePrincipal {
    pub workspace: AuthWorkspace,
    pub api_token: RequestApiToken,
    pub user_id: String,
    pub member_id: Option<String>,
    pub device_id: Option<String>,
    /// `mobileCredentialId(mobile)`.
    pub credential_id: String,
}

/// `withAuditContext` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditContext {
    pub user_id: String,
    pub workspace_id: String,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub source: String,
}

/// The context `next` receives (`workspace`, `apiToken`, `apiCredentialId`, mobile fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTokenContext {
    pub workspace: AuthWorkspace,
    pub api_token: RequestApiToken,
    pub api_credential_id: String,
    pub audit: AuditContext,
    /// Set for a mobile access token.
    pub mobile: Option<MobileWorkspacePrincipal>,
    /// `requestIssuedAtMs` (set for a mobile access token).
    pub request_issued_at_ms: Option<i64>,
}

/// One request as the middleware sees it.
#[derive(Debug, Clone, Copy)]
pub struct TokenAuthRequest<'a> {
    pub headers: &'a HeaderMap,
    /// The full request URL (`context.url`), for `?token=`.
    pub url: Option<&'a str>,
    /// The route method (`procedure["~orpc"].route.method`).
    pub method: Option<&'a str>,
    /// The route path template (`procedure["~orpc"].route.path`).
    pub path: Option<&'a str>,
}

/// The services of the middleware.
pub trait WorkspaceTokenDeps: WorkspaceQuota + Sync {
    /// `workspaceApiTokenService.findWorkspaceByTokenHash({ tokenHash })`
    /// (`Err` with code `notFound` = the token's workspace is purged).
    fn find_workspace_by_token_hash(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = Result<Option<WorkspaceTokenLookup>, ChatbotXException>> + Send;

    /// `assertApiNotRateLimited({ scope, key, limit })`.
    fn assert_not_rate_limited(
        &self,
        scope: &str,
        key: &str,
        limit: Option<u64>,
    ) -> impl Future<Output = Result<(), RateLimited>> + Send;

    /// `logger.warn(fields, message)`.
    fn warn(&self, fields: Value, message: &str);

    /// `isCloud()`.
    fn is_cloud(&self) -> bool;

    /// `getMobileApiAudience()`; `None` = mobile access tokens are off.
    fn mobile_api_audience(&self) -> Option<String>;

    /// `resolveMobileWorkspacePrincipal({ bearerToken, requestedWorkspaceId, method, path })`.
    fn resolve_mobile_principal(
        &self,
        bearer_token: &str,
        requested_workspace_id: Option<&str>,
        method: Option<&str>,
        path: Option<&str>,
    ) -> impl Future<Output = Result<MobileWorkspacePrincipal, OrpcError>> + Send;

    /// `verifyMcpAccessToken(auth, bearerToken)`.
    fn verify_mcp_access_token(
        &self,
        bearer_token: &str,
    ) -> impl Future<Output = Result<Option<McpAccessToken>, McpTokenVerificationUnavailable>> + Send;

    /// `mcpOAuthService.resolvePrincipal({ clientId, userId, workspaceId })`.
    fn resolve_oauth_principal(
        &self,
        client_id: &str,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> impl Future<Output = OAuthPrincipalResolution> + Send;

    /// `Date.now()`.
    fn now_ms(&self) -> i64;

    /// `findIntegrationApiByTokenHash` + its workspace and inbox (the API
    /// channel token of `channelApiTokenAuthMidddleware`). Default: no channel.
    fn find_channel_api_by_token_hash(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = Result<Option<ChannelApiLookup>, ChatbotXException>> + Send {
        let _ = token_hash;
        async { Ok(None) }
    }
}

/// An API channel (`IntegrationApi`) found by its token hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelApiLookup {
    pub integration_api_id: String,
    pub enabled: bool,
    pub workspace: AuthWorkspace,
    /// `inboxService.find({ id: integrationApi.inboxId })` found the inbox.
    pub inbox_id: Option<String>,
}

/// The context of an API channel request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTokenContext {
    pub integration_api_id: String,
    pub workspace: AuthWorkspace,
    pub inbox_id: String,
    /// `channel-api:<id>`.
    pub api_credential_id: String,
}

/// `channelApiTokenAuthMidddleware`: `Bearer <cbx_api_ token>` of an enabled API channel.
pub async fn authenticate_channel_api_token<D: WorkspaceTokenDeps>(
    deps: &D,
    headers: &HeaderMap,
) -> Result<ChannelTokenContext, OrpcError> {
    let unauthorized = || OrpcError::new("UNAUTHORIZED", None, None);
    let token = extract_bearer(headers)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(unauthorized)?;
    let lookup = deps
        .find_channel_api_by_token_hash(&hash_token(token))
        .await
        .map_err(|error| chatbotx_exception_to_orpc(&error))?
        .filter(|lookup| lookup.enabled)
        .ok_or_else(unauthorized)?;
    if is_workspace_scheduled_for_deletion(&lookup.workspace) {
        return Err(forbidden("Workspace deletion scheduled"));
    }
    let inbox_id = lookup
        .inbox_id
        .ok_or_else(|| OrpcError::new("UNAUTHORIZED", None, Some("Inbox not found")))?;
    Ok(ChannelTokenContext {
        api_credential_id: format!("channel-api:{}", lookup.integration_api_id),
        integration_api_id: lookup.integration_api_id,
        workspace: lookup.workspace,
        inbox_id,
    })
}

/// The answer of a mobile-user route to anything but a valid mobile access
/// token (`mobileUserAuthMiddleware`; the verification itself is P3.5e).
pub fn invalid_access_token_error() -> OrpcError {
    OrpcError::new(
        "INVALID_CHATBOT_TOKEN",
        Some(401),
        Some("Invalid or missing access token"),
    )
}

/// `extractBearer(headers)`: the value after `Bearer ` of `Authorization`.
pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// `hashToken(token)`: lowercase hex SHA-256 of the UTF-8 token.
pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// `getGuestClientIp(headers)`: first `x-forwarded-for` entry, else `x-real-ip`, else `unknown`.
pub fn guest_client_ip(headers: &HeaderMap) -> String {
    if let Some(forwarded_for) = header_str(headers, "x-forwarded-for") {
        let first = forwarded_for.split(',').next().unwrap_or_default().trim();
        return if first.is_empty() {
            UNKNOWN_CLIENT_IP.to_string()
        } else {
            first.to_string()
        };
    }
    match header_str(headers, "x-real-ip").map(str::trim) {
        Some(ip) if !ip.is_empty() => ip.to_string(),
        _ => UNKNOWN_CLIENT_IP.to_string(),
    }
}

/// `isWorkspaceScheduledForDeletion(workspace)`.
pub fn is_workspace_scheduled_for_deletion(workspace: &AuthWorkspace) -> bool {
    workspace.scheduled_deletion_at.is_some()
}

/// `peekJwtAudiences(token)`: the unverified `aud` claim (string or string array).
pub fn peek_jwt_audiences(token: &str) -> Vec<String> {
    let segments: Vec<&str> = token.split('.').collect();
    if segments.len() != JWT_SEGMENT_COUNT {
        return Vec::new();
    }
    let payload = segments[1].trim_end_matches('=');
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload) else {
        return Vec::new();
    };
    let Ok(claims) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    match &claims["aud"] {
        Value::String(aud) => vec![aud.clone()],
        Value::Array(auds) => auds
            .iter()
            .filter_map(|aud| aud.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// `isMobileAccessTokenCandidate(token)`: the unverified `aud` claim names `audience`.
pub fn is_mobile_access_token_candidate(token: &str, audience: &str) -> bool {
    peek_jwt_audiences(token).iter().any(|aud| aud == audience)
}

/// `INVALID_CHATBOT_TOKEN` 401 "Invalid or missing workspace API token".
pub fn invalid_token_error() -> OrpcError {
    OrpcError::new(
        "INVALID_CHATBOT_TOKEN",
        Some(401),
        Some("Invalid or missing workspace API token"),
    )
}

/// The oRPC error of a `ChatbotXException` (`toKnownOrpcError`).
pub fn chatbotx_exception_to_orpc(error: &ChatbotXException) -> OrpcError {
    OrpcError::new(
        &error.code,
        Some(error.http_status_code),
        Some(&error.message),
    )
}

/// The oRPC error of an API rate limit (`tooManyRequests` 429).
pub fn rate_limited_to_orpc(error: &RateLimited) -> OrpcError {
    OrpcError::new(
        error.code,
        Some(error.http_status),
        Some(&error.to_string()),
    )
}

const JWT_SEGMENT_COUNT: usize = 3;

fn forbidden(message: &str) -> OrpcError {
    OrpcError::new("FORBIDDEN", None, Some(message))
}

/// `(pathname, query)` of a full URL or of a path with a query.
fn split_url(url: &str) -> (&str, &str) {
    let without_fragment = url.split('#').next().unwrap_or_default();
    let after_authority = match without_fragment.find("://") {
        Some(scheme_end) => {
            let rest = &without_fragment[scheme_end + 3..];
            rest.find(['/', '?']).map_or("", |start| &rest[start..])
        }
        None => without_fragment,
    };
    let (path, query) = after_authority
        .split_once('?')
        .unwrap_or((after_authority, ""));
    (if path.is_empty() { "/" } else { path }, query)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `application/x-www-form-urlencoded` decoding of one query component.
pub(crate) fn decode_query_component(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' => {
                match (
                    bytes.get(index + 1).copied().and_then(hex_value),
                    bytes.get(index + 2).copied().and_then(hex_value),
                ) {
                    (Some(high), Some(low)) => {
                        out.push(high * 16 + low);
                        index += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `url.searchParams.get(name)` (first value).
fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split_once('=').unwrap_or((pair, "")))
        .find(|(key, _)| decode_query_component(key) == name)
        .map(|(_, value)| decode_query_component(value))
}

async fn rate_limit<D: WorkspaceTokenDeps>(
    deps: &D,
    scope: &str,
    key: &str,
    limit: Option<u64>,
    res_headers: &mut HeaderMap,
) -> Result<(), OrpcError> {
    deps.assert_not_rate_limited(scope, key, limit)
        .await
        .map_err(|limited| {
            res_headers.insert(header::RETRY_AFTER, HeaderValue::from(limited.retry_after));
            rate_limited_to_orpc(&limited)
        })
}

/// The principal behind the token, before the shared gates.
struct Resolved {
    workspace: AuthWorkspace,
    api_token: RequestApiToken,
    audit_user_id: String,
    audit_source: String,
    credential_id: String,
    mobile: Option<MobileWorkspacePrincipal>,
}

async fn resolve_oauth<D: WorkspaceTokenDeps>(
    deps: &D,
    bearer: Option<&str>,
    requested_workspace_id: Option<&str>,
) -> Result<Resolved, OrpcError> {
    let Some(bearer) = bearer.filter(|b| b.split('.').count() == JWT_SEGMENT_COUNT) else {
        return Err(invalid_token_error());
    };
    let access_token = deps
        .verify_mcp_access_token(bearer)
        .await
        .map_err(|McpTokenVerificationUnavailable| {
            OrpcError::new(
                "SERVICE_UNAVAILABLE",
                None,
                Some("Token verification is temporarily unavailable"),
            )
        })?
        .ok_or_else(invalid_token_error)?;
    let resolution = deps
        .resolve_oauth_principal(
            &access_token.client_id,
            &access_token.user_id,
            requested_workspace_id,
        )
        .await;
    match resolution {
        OAuthPrincipalResolution::InvalidToken => Err(invalid_token_error()),
        OAuthPrincipalResolution::Forbidden => {
            Err(forbidden("Account has no access to this workspace"))
        }
        OAuthPrincipalResolution::WorkspaceSelectionRequired(workspaces) => {
            let list = workspaces
                .iter()
                .map(|workspace| format!("{} ({})", workspace.id, workspace.name))
                .collect::<Vec<_>>()
                .join(", ");
            Err(OrpcError::new(
                "WORKSPACE_SELECTION_REQUIRED",
                Some(400),
                Some(&format!(
                    "This account has access to {} workspaces. Disconnect and reconnect this MCP client to choose one on the consent screen, or add ?workspace_id=<id> to the MCP server URL (or send the x-workspace-id header). Available: {list}",
                    workspaces.len()
                )),
            ))
        }
        OAuthPrincipalResolution::Principal {
            workspace,
            user_id,
            client_id,
        } => {
            let source = format!("mcp-oauth:{client_id}");
            Ok(Resolved {
                api_token: RequestApiToken {
                    id: format!("oauth:{client_id}:{user_id}"),
                    workspace_id: workspace.id.clone(),
                    permission: "full".to_string(),
                    scopes: None,
                    is_default: false,
                },
                workspace,
                audit_user_id: user_id,
                credential_id: source.clone(),
                audit_source: source,
                mobile: None,
            })
        }
    }
}

/// `workspaceTokenAuthMidddleware`: authenticate the request. On a rate limit
/// the `Retry-After` header goes to `res_headers`.
pub async fn authenticate_workspace_token<D: WorkspaceTokenDeps>(
    deps: &D,
    request: TokenAuthRequest<'_>,
    res_headers: &mut HeaderMap,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let request_issued_at_ms = deps.now_ms();
    let headers = request.headers;
    let bearer = extract_bearer(headers);
    let (url_path, query) = request.url.map_or(("", ""), split_url);
    let api_key = query_param(query, "token");
    let requested_workspace_id = header_str(headers, "x-workspace-id")
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let Some(token) = bearer.map(str::to_string).or_else(|| api_key.clone()) else {
        return Err(invalid_token_error());
    };
    if bearer.is_none() && api_key.is_some() {
        deps.warn(json!({ "path": url_path }), DEPRECATED_QUERY_TOKEN_WARNING);
    }
    let client_ip = guest_client_ip(headers);
    rate_limit(
        deps,
        WORKSPACE_TOKEN_PREAUTH_RATE_LIMIT,
        &client_ip,
        Some(PREAUTH_REQUEST_LIMIT),
        res_headers,
    )
    .await?;

    let mobile_bearer = bearer.filter(|bearer| {
        deps.mobile_api_audience()
            .is_some_and(|audience| is_mobile_access_token_candidate(bearer, &audience))
    });
    let resolved = if let Some(bearer) = mobile_bearer {
        let mobile = deps
            .resolve_mobile_principal(bearer, requested_workspace_id, request.method, request.path)
            .await?;
        Resolved {
            workspace: mobile.workspace.clone(),
            api_token: mobile.api_token.clone(),
            audit_user_id: mobile.user_id.clone(),
            audit_source: format!(
                "mobile:{}",
                mobile.device_id.as_deref().unwrap_or("unknown")
            ),
            credential_id: mobile.credential_id.clone(),
            mobile: Some(mobile),
        }
    } else {
        let token_hash = hash_token(&token);
        let lookup = match deps.find_workspace_by_token_hash(&token_hash).await {
            Ok(lookup) => lookup,
            Err(error) if error.code == "notFound" => {
                deps.warn(
                    json!({ "err": error.message, "tokenHash": token_hash }),
                    PURGED_WORKSPACE_TOKEN_WARNING,
                );
                return Err(invalid_token_error());
            }
            Err(error) => return Err(chatbotx_exception_to_orpc(&error)),
        };
        match lookup {
            Some(WorkspaceTokenLookup {
                workspace,
                api_token,
            }) => Resolved {
                audit_user_id: workspace.owner_id.clone(),
                audit_source: format!("api-token:{}", api_token.id),
                credential_id: format!("api-token:{}", api_token.id),
                workspace,
                api_token,
                mobile: None,
            },
            None => resolve_oauth(deps, bearer, requested_workspace_id).await?,
        }
    };

    match &resolved.mobile {
        Some(mobile) => {
            rate_limit(
                deps,
                MOBILE_USER_RATE_LIMIT,
                &mobile.user_id,
                None,
                res_headers,
            )
            .await?;
        }
        None => {
            rate_limit(
                deps,
                WORKSPACE_TOKEN_RATE_LIMIT,
                &resolved.workspace.id,
                None,
                res_headers,
            )
            .await?;
        }
    }
    if is_workspace_scheduled_for_deletion(&resolved.workspace) {
        return Err(forbidden("Workspace deletion scheduled"));
    }
    if resolved.api_token.permission == "read_only"
        && !is_read_only_token_allowed_method(request.method, request.path)
    {
        return Err(forbidden("Read-only token cannot perform this operation"));
    }
    assert_workspace_owner_access_for_method(
        deps,
        deps.is_cloud(),
        request.method,
        &resolved.workspace.owner_id,
        request.path,
    )
    .await?;

    let audit = AuditContext {
        user_id: resolved.audit_user_id,
        workspace_id: resolved.workspace.id.clone(),
        ip_address: client_ip,
        user_agent: header_str(headers, "user-agent").map(str::to_string),
        source: resolved.audit_source,
    };
    let is_mobile = resolved.mobile.is_some();
    Ok(WorkspaceTokenContext {
        workspace: resolved.workspace,
        api_token: resolved.api_token,
        api_credential_id: resolved.credential_id,
        audit,
        mobile: resolved.mobile,
        request_issued_at_ms: is_mobile.then_some(request_issued_at_ms),
    })
}

/// `requireTokenScope(scope)`: any-of; message `Token is not authorized for the 'a' or 'b' scope`.
pub fn require_token_scope(api_token: &RequestApiToken, scopes: &[&str]) -> Result<(), OrpcError> {
    if token_has_any_scope(api_token.scopes.as_deref(), scopes) {
        return Ok(());
    }
    Err(forbidden(&format!(
        "Token is not authorized for the '{}' scope",
        scopes.join("' or '")
    )))
}

/// `workspaceTokenAuthAPIForScope(scope)`: the middleware, then the scope check.
pub async fn authorize_workspace_token_for_scope<D: WorkspaceTokenDeps>(
    deps: &D,
    request: TokenAuthRequest<'_>,
    scopes: &[&str],
    res_headers: &mut HeaderMap,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let context = authenticate_workspace_token(deps, request, res_headers).await?;
    require_token_scope(&context.api_token, scopes)?;
    Ok(context)
}
