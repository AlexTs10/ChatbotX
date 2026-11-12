//! Port of `apps/builder/__tests__/workspace-token-auth-middleware.test.ts`.

mod support;

use axum::http::{HeaderMap, HeaderValue};
use chatbotx_api::auth::{
    AuditContext, AuthWorkspace, McpAccessToken, OAuthPrincipalResolution, OwnerAccessState,
    RequestApiToken, TokenAuthRequest, WorkspaceChoice, WorkspaceTokenContext,
    authenticate_workspace_token, hash_token,
};
use chatbotx_api::errors::ChatbotXException;
use chatbotx_api::orpc::OrpcError;
use chatbotx_kv::rate_limit::RateLimited;
use serde_json::json;
use support::{Lookup, McpVerify, MockDeps, auth_result, bearer};

async fn call_middleware(
    deps: &MockDeps,
    headers: &HeaderMap,
    method: Option<&str>,
    url: Option<&str>,
    path: Option<&str>,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let mut res_headers = HeaderMap::new();
    authenticate_workspace_token(
        deps,
        TokenAuthRequest {
            headers,
            url,
            method,
            path,
        },
        &mut res_headers,
    )
    .await
}

fn oauth_workspace() -> AuthWorkspace {
    AuthWorkspace {
        id: "ws-oauth".to_string(),
        name: "OAuth".to_string(),
        owner_id: "owner-oauth".to_string(),
        scheduled_deletion_at: None,
    }
}

fn oauth_verified(deps: &MockDeps) {
    *deps.mcp.lock().unwrap() = McpVerify::Value(Some(McpAccessToken {
        client_id: "client-oauth".to_string(),
        user_id: "user-oauth".to_string(),
    }));
}

fn blocked(deps: &MockDeps) {
    *deps.access_state.lock().unwrap() = OwnerAccessState {
        blocked: true,
        reason: Some("status".to_string()),
    };
}

#[tokio::test]
async fn hash_hit_authenticates_the_workspace_and_does_not_warn() {
    let token = "ws1_abc";
    let deps = MockDeps::default();
    *deps.lookup.lock().unwrap() = Lookup::ForHash(hash_token(token), auth_result(None, "full"));

    let result = call_middleware(&deps, &bearer(token), Some("GET"), None, None).await;

    assert_eq!(deps.find_call_count(), 1);
    assert!(deps.warns().is_empty());
    assert!(result.is_ok());
    assert_eq!(
        deps.rate_limit_calls(),
        vec![
            (
                "workspace-token-preauth-rate-limit".to_string(),
                "203.0.113.9".to_string(),
                Some(600)
            ),
            (
                "workspace-token-rate-limit".to_string(),
                "ws-1".to_string(),
                None
            ),
        ]
    );
}

#[tokio::test]
async fn stale_cached_token_pointing_at_a_purged_workspace_invalid_chatbot_token_not_a_raw_notfound()
 {
    let deps = MockDeps::default();
    *deps.lookup.lock().unwrap() = Lookup::Error(ChatbotXException::with_code(
        "Workspace not found",
        "notFound",
        400,
    ));

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
}

#[tokio::test]
async fn hash_miss_invalid_chatbot_token_no_plaintext_fallback() {
    let deps = MockDeps::with_lookup(None);

    let error = call_middleware(&deps, &bearer("nope"), Some("GET"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
    assert_eq!(deps.find_call_count(), 1);
    let calls = deps.rate_limit_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "workspace-token-preauth-rate-limit");
}

#[tokio::test]
async fn get_procedure_with_a_blocked_owner_still_passes() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    blocked(&deps);

    let result = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None).await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn post_procedure_with_a_blocked_owner_is_rejected_with_trialexpired_403() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    blocked(&deps);

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("POST"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "trialExpired");
    assert_eq!(error.status, 403);
}

