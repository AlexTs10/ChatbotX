//! `apps/builder/src/lib/workspace/authorize-workspace-access.ts`.

use crate::errors::ChatbotXException;
use crate::orpc::OrpcError;

/// `ADS_CAMPAIGNS_INSIGHTS_PATH`.
pub const ADS_CAMPAIGNS_INSIGHTS_PATH: &str = "/v1/ads/campaigns/insights";
/// `BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH`.
pub const BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH: &str = "/v1/broadcasts/audience/preview";
/// `FILTER_VALUE_LABELS_POST_PATH`.
pub const FILTER_VALUE_LABELS_POST_PATH: &str =
    "/workspaces/{workspaceId}/contact-filter/value-labels";
/// `FILTER_VALUE_LABELS_TOKEN_PATH`.
pub const FILTER_VALUE_LABELS_TOKEN_PATH: &str = "/v1/contacts/filter-value-labels";
/// `CONVERSATIONS_LIST_POST_PATH`.
pub const CONVERSATIONS_LIST_POST_PATH: &str = "/workspaces/{workspaceId}/conversations/list";
/// `BROADCAST_STOP_TOKEN_PATH`.
pub const BROADCAST_STOP_TOKEN_PATH: &str = "/v1/broadcasts/{id}/stop";
/// `MARKETING_MESSAGE_STOP_TOKEN_PATH`.
pub const MARKETING_MESSAGE_STOP_TOKEN_PATH: &str = "/v1/marketing-messages/campaigns/{id}/stop";

/// `WorkspaceAccessDenialReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAccessDenialReason {
    TrialExpired,
    MacLimitReached,
}

impl WorkspaceAccessDenialReason {
    /// The code (`trialExpired` | `macLimitReached`).
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceAccessDenialReason::TrialExpired => "trialExpired",
            WorkspaceAccessDenialReason::MacLimitReached => "macLimitReached",
        }
    }

    /// `DENIAL_MESSAGES[reason]`.
    pub fn message(self) -> &'static str {
        match self {
            WorkspaceAccessDenialReason::TrialExpired => "Trial expired",
            WorkspaceAccessDenialReason::MacLimitReached => "Monthly active contact limit reached",
        }
    }
}

/// `userQuotaService.getAccessState(ownerId)` (the fields the gate reads).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnerAccessState {
    pub blocked: bool,
    /// `status` | `mac` | `None`.
    pub reason: Option<String>,
}

/// The quota services the owner gate calls.
pub trait WorkspaceQuota {
    /// `userQuotaService.getAccessState(ownerId)`.
    fn get_access_state(&self, owner_id: &str) -> impl Future<Output = OwnerAccessState> + Send;
    /// `quotaEnforcementService.isAtLimit({ userId, metric })`.
    fn is_at_limit(&self, user_id: &str, metric: &str) -> impl Future<Output = bool> + Send;
}

/// `isWorkspaceMutationMethod(method)`: everything but GET/HEAD/DELETE (no method = POST).
pub fn is_workspace_mutation_method(method: Option<&str>) -> bool {
    !matches!(method.unwrap_or("POST"), "GET" | "HEAD" | "DELETE")
}

/// `isReadOnlyTokenAllowedMethod(method, path)`.
pub fn is_read_only_token_allowed_method(method: Option<&str>, path: Option<&str>) -> bool {
    matches!(method.unwrap_or("POST"), "GET" | "HEAD")
        || (method == Some("POST")
            && READ_ONLY_TOKEN_ALLOWED_POST_PATHS.contains(&path.unwrap_or("")))
}

/// `checkWorkspaceOwnerAccess({ ownerId })`; `is_cloud` is `isCloud()`.
pub async fn check_workspace_owner_access<Q: WorkspaceQuota + Sync>(
    quota: &Q,
    is_cloud: bool,
    owner_id: &str,
) -> Option<WorkspaceAccessDenialReason> {
    if !is_cloud {
        return None;
    }
    let state = owner_access_state(quota, owner_id).await;
    if !state.blocked {
        return None;
    }
    Some(if state.reason.as_deref() == Some("mac") {
        WorkspaceAccessDenialReason::MacLimitReached
    } else {
        WorkspaceAccessDenialReason::TrialExpired
    })
}

/// `workspaceAccessDenialException(reason)`.
pub fn workspace_access_denial_exception(reason: WorkspaceAccessDenialReason) -> ChatbotXException {
    ChatbotXException::with_code(reason.message(), reason.as_str(), DENIAL_HTTP_STATUS)
}

/// `workspaceAccessDenialOrpcError(reason)`.
pub fn workspace_access_denial_orpc_error(reason: WorkspaceAccessDenialReason) -> OrpcError {
    OrpcError::new(
        reason.as_str(),
        Some(DENIAL_HTTP_STATUS),
        Some(reason.message()),
    )
}

/// `assertWorkspaceOwnerAccessForMethod({ method, ownerId, path })`.
pub async fn assert_workspace_owner_access_for_method<Q: WorkspaceQuota + Sync>(
    quota: &Q,
    is_cloud: bool,
    method: Option<&str>,
    owner_id: &str,
    path: Option<&str>,
) -> Result<(), OrpcError> {
    let read_only_post = method == Some("POST")
        && path.is_some_and(|p| !p.is_empty() && READ_ONLY_POST_PATHS.contains(&p));
    if !is_workspace_mutation_method(method) || read_only_post {
        return Ok(());
    }
    match check_workspace_owner_access(quota, is_cloud, owner_id).await {
        Some(reason) => Err(workspace_access_denial_orpc_error(reason)),
        None => Ok(()),
    }
}

const DENIAL_HTTP_STATUS: u16 = 403;

/// POST routes a `read_only` token may call (reads that take a body).
const READ_ONLY_TOKEN_ALLOWED_POST_PATHS: &[&str] = &[
    ADS_CAMPAIGNS_INSIGHTS_PATH,
    FILTER_VALUE_LABELS_TOKEN_PATH,
    "/v1/templates/dependencies-preview",
    "/v1/templates/locked-resources",
];

/// POST routes the owner quota gate lets through (reads, and stops/uninstalls the UI allows).
const READ_ONLY_POST_PATHS: &[&str] = &[
    CONVERSATIONS_LIST_POST_PATH,
    FILTER_VALUE_LABELS_POST_PATH,
    FILTER_VALUE_LABELS_TOKEN_PATH,
    BROADCAST_STOP_TOKEN_PATH,
    MARKETING_MESSAGE_STOP_TOKEN_PATH,
    BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH,
    "/v1/template-installations/{id}/uninstall",
    "/v1/template-installations/{id}/uninstall/cancel",
    "/v1/templates/{id}/installers/{installerId}/uninstall",
];

/// The owner's access state, with the MAC pool limit folded in.
async fn owner_access_state<Q: WorkspaceQuota + Sync>(
    quota: &Q,
    owner_id: &str,
) -> OwnerAccessState {
    let state = quota.get_access_state(owner_id).await;
    if state.blocked {
        return state;
    }
    if quota.is_at_limit(owner_id, "mac").await {
        return OwnerAccessState {
            blocked: true,
            reason: Some("mac".to_string()),
        };
    }
    state
}
