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
    let claims = verify_claims(verifier, auth_base_url, mcp_resource_url, token, "MCP").await?;
    let Some(claims) = claims else {
        return Ok(None);
    };
    match (claims["client_id"].as_str(), claims["sub"].as_str()) {
        (Some(client_id), Some(user_id)) => Ok(Some(McpAccessTokenSubject {
            client_id: client_id.to_string(),
            user_id: user_id.to_string(),
        })),
        _ => Ok(None),
    }
}

/// `verifyMobileAccessToken(auth, token)`.
pub async fn verify_mobile_access_token<V: BearerTokenVerifier>(
    verifier: &V,
    auth_base_url: &str,
    mobile_audience: &str,
    token: &str,
) -> Result<Option<MobileAccessTokenSubject>, TokenVerifyError> {
    let claims = verify_claims(verifier, auth_base_url, mobile_audience, token, "Mobile").await?;
    let Some(claims) = claims else {
        return Ok(None);
    };
    let Some(user_id) = claims["sub"].as_str().filter(|sub| !sub.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(MobileAccessTokenSubject {
        user_id: user_id.to_string(),
        device_id: claims["deviceId"].as_str().map(str::to_string),
        must_change_password: claims["mustChangePassword"] == Value::Bool(true),
    }))
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
    if !verifier.verify_oauth_query_params(query, secret).await {
        return None;
    }
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let get = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let client_id = get("client_id").filter(|id| !id.is_empty())?;
    let scopes = get("scope")
        .map(|scope| {
            scope
                .split(' ')
                .filter(|scope| !scope.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(McpConsentQuery { client_id, scopes })
}

/// jose codes of a JWKS that cannot be read (infrastructure, not the token).
const JWKS_INFRASTRUCTURE_ERROR_CODES: [&str; 3] = [
    "ERR_JWKS_TIMEOUT",
    "ERR_JWKS_INVALID",
    "ERR_JWKS_MULTIPLE_MATCHING_KEYS",
];

/// `verifyBearerToken(...).catch(...)`: an invalid token is `None`, a JWKS
/// failure is `Unavailable`, anything else is rethrown.
async fn verify_claims<V: BearerTokenVerifier>(
    verifier: &V,
    auth_base_url: &str,
    audience: &str,
    token: &str,
    kind: &str,
) -> Result<Option<Value>, TokenVerifyError> {
    let issuer = url::Url::parse(auth_base_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let options = VerifyBearerOptions {
        jwks_url: format!("{auth_base_url}/jwks"),
        audience: audience.to_string(),
        issuer,
    };
    match verifier.verify_bearer_token(token, &options).await {
        Ok(claims) => Ok(Some(claims)),
        Err(BearerVerifyError::Api { status, .. }) if status == "UNAUTHORIZED" => Ok(None),
        Err(BearerVerifyError::Jose(code))
            if JWKS_INFRASTRUCTURE_ERROR_CODES.contains(&code.as_str()) =>
        {
            tracing::error!(code, "{kind} JWKS verification unavailable");
            Err(TokenVerifyError::Unavailable)
        }
        Err(BearerVerifyError::Jose(code)) if code.starts_with("ERR_J") => Ok(None),
        Err(error) => Err(TokenVerifyError::Rethrown(error)),
    }
}
