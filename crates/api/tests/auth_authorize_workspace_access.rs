//! Port of `apps/builder/__tests__/authorize-workspace-access.test.ts`.
//!
//! The mocked `userQuotaService` / `quotaEnforcementService` are a fake
//! `WorkspaceQuota`; `isCloud()` is the `is_cloud` argument.

use std::sync::Mutex;

use chatbotx_api::auth::{
    ADS_CAMPAIGNS_INSIGHTS_PATH, CONVERSATIONS_LIST_POST_PATH, FILTER_VALUE_LABELS_POST_PATH,
    OwnerAccessState, WorkspaceAccessDenialReason, WorkspaceQuota,
    assert_workspace_owner_access_for_method, check_workspace_owner_access,
    is_read_only_token_allowed_method, is_workspace_mutation_method,
    workspace_access_denial_exception, workspace_access_denial_orpc_error,
};

#[derive(Default)]
struct FakeQuota {
    access_state: OwnerAccessState,
    at_limit: bool,
    access_calls: Mutex<Vec<String>>,
    limit_calls: Mutex<Vec<(String, String)>>,
}

impl FakeQuota {
    fn blocked(reason: &str) -> FakeQuota {
        FakeQuota {
            access_state: OwnerAccessState {
                blocked: true,
                reason: Some(reason.to_string()),
            },
            ..FakeQuota::default()
        }
    }

    fn open(at_limit: bool) -> FakeQuota {
        FakeQuota {
            at_limit,
            ..FakeQuota::default()
        }
    }

    fn access_calls(&self) -> usize {
        self.access_calls.lock().unwrap().len()
    }
}

impl WorkspaceQuota for FakeQuota {
    async fn get_access_state(&self, owner_id: &str) -> OwnerAccessState {
        self.access_calls.lock().unwrap().push(owner_id.to_string());
        self.access_state.clone()
    }

    async fn is_at_limit(&self, user_id: &str, metric: &str) -> bool {
        self.limit_calls
            .lock()
            .unwrap()
            .push((user_id.to_string(), metric.to_string()));
        self.at_limit
    }
}

// --- checkWorkspaceOwnerAccess ---

#[tokio::test]
async fn checkworkspaceowneraccess_returns_null_on_self_hosted_non_cloud_regardless_of_quota_state()
{
    let quota = FakeQuota::blocked("status");

    let result = check_workspace_owner_access(&quota, false, "owner-1").await;

    assert_eq!(result, None);
    assert_eq!(quota.access_calls(), 0);
}

#[tokio::test]
async fn checkworkspaceowneraccess_returns_trialexpired_when_the_owner_s_access_state_is_blocked_for_a_non_mac_reason()
 {
    let quota = FakeQuota::blocked("status");

    let result = check_workspace_owner_access(&quota, true, "owner-1").await;

    assert_eq!(result, Some(WorkspaceAccessDenialReason::TrialExpired));
}

#[tokio::test]
async fn checkworkspaceowneraccess_returns_maclimitreached_when_already_blocked_for_the_mac_reason()
{
    let quota = FakeQuota::blocked("mac");

    let result = check_workspace_owner_access(&quota, true, "owner-1").await;

    assert_eq!(result, Some(WorkspaceAccessDenialReason::MacLimitReached));
    assert!(quota.limit_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn checkworkspaceowneraccess_returns_maclimitreached_when_not_blocked_by_status_but_the_pool_is_at_the_mac_limit()
 {
    let quota = FakeQuota::open(true);

    let result = check_workspace_owner_access(&quota, true, "owner-1").await;

    assert_eq!(result, Some(WorkspaceAccessDenialReason::MacLimitReached));
    assert_eq!(
        *quota.limit_calls.lock().unwrap(),
        vec![("owner-1".to_string(), "mac".to_string())]
    );
}

#[tokio::test]
async fn checkworkspaceowneraccess_returns_null_when_not_blocked_and_under_the_mac_limit() {
    let quota = FakeQuota::open(false);

    let result = check_workspace_owner_access(&quota, true, "owner-1").await;

    assert_eq!(result, None);
}

// --- assertWorkspaceOwnerAccessForMethod ---

#[tokio::test]
async fn assertworkspaceowneraccessformethod_allows_the_conversation_list_post_but_denies_other_post_routes()
 {
    let quota = FakeQuota::blocked("status");

    let allowed = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some(CONVERSATIONS_LIST_POST_PATH),
    )
    .await;
    assert!(allowed.is_ok());
    assert_eq!(quota.access_calls(), 0);

    let denied = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some("/workspaces/{workspaceId}/conversations/assign"),
    )
    .await;
    assert_eq!(denied.unwrap_err().code, "trialExpired");
}

// --- isWorkspaceMutationMethod ---

#[test]
fn isworkspacemutationmethod_method_s_mutation_s() {
    for (method, expected) in [
        (Some("GET"), false),
        (Some("HEAD"), false),
        (Some("DELETE"), false),
        (Some("POST"), true),
        (Some("PUT"), true),
        (Some("PATCH"), true),
        (None, true),
    ] {
        assert_eq!(is_workspace_mutation_method(method), expected, "{method:?}");
    }
}

// --- isReadOnlyTokenAllowedMethod ---

#[test]
fn isreadonlytokenallowedmethod_allows_get_head_regardless_of_path() {
    assert!(is_read_only_token_allowed_method(Some("GET"), None));
    assert!(is_read_only_token_allowed_method(Some("HEAD"), None));
}