#[tokio::test]
async fn allows_the_conversations_list_post_path_for_a_trial_expired_workspace() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    blocked(&deps);

    let result = call_middleware(
        &deps,
        &bearer("ws1_abc"),
        Some("POST"),
        None,
        Some("/workspaces/{workspaceId}/conversations/list"),
    )
    .await;

    assert!(result.is_ok());
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn delete_procedure_with_a_blocked_owner_still_passes_invariant_14() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    blocked(&deps);

    let result = call_middleware(&deps, &bearer("ws1_abc"), Some("DELETE"), None, None).await;

    assert!(result.is_ok());
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn scheduled_deletion_workspace_is_rejected_with_forbidden() {
    let mut lookup = auth_result(None, "full");
    lookup.workspace.scheduled_deletion_at = Some(1_700_000_000_000);
    let deps = MockDeps::with_lookup(Some(lookup));

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(error.message, "Workspace deletion scheduled");
}

#[tokio::test]
async fn rate_limited_workspace_token_is_rejected_before_the_owner_access_gate() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));
    *deps.rate_limited.lock().unwrap() = Some(RateLimited {
        code: "tooManyRequests",
        http_status: 429,
        retry_after: 5,
    });

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "tooManyRequests");
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn query_param_token_warns_about_the_deprecated_token_usage() {
    let token = "ws1_abc";
    let deps = MockDeps::default();
    *deps.lookup.lock().unwrap() = Lookup::ForHash(hash_token(token), auth_result(None, "full"));

    let url = format!("https://example.com/api/v1/tags?token={token}");
    let _ = call_middleware(&deps, &HeaderMap::new(), Some("GET"), Some(&url), None).await;

    assert!(deps.warns().contains(&(
        json!({ "path": "/api/v1/tags" }),
        "Workspace token authenticated via deprecated ?token= query param".to_string()
    )));
}

#[tokio::test]
async fn read_only_token_on_a_mutation_is_rejected_with_forbidden_before_the_owner_quota_gate() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "read_only")));

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("POST"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Read-only token cannot perform this operation"
    );
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn read_only_token_on_a_get_request_passes() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "read_only")));

    let result = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None).await;

    assert!(result.is_ok());
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn read_only_token_on_a_delete_request_is_rejected_with_forbidden() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "read_only")));

    let error = call_middleware(&deps, &bearer("ws1_abc"), Some("DELETE"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Read-only token cannot perform this operation"
    );
    assert!(deps.access_calls().is_empty());
}

#[tokio::test]
async fn full_permission_token_on_a_mutation_reaches_the_owner_quota_gate() {
    let deps = MockDeps::with_lookup(Some(auth_result(None, "full")));

    let result = call_middleware(&deps, &bearer("ws1_abc"), Some("POST"), None, None).await;

    assert_eq!(deps.access_calls(), vec!["owner-1".to_string()]);
    assert!(result.is_ok());
}

#[tokio::test]
async fn forwards_the_authenticated_token_and_its_credential_identity_downstream() {
    let auth = auth_result(None, "full");
    let deps = MockDeps::with_lookup(Some(auth.clone()));

    let context = call_middleware(&deps, &bearer("ws1_abc"), Some("GET"), None, None)
        .await
        .unwrap();

    assert_eq!(context.workspace, auth.workspace);
    assert_eq!(
        context.api_token,
        RequestApiToken {
            id: "token-1".to_string(),
            workspace_id: "ws-1".to_string(),
            permission: "full".to_string(),
            scopes: None,
            is_default: false,
        }
    );
    assert_eq!(context.api_credential_id, "api-token:token-1");
    assert_eq!(context.mobile, None);
}

