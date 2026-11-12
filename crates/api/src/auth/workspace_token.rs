//! `workspaceTokenAuthMidddleware` (`apps/builder/src/middlewares/workspace-token-auth.ts`)
//! and `workspaceTokenAuthAPIForScope` (`requireTokenScope`, `apps/builder/src/orpc.ts`).
//!
//! The services the middleware calls are a trait ([`WorkspaceTokenDeps`]), so the
//! tests can stub them like the vitest mocks of the original.

use std::future::Future;

use axum::http::HeaderMap;
use chatbotx_kv::rate_limit::RateLimited;
use serde_json::Value;

use super::{RequestApiToken, WorkspaceQuota};
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
}

/// `extractBearer(headers)`: the value after `Bearer ` of `Authorization`.
pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let _ = headers;
    todo!("P3.5b")
}

/// `hashToken(token)`: lowercase hex SHA-256 of the UTF-8 token.
pub fn hash_token(token: &str) -> String {
    let _ = token;
    todo!("P3.5b")
}

/// `getGuestClientIp(headers)`: first `x-forwarded-for` entry, else `x-real-ip`, else `unknown`.
pub fn guest_client_ip(headers: &HeaderMap) -> String {
    let _ = headers;
    todo!("P3.5b")
}

/// `isWorkspaceScheduledForDeletion(workspace)`.
pub fn is_workspace_scheduled_for_deletion(workspace: &AuthWorkspace) -> bool {
    let _ = workspace;
    todo!("P3.5b")
}

/// `isMobileAccessTokenCandidate(token)`: the unverified `aud` claim names `audience`.
pub fn is_mobile_access_token_candidate(token: &str, audience: &str) -> bool {
    let _ = (token, audience);
    todo!("P3.5b")
}

/// `INVALID_CHATBOT_TOKEN` 401 "Invalid or missing workspace API token".
pub fn invalid_token_error() -> OrpcError {
    todo!("P3.5b")
}

/// The oRPC error of a `ChatbotXException` (`toKnownOrpcError`).
pub fn chatbotx_exception_to_orpc(error: &ChatbotXException) -> OrpcError {
    let _ = error;
    todo!("P3.5b")
}

/// The oRPC error of an API rate limit (`tooManyRequests` 429).
pub fn rate_limited_to_orpc(error: &RateLimited) -> OrpcError {
    let _ = error;
    todo!("P3.5b")
}

/// `workspaceTokenAuthMidddleware`: authenticate the request. On a rate limit
/// the `Retry-After` header goes to `res_headers`.
pub async fn authenticate_workspace_token<D: WorkspaceTokenDeps>(
    deps: &D,
    request: TokenAuthRequest<'_>,
    res_headers: &mut HeaderMap,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let _ = (deps, request, res_headers);
    todo!("P3.5b")
}

/// `requireTokenScope(scope)`: any-of; message `Token is not authorized for the 'a' or 'b' scope`.
pub fn require_token_scope(api_token: &RequestApiToken, scopes: &[&str]) -> Result<(), OrpcError> {
    let _ = (api_token, scopes);
    todo!("P3.5b")
}

/// `workspaceTokenAuthAPIForScope(scope)`: the middleware, then the scope check.
pub async fn authorize_workspace_token_for_scope<D: WorkspaceTokenDeps>(
    deps: &D,
    request: TokenAuthRequest<'_>,
    scopes: &[&str],
    res_headers: &mut HeaderMap,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let _ = (deps, request, scopes, res_headers);
    todo!("P3.5b")
}
