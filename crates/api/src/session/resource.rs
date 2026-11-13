//! OAuth resource-server checks of `packages/auth`: `mcp-resource.ts`
//! (`verifyMcpAccessToken`, `verifyMcpOAuthQuery`) and `mobile-resource.ts`
//! (`verifyMobileAccessToken`). The JWT verification itself
//! (`oauthProviderResourceClient(auth).verifyBearerToken`) and
//! `verifyOAuthQueryParams` are traits.

use std::future::Future;

use serde_json::Value;

/// `verifyBearerToken(token, { jwksUrl, verifyOptions: { audience, issuer } })`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyBearerOptions {
    pub jwks_url: String,
    pub audience: String,
    pub issuer: String,
}

/// A rejection of `verifyBearerToken`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BearerVerifyError {
    /// A better-auth `APIError` (`UNAUTHORIZED`, `INTERNAL_SERVER_ERROR`, ...).
    Api { status: String, message: String },
    /// A jose error with its `code` (`ERR_JWT_EXPIRED`, `ERR_JWKS_TIMEOUT`, ...).
    Jose(String),
    /// Any other error.
    Other(String),
}

/// The JWT verifier of the resource client.
pub trait BearerTokenVerifier: Sync {
    fn verify_bearer_token(
        &self,
        token: &str,
        options: &VerifyBearerOptions,
    ) -> impl Future<Output = Result<Value, BearerVerifyError>> + Send;
}

/// `McpTokenVerificationUnavailableError` / `MobileTokenVerificationUnavailableError`
/// (`Unavailable`), or the rethrown error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenVerifyError {
    Unavailable,
    Rethrown(BearerVerifyError),
}

/// `McpAccessTokenSubject`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAccessTokenSubject {
    pub client_id: String,
    pub user_id: String,
}

/// `MobileAccessTokenSubject`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAccessTokenSubject {
    pub user_id: String,
    pub device_id: Option<String>,
    pub must_change_password: bool,
}

/// `verifyMcpAccessToken(auth, token)`; `auth_base_url` is better-auth's
/// `baseURL` (`<BETTER_AUTH_URL>/api/auth`).
pub async fn verify_mcp_access_token<V: BearerTokenVerifier>(
    verifier: &V,
    auth_base_url: &str,
    mcp_resource_url: &str,
    token: &str,
) -> Result<Option<McpAccessTokenSubject>, TokenVerifyError> {
    let _ = (verifier, auth_base_url, mcp_resource_url, token);
    todo!("P3.5e")
}

/// `verifyMobileAccessToken(auth, token)`.
pub async fn verify_mobile_access_token<V: BearerTokenVerifier>(
    verifier: &V,
    auth_base_url: &str,
    mobile_audience: &str,
    token: &str,
) -> Result<Option<MobileAccessTokenSubject>, TokenVerifyError> {
    let _ = (verifier, auth_base_url, mobile_audience, token);
    todo!("P3.5e")
}

/// `verifyOAuthQueryParams(query, secret)`.
pub trait OAuthQueryVerifier: Sync {
    fn verify_oauth_query_params(
        &self,
        query: &str,
        secret: &str,
    ) -> impl Future<Output = bool> + Send;
}

/// `McpConsentQuery`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConsentQuery {
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// `verifyMcpOAuthQuery(query)` with `BETTER_AUTH_SECRET` as `secret`.
pub async fn verify_mcp_oauth_query<V: OAuthQueryVerifier>(
    verifier: &V,
    query: &str,
    secret: &str,
) -> Option<McpConsentQuery> {
    let _ = (verifier, query, secret);
    todo!("P3.5e")
}
