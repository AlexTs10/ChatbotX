//! Port of `apps/builder/__tests__/workspace-token-scope-enforcement.test.ts`.
//!
//! `buildProcedure(scope, method)` is a route `/v1/test-<scope>` behind
//! `workspaceTokenAuthAPIForScope(scope)`; here it is
//! `authorize_workspace_token_for_scope` with that method and path.

mod support;

use axum::http::HeaderMap;
use chatbotx_api::auth::{
    TokenAuthRequest, WorkspaceTokenContext, authorize_workspace_token_for_scope,
};
use chatbotx_api::orpc::OrpcError;
use chatbotx_api::routes::{PUBLIC_ROUTES, routes_of_module};
use support::{MockDeps, TOKEN, auth_result, bearer};

async fn invoke(
    deps: &MockDeps,
    scope: &str,
    method: &str,
) -> Result<WorkspaceTokenContext, OrpcError> {
    let path = format!("/v1/test-{scope}");
    let headers = bearer(TOKEN);
    let mut res_headers = HeaderMap::new();
    authorize_workspace_token_for_scope(
        deps,
        TokenAuthRequest {
            headers: &headers,
            url: None,
            method: Some(method),
            path: Some(&path),
        },
        &[scope],
        &mut res_headers,
    )
    .await
}

fn deps(scopes: Option<&[&str]>, permission: &str) -> MockDeps {
    MockDeps::with_lookup(Some(auth_result(scopes, permission)))
}

#[tokio::test]
async fn null_scopes_unrestricted_passes_every_scoped_route() {
    let deps = deps(None, "full");

    let contacts = invoke(&deps, "contacts", "GET").await.unwrap();
    let broadcasts = invoke(&deps, "broadcasts", "GET").await.unwrap();

    assert_eq!(contacts.workspace.id, "ws-1");
    assert_eq!(broadcasts.workspace.id, "ws-1");
}

#[tokio::test]
async fn a_token_scoped_to_contacts_passes_the_contacts_scoped_route() {
    let deps = deps(Some(&["contacts"]), "full");

    assert!(invoke(&deps, "contacts", "GET").await.is_ok());
}

#[tokio::test]
async fn a_token_scoped_to_contacts_is_denied_a_broadcasts_scoped_route_with_forbidden() {
    let deps = deps(Some(&["contacts"]), "full");

    let error = invoke(&deps, "broadcasts", "GET").await.unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Token is not authorized for the 'broadcasts' scope"
    );
}

#[tokio::test]
async fn a_mixed_scope_contact_router_a_contacts_scoped_token_can_hit_the_contacts_scoped_procedure_but_not_the_inbox_scoped_one()
 {
    let deps = deps(Some(&["contacts"]), "full");

    assert!(invoke(&deps, "contacts", "POST").await.is_ok());
    let error = invoke(&deps, "inbox", "POST").await.unwrap_err();
    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Token is not authorized for the 'inbox' scope"
    );
}

#[tokio::test]
async fn an_inbox_scoped_token_can_hit_the_inbox_scoped_procedure_but_not_the_contacts_scoped_one()
{
    let deps = deps(Some(&["inbox"]), "full");

    assert!(invoke(&deps, "inbox", "POST").await.is_ok());
    let error = invoke(&deps, "contacts", "POST").await.unwrap_err();
    assert_eq!(error.code, "FORBIDDEN");
}

#[tokio::test]
async fn a_token_without_the_automation_scope_is_denied_a_new_automation_write_route_with_forbidden()
 {
    let deps = deps(Some(&["contacts"]), "full");

    let error = invoke(&deps, "automation", "POST").await.unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Token is not authorized for the 'automation' scope"
    );
}

#[tokio::test]
async fn an_automation_scoped_token_can_hit_an_automation_write_route_e_g_post_v1_flows_post_v1_ai_functions()
 {
    let deps = deps(Some(&["automation"]), "full");

    assert!(invoke(&deps, "automation", "POST").await.is_ok());
}

#[tokio::test]
async fn scope_enforcement_runs_after_the_read_only_permission_gate_a_read_only_token_is_still_blocked_from_a_mutation_regardless_of_scope()
 {
    let deps = deps(Some(&["contacts"]), "read_only");

    let error = invoke(&deps, "contacts", "POST").await.unwrap_err();

    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Read-only token cannot perform this operation"
    );
}

#[tokio::test]
async fn scope_enforcement_does_not_block_a_read_only_token_s_in_scope_get() {
    let deps = deps(Some(&["contacts"]), "read_only");

    assert!(invoke(&deps, "contacts", "GET").await.is_ok());
}

#[tokio::test]
async fn an_invalid_token_is_rejected_before_scope_is_ever_considered() {
    let deps = MockDeps::with_lookup(None);

    let error = invoke(&deps, "contacts", "GET").await.unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
}

#[tokio::test]
async fn an_invalid_token_error_is_upgraded_to_defined_true_once_commonapierrors_is_attached() {
    let deps = MockDeps::with_lookup(None);

    let error = invoke(&deps, "contacts", "GET").await.unwrap_err();

    assert_eq!(error.code, "INVALID_CHATBOT_TOKEN");
    assert_eq!(error.status, 401);
    assert!(error.defined);
}

#[test]
fn the_public_router_no_longer_imports_or_mounts_the_workspace_token_workspace_fetch_router() {
    assert!(routes_of_module("features/workspaces/api/workspace-token").is_empty());
    assert!(
        PUBLIC_ROUTES
            .iter()
            .all(|route| !(route.method == "GET" && route.path == "/v1/workspaces")),
        "GET /v1/workspaces is gone from the public API"
    );
    assert!(
        PUBLIC_ROUTES
            .iter()
            .all(|route| !route.id.starts_with("workspaces."))
    );
}
