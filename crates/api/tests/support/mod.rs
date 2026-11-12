//! Stub services of the workspace-token middleware (the vitest mocks of
//! `workspace-token-auth-middleware.test.ts` and the `*-public-scope` tests).
#![allow(dead_code)]

use std::sync::Mutex;

use axum::body::to_bytes;
use axum::http::{HeaderMap, HeaderValue, Method, Uri};
use chatbotx_api::auth::{
    AuthWorkspace, McpAccessToken, McpTokenVerificationUnavailable, MobileWorkspacePrincipal,
    OAuthPrincipalResolution, OwnerAccessState, RequestApiToken, WorkspaceQuota,
    WorkspaceTokenDeps, WorkspaceTokenLookup,
};
use chatbotx_api::errors::ChatbotXException;
use chatbotx_api::orpc::OrpcError;
use chatbotx_api::routes::{RouteAuth, handle_public_request, route_by_id};
use chatbotx_kv::rate_limit::RateLimited;
use serde_json::Value;

/// The token every stubbed request sends.
pub const TOKEN: &str = "cbx_ws_fixture";

/// What `findWorkspaceByTokenHash` answers.
#[derive(Debug, Clone)]
pub enum Lookup {
    /// `mockResolvedValue(value)`.
    Value(Option<WorkspaceTokenLookup>),
    /// `mockRejectedValue(error)`.
    Error(ChatbotXException),
    /// `mockImplementation(candidate === hash ? value : undefined)`.
    ForHash(String, WorkspaceTokenLookup),
}

/// What `verifyMcpAccessToken` answers.
#[derive(Debug, Clone)]
pub enum McpVerify {
    Value(Option<McpAccessToken>),
    Unavailable,
}

/// The stubbed services and the calls they saw.
pub struct MockDeps {
    pub lookup: Mutex<Lookup>,
    pub find_calls: Mutex<Vec<String>>,
    pub rate_limit_calls: Mutex<Vec<(String, String, Option<u64>)>>,
    pub rate_limited: Mutex<Option<RateLimited>>,
    pub warns: Mutex<Vec<(Value, String)>>,
    pub access_state: Mutex<OwnerAccessState>,
    pub access_calls: Mutex<Vec<String>>,
    pub at_limit: Mutex<bool>,
    pub mcp: Mutex<McpVerify>,
    pub mcp_calls: Mutex<Vec<String>>,
    pub resolution: Mutex<OAuthPrincipalResolution>,
    pub resolve_calls: Mutex<Vec<(String, String, Option<String>)>>,
    pub cloud: bool,
}

impl Default for MockDeps {
    fn default() -> Self {
        MockDeps {
            lookup: Mutex::new(Lookup::Value(None)),
            find_calls: Mutex::new(Vec::new()),
            rate_limit_calls: Mutex::new(Vec::new()),
            rate_limited: Mutex::new(None),
            warns: Mutex::new(Vec::new()),
            access_state: Mutex::new(OwnerAccessState::default()),
            access_calls: Mutex::new(Vec::new()),
            at_limit: Mutex::new(false),
            mcp: Mutex::new(McpVerify::Value(None)),
            mcp_calls: Mutex::new(Vec::new()),
            resolution: Mutex::new(OAuthPrincipalResolution::InvalidToken),
            resolve_calls: Mutex::new(Vec::new()),
            cloud: true,
        }
    }
}

impl MockDeps {
    pub fn with_lookup(lookup: Option<WorkspaceTokenLookup>) -> MockDeps {
        let deps = MockDeps::default();
        *deps.lookup.lock().unwrap() = Lookup::Value(lookup);
        deps
    }

    pub fn find_call_count(&self) -> usize {
        self.find_calls.lock().unwrap().len()
    }

    pub fn rate_limit_calls(&self) -> Vec<(String, String, Option<u64>)> {
        self.rate_limit_calls.lock().unwrap().clone()
    }

    pub fn access_calls(&self) -> Vec<String> {
        self.access_calls.lock().unwrap().clone()
    }

    pub fn warns(&self) -> Vec<(Value, String)> {
        self.warns.lock().unwrap().clone()
    }
}

impl WorkspaceQuota for MockDeps {
    async fn get_access_state(&self, owner_id: &str) -> OwnerAccessState {
        self.access_calls.lock().unwrap().push(owner_id.to_string());
        self.access_state.lock().unwrap().clone()
    }

    async fn is_at_limit(&self, _user_id: &str, _metric: &str) -> bool {
        *self.at_limit.lock().unwrap()
    }
}

impl WorkspaceTokenDeps for MockDeps {
    async fn find_workspace_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<WorkspaceTokenLookup>, ChatbotXException> {
        self.find_calls.lock().unwrap().push(token_hash.to_string());
        match self.lookup.lock().unwrap().clone() {
            Lookup::Value(value) => Ok(value),
            Lookup::Error(error) => Err(error),
            Lookup::ForHash(hash, value) => Ok((hash == token_hash).then_some(value)),
        }
    }