#[tokio::test]
async fn authenticates_an_oauth_principal_with_user_attribution_and_forwards_x_workspace_id() {
    let deps = MockDeps::with_lookup(None);
    oauth_verified(&deps);
    *deps.resolution.lock().unwrap() = OAuthPrincipalResolution::Principal {
        workspace: oauth_workspace(),
        user_id: "user-oauth".to_string(),
        client_id: "client-oauth".to_string(),
    };
    let mut headers = bearer("header.payload.signature");
    headers.insert("user-agent", HeaderValue::from_static("MCP Inspector"));
    headers.insert("x-workspace-id", HeaderValue::from_static("ws-oauth"));

    let context = call_middleware(&deps, &headers, Some("GET"), None, None)
        .await
        .unwrap();

    assert_eq!(
        deps.resolve_calls.lock().unwrap().clone(),
        vec![(
            "client-oauth".to_string(),
            "user-oauth".to_string(),
            Some("ws-oauth".to_string())
        )]
    );
    assert_eq!(context.workspace, oauth_workspace());
    assert_eq!(
        context.api_token,
        RequestApiToken {
            id: "oauth:client-oauth:user-oauth".to_string(),
            workspace_id: "ws-oauth".to_string(),
            permission: "full".to_string(),
            scopes: None,
            is_default: false,
        }
    );
    assert_eq!(context.api_credential_id, "mcp-oauth:client-oauth");
    assert_eq!(
        context.audit,
        AuditContext {
            user_id: "user-oauth".to_string(),
            workspace_id: "ws-oauth".to_string(),
            ip_address: "203.0.113.9".to_string(),
            user_agent: Some("MCP Inspector".to_string()),
            source: "mcp-oauth:client-oauth".to_string(),
        }
    );
}

#[tokio::test]
async fn maps_a_forbidden_oauth_principal_to_403() {
    let deps = MockDeps::with_lookup(None);
    oauth_verified(&deps);
    *deps.resolution.lock().unwrap() = OAuthPrincipalResolution::Forbidden;

    let error = call_middleware(
        &deps,
        &bearer("header.payload.signature"),
        Some("GET"),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(error.status, 403);
}

#[tokio::test]
async fn maps_oauth_workspace_selection_to_a_documented_400() {
    let deps = MockDeps::with_lookup(None);
    oauth_verified(&deps);
    *deps.resolution.lock().unwrap() =
        OAuthPrincipalResolution::WorkspaceSelectionRequired(vec![WorkspaceChoice {
            id: "ws-1".to_string(),
            name: "Alpha".to_string(),
        }]);

    let error = call_middleware(
        &deps,
        &bearer("header.payload.signature"),
        Some("GET"),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "WORKSPACE_SELECTION_REQUIRED");
    assert_eq!(error.status, 400);
    assert!(error.message.contains("ws-1 (Alpha)"), "{}", error.message);
}

#[tokio::test]
async fn maps_an_invalid_oauth_token_to_401() {
    let deps = MockDeps::with_lookup(None);
    *deps.resolution.lock().unwrap() = OAuthPrincipalResolution::InvalidToken;

    let error = call_middleware(
        &deps,
        &bearer("header.payload.signature"),
        Some("GET"),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
}

#[tokio::test]
async fn rejects_a_non_jwt_bearer_without_verifying_it_as_an_oauth_token() {
    let deps = MockDeps::with_lookup(None);

    let error = call_middleware(&deps, &bearer("revoked-api-key"), Some("GET"), None, None)
        .await
        .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
    assert!(deps.mcp_calls.lock().unwrap().is_empty());
    assert!(deps.resolve_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn maps_a_token_the_verifier_rejects_to_401() {
    let deps = MockDeps::with_lookup(None);
    *deps.mcp.lock().unwrap() = McpVerify::Value(None);

    let error = call_middleware(
        &deps,
        &bearer("header.payload.signature"),
        Some("GET"),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
    assert!(deps.resolve_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn maps_a_jwks_outage_to_503_instead_of_500() {
    let deps = MockDeps::with_lookup(None);
    *deps.mcp.lock().unwrap() = McpVerify::Unavailable;

    let error = call_middleware(
        &deps,
        &bearer("header.payload.signature"),
        Some("GET"),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "SERVICE_UNAVAILABLE");
    assert_eq!(error.status, 503);
}

#[tokio::test]
async fn never_sends_a_query_string_workspace_token_to_oauth_resolution() {
    let deps = MockDeps::with_lookup(None);

    let error = call_middleware(
        &deps,
        &HeaderMap::new(),
        Some("GET"),
        Some("https://example.com/api/v1/tags?token=legacy-workspace-token"),
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
    assert!(deps.resolve_calls.lock().unwrap().is_empty());
}
