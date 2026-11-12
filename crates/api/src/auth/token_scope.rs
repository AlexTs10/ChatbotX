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
    const fn entry(label_key: &'static str, order: usize) -> ScopeRegistryEntry {
        ScopeRegistryEntry { label_key, order }
    }
    const REGISTRY: &[(&str, ScopeRegistryEntry)] = &[
        ("contacts", entry("fields.tokenScopes.contacts", 0)),
        ("inbox", entry("fields.tokenScopes.inbox", 1)),
        ("automation", entry("fields.tokenScopes.automation", 2)),
        ("broadcasts", entry("fields.tokenScopes.broadcasts", 3)),
        ("analytics", entry("fields.tokenScopes.analytics", 4)),
        ("ecommerce", entry("fields.tokenScopes.ecommerce", 5)),
        ("channels", entry("fields.tokenScopes.channels", 6)),
        ("integrations", entry("fields.tokenScopes.integrations", 7)),
        ("minigames", entry("fields.tokenScopes.minigames", 8)),
        ("appointments", entry("fields.tokenScopes.appointments", 9)),
        ("media", entry("fields.tokenScopes.media", 10)),
        ("ads", entry("fields.tokenScopes.ads", 11)),
        ("settings", entry("fields.tokenScopes.settings", 12)),
        ("mini-apps", entry("fields.tokenScopes.miniApps", 13)),
        ("leaderboards", entry("fields.tokenScopes.leaderboards", 14)),
        ("pipelines", entry("fields.tokenScopes.pipelines", 15)),
        ("places", entry("fields.tokenScopes.places", 16)),
        ("polls", entry("fields.tokenScopes.polls", 17)),
        ("templates", entry("fields.tokenScopes.templates", 18)),
    ];
    REGISTRY
}

/// `orderedWorkspaceApiTokenScopes`.
pub fn ordered_workspace_api_token_scopes() -> Vec<&'static str> {
    let mut entries: Vec<(&'static str, usize)> = workspace_api_token_scope_registry()
        .iter()
        .map(|(scope, entry)| (*scope, entry.order))
        .collect();
    entries.sort_by_key(|(_, order)| *order);
    entries.into_iter().map(|(scope, _)| scope).collect()
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
    scopes.is_none_or(|scopes| {
        required
            .iter()
            .any(|candidate| scopes.iter().any(|s| s == candidate))
    })
}

/// `assertTokenScope(context, scope)`: a caller without a token passes.
pub fn assert_token_scope(
    api_token: Option<&RequestApiToken>,
    scope: &str,
) -> Result<(), OrpcError> {
    let scopes = api_token.and_then(|token| token.scopes.as_deref());
    if token_has_any_scope(scopes, &[scope]) {
        return Ok(());
    }
    Err(OrpcError::new(
        "FORBIDDEN",
        None,
        Some(&format!("Token is not authorized for the '{scope}' scope")),
    ))
}