#[test]
fn isreadonlytokenallowedmethod_allows_post_to_the_allowlisted_ads_campaigns_insights_path() {
    assert!(is_read_only_token_allowed_method(
        Some("POST"),
        Some(ADS_CAMPAIGNS_INSIGHTS_PATH)
    ));
}

#[test]
fn isreadonlytokenallowedmethod_allows_post_to_the_template_dependency_and_locked_resource_previews()
 {
    assert!(is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/templates/dependencies-preview")
    ));
    assert!(is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/templates/locked-resources")
    ));
    assert!(!is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/templates/save")
    ));
}

#[test]
fn isreadonlytokenallowedmethod_allows_post_to_the_public_filter_value_labels_read_and_not_its_put()
{
    assert!(is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/contacts/filter-value-labels")
    ));
    assert!(!is_read_only_token_allowed_method(
        Some("PUT"),
        Some("/v1/contacts/filter-value-labels")
    ));
}

#[test]
fn isreadonlytokenallowedmethod_does_not_let_a_read_only_token_run_the_broadcast_audience_preview()
{
    assert!(!is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/broadcasts/audience/preview")
    ));
}

#[test]
fn isreadonlytokenallowedmethod_rejects_post_put_patch_to_every_other_path() {
    assert!(!is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/ads/campaigns")
    ));
    assert!(!is_read_only_token_allowed_method(
        Some("PUT"),
        Some(ADS_CAMPAIGNS_INSIGHTS_PATH)
    ));
    assert!(!is_read_only_token_allowed_method(
        Some("PATCH"),
        Some(ADS_CAMPAIGNS_INSIGHTS_PATH)
    ));
}

#[test]
fn isreadonlytokenallowedmethod_rejects_post_with_no_path_at_all() {
    assert!(!is_read_only_token_allowed_method(Some("POST"), None));
    assert!(!is_read_only_token_allowed_method(Some("POST"), None));
}

#[test]
fn isreadonlytokenallowedmethod_rejects_post_to_the_conversations_list_path_it_is_allow_listed_for_the_trial_gate_only_not_for_read_only_tokens()
 {
    assert!(!is_read_only_token_allowed_method(
        Some("POST"),
        Some(CONVERSATIONS_LIST_POST_PATH)
    ));
}

// --- assertWorkspaceOwnerAccessForMethod (trial-expired paths) ---

#[tokio::test]
async fn assertworkspaceowneraccessformethod_allows_the_conversations_list_post_path_for_a_trial_expired_workspace()
 {
    let quota = FakeQuota::blocked("status");

    let result = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some("/workspaces/{workspaceId}/conversations/list"),
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn assertworkspaceowneraccessformethod_allows_the_filter_value_labels_post_path_for_a_trial_expired_workspace()
 {
    let quota = FakeQuota::blocked("status");

    let result = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some(FILTER_VALUE_LABELS_POST_PATH),
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn assertworkspaceowneraccessformethod_allows_template_uninstall_s_on_a_trial_expired_workspace_ui_parity()
 {
    for path in [
        "/v1/template-installations/{id}/uninstall",
        "/v1/template-installations/{id}/uninstall/cancel",
        "/v1/templates/{id}/installers/{installerId}/uninstall",
    ] {
        let quota = FakeQuota::blocked("status");

        let result = assert_workspace_owner_access_for_method(
            &quota,
            true,
            Some("POST"),
            "owner-1",
            Some(path),
        )
        .await;

        assert!(result.is_ok(), "{path}");
        // A write: never opened to read_only tokens.
        assert!(!is_read_only_token_allowed_method(Some("POST"), Some(path)));
    }
}

#[tokio::test]
async fn assertworkspaceowneraccessformethod_allows_stopping_a_broadcast_on_a_trial_expired_workspace_ui_parity()
 {
    let quota = FakeQuota::blocked("status");

    let result = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some("/v1/broadcasts/{id}/stop"),
    )
    .await;

    assert!(result.is_ok());
}

#[test]
fn assertworkspaceowneraccessformethod_a_read_only_token_still_cannot_stop_a_broadcast() {
    assert!(!is_read_only_token_allowed_method(
        Some("POST"),
        Some("/v1/broadcasts/{id}/stop")
    ));
}

#[tokio::test]
async fn assertworkspaceowneraccessformethod_blocks_a_different_post_path_for_a_trial_expired_workspace()
 {
    let quota = FakeQuota::blocked("status");

    let result = assert_workspace_owner_access_for_method(
        &quota,
        true,
        Some("POST"),
        "owner-1",
        Some("/workspaces/{workspaceId}/conversations/archive"),
    )
    .await;

    let error = result.unwrap_err();
    assert_eq!(error.code, "trialExpired");
    assert_eq!(error.status, 403);
}

// --- workspaceAccessDenialException ---

#[test]
fn workspaceaccessdenialexception_carries_the_reason_as_the_code_and_a_403_status() {
    let error = workspace_access_denial_exception(WorkspaceAccessDenialReason::TrialExpired);

    assert_eq!(error.code, "trialExpired");
    assert_eq!(error.http_status_code, 403);
}

// --- workspaceAccessDenialOrpcError ---

#[test]
fn workspaceaccessdenialorpcerror_carries_the_reason_as_the_orpcerror_code_and_a_403_status() {
    let error = workspace_access_denial_orpc_error(WorkspaceAccessDenialReason::MacLimitReached);

    assert_eq!(error.code, "macLimitReached");
    assert_eq!(error.status, 403);
}