    async fn assert_not_rate_limited(
        &self,
        scope: &str,
        key: &str,
        limit: Option<u64>,
    ) -> Result<(), RateLimited> {
        self.rate_limit_calls
            .lock()
            .unwrap()
            .push((scope.to_string(), key.to_string(), limit));
        match self.rate_limited.lock().unwrap().clone() {
            Some(limited) => Err(limited),
            None => Ok(()),
        }
    }

    fn warn(&self, fields: Value, message: &str) {
        self.warns
            .lock()
            .unwrap()
            .push((fields, message.to_string()));
    }

    fn is_cloud(&self) -> bool {
        self.cloud
    }

    fn mobile_api_audience(&self) -> Option<String> {
        Some("https://builder.test/api".to_string())
    }

    async fn resolve_mobile_principal(
        &self,
        _bearer_token: &str,
        _requested_workspace_id: Option<&str>,
        _method: Option<&str>,
        _path: Option<&str>,
    ) -> Result<MobileWorkspacePrincipal, OrpcError> {
        Err(OrpcError::new("INVALID_CHATBOT_TOKEN", Some(401), None))
    }

    async fn verify_mcp_access_token(
        &self,
        bearer_token: &str,
    ) -> Result<Option<McpAccessToken>, McpTokenVerificationUnavailable> {
        self.mcp_calls
            .lock()
            .unwrap()
            .push(bearer_token.to_string());
        match self.mcp.lock().unwrap().clone() {
            McpVerify::Value(value) => Ok(value),
            McpVerify::Unavailable => Err(McpTokenVerificationUnavailable),
        }
    }

    async fn resolve_oauth_principal(
        &self,
        client_id: &str,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> OAuthPrincipalResolution {
        self.resolve_calls.lock().unwrap().push((
            client_id.to_string(),
            user_id.to_string(),
            workspace_id.map(str::to_string),
        ));
        self.resolution.lock().unwrap().clone()
    }

    fn now_ms(&self) -> i64 {
        1_700_000_000_000
    }
}

/// `authResult(scopes, permission)`: workspace `ws-1` of `owner-1`, token `token-1`.
pub fn auth_result(scopes: Option<&[&str]>, permission: &str) -> WorkspaceTokenLookup {
    WorkspaceTokenLookup {
        workspace: AuthWorkspace {
            id: "ws-1".to_string(),
            name: "Workspace".to_string(),
            owner_id: "owner-1".to_string(),
            scheduled_deletion_at: None,
        },
        api_token: RequestApiToken {
            id: "token-1".to_string(),
            workspace_id: "ws-1".to_string(),
            permission: permission.to_string(),
            scopes: scopes.map(|s| s.iter().map(|x| (*x).to_string()).collect()),
            is_default: false,
        },
    }
}

/// Headers with `Authorization: Bearer <token>` and the client IP the
/// original tests stub (`getGuestClientIp` -> `203.0.113.9`).
pub fn bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
    headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    headers
}

/// The concrete path of a route template (`{param}` -> `1`).
pub fn concrete_path(template: &str) -> String {
    template
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "1"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Status and JSON body (`Value::Null` when the body is not JSON).
pub struct Answer {
    pub status: u16,
    pub body: Value,
}

impl Answer {
    pub fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or_default()
    }

    pub fn message(&self) -> &str {
        self.body["message"].as_str().unwrap_or_default()
    }

    /// The request passed auth: neither 401 nor 403, and below 500.
    pub fn passed_auth(&self) -> bool {
        self.status != 401 && self.status != 403 && self.status < 500
    }
}

/// Send `method path` (under `/api`) with the bearer token to the public API.
pub async fn send(deps: &MockDeps, method: &str, path: &str, headers: &HeaderMap) -> Answer {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let uri: Uri = format!("/api{path}").parse().unwrap();
    let response = handle_public_request(deps, &method, &uri, headers).await;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, body }
}

/// Call operation `id` (e.g. `ads.listRules`) with `Bearer TOKEN`.
pub async fn call_op(deps: &MockDeps, id: &str) -> Answer {
    let route = route_by_id(id).unwrap_or_else(|| panic!("no route {id}"));
    send(
        deps,
        route.method,
        &concrete_path(route.path),
        &bearer(TOKEN),
    )
    .await
}

/// The scopes of a workspace-token operation.
pub fn op_scopes(id: &str) -> &'static [&'static str] {
    match route_by_id(id)
        .unwrap_or_else(|| panic!("no route {id}"))
        .auth
    {
        RouteAuth::WorkspaceToken(scopes) => scopes,
        other => panic!("{id} is not a workspace-token route: {other:?}"),
    }
}
