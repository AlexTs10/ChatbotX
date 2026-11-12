//! `apps/builder/src/lib/public-api/token-scope.ts` and the scope registry of
//! `apps/builder/src/features/workspaces/lib/workspace-token-scopes.ts`.

pub use chatbotx_db::repo::workspace_api_token::WORKSPACE_API_TOKEN_SCOPES;

use crate::orpc::OrpcError;

/// One scope of the UI registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeRegistryEntry {
    pub label_key: &'static str,
    pub order: usize,
}

/// `workspaceApiTokenScopeRegistry`: every scope with its label key and order.
pub fn workspace_api_token_scope_registry() -> &'static [(&'static str, ScopeRegistryEntry)] {
    todo!("P3.5a")
}

/// `orderedWorkspaceApiTokenScopes`.
pub fn ordered_workspace_api_token_scopes() -> Vec<&'static str> {
    todo!("P3.5a")
}

/// The token of a request (`RequestApiToken` of `@/middlewares/context`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestApiToken {
    pub id: String,
    pub workspace_id: String,
    /// `full` | `read_only`.
    pub permission: String,
    /// `None` = all scopes.
    pub scopes: Option<Vec<String>>,
    pub is_default: bool,
}

/// `tokenHasAnyScope(scopes, required)`: `None` scopes mean all scopes; any-of.
pub fn token_has_any_scope(scopes: Option<&[String]>, required: &[&str]) -> bool {
    let _ = (scopes, required);
    todo!("P3.5a")
}

/// `assertTokenScope(context, scope)`: a caller without a token passes.
pub fn assert_token_scope(
    api_token: Option<&RequestApiToken>,
    scope: &str,
) -> Result<(), OrpcError> {
    let _ = (api_token, scope);
    todo!("P3.5a")
}
