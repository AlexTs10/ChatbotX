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
        todo!("P3.5a")
    }

    /// `DENIAL_MESSAGES[reason]`.
    pub fn message(self) -> &'static str {
        todo!("P3.5a")
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
    let _ = method;
    todo!("P3.5a")
}

/// `isReadOnlyTokenAllowedMethod(method, path)`.
pub fn is_read_only_token_allowed_method(method: Option<&str>, path: Option<&str>) -> bool {
    let _ = (method, path);
    todo!("P3.5a")
}

/// `checkWorkspaceOwnerAccess({ ownerId })`; `is_cloud` is `isCloud()`.
pub async fn check_workspace_owner_access<Q: WorkspaceQuota + Sync>(
    quota: &Q,
    is_cloud: bool,
    owner_id: &str,
) -> Option<WorkspaceAccessDenialReason> {
    let _ = (quota, is_cloud, owner_id);
    todo!("P3.5a")
}

/// `workspaceAccessDenialException(reason)`.
pub fn workspace_access_denial_exception(reason: WorkspaceAccessDenialReason) -> ChatbotXException {
    let _ = reason;
    todo!("P3.5a")
}

/// `workspaceAccessDenialOrpcError(reason)`.
pub fn workspace_access_denial_orpc_error(reason: WorkspaceAccessDenialReason) -> OrpcError {
    let _ = reason;
    todo!("P3.5a")
}

/// `assertWorkspaceOwnerAccessForMethod({ method, ownerId, path })`.
pub async fn assert_workspace_owner_access_for_method<Q: WorkspaceQuota + Sync>(
    quota: &Q,
    is_cloud: bool,
    method: Option<&str>,
    owner_id: &str,
    path: Option<&str>,
) -> Result<(), OrpcError> {
    let _ = (quota, is_cloud, method, owner_id, path);
    todo!("P3.5a")
}
