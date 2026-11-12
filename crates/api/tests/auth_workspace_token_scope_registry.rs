//! Port of `apps/builder/__tests__/workspace-token-scope-registry.test.ts`.

use chatbotx_api::auth::{
    WORKSPACE_API_TOKEN_SCOPES, ordered_workspace_api_token_scopes,
    workspace_api_token_scope_registry,
};

const NEW_SCOPES: &[&str] = &[
    "channels",
    "integrations",
    "minigames",
    "appointments",
    "media",
    "ads",
    "settings",
    "mini-apps",
    "leaderboards",
    "pipelines",
    "places",
    "polls",
    "templates",
];

#[test]
fn workspaceapitokenscopes_preserves_distinct_channel_and_integration_resource_area_scopes() {
    for scope in NEW_SCOPES {
        assert!(WORKSPACE_API_TOKEN_SCOPES.contains(scope), "{scope}");
    }
    assert!(!WORKSPACE_API_TOKEN_SCOPES.contains(&"connections"));
    assert_eq!(WORKSPACE_API_TOKEN_SCOPES.len(), 19);
}

#[test]
fn orderedworkspaceapitokenscopes_has_19_entries_with_unique_contiguous_orders() {
    let ordered = ordered_workspace_api_token_scopes();
    assert_eq!(ordered.len(), 19);

    let registry = workspace_api_token_scope_registry();
    let mut orders: Vec<usize> = ordered
        .iter()
        .map(|scope| {
            registry
                .iter()
                .find(|(name, _)| name == scope)
                .expect("registered")
                .1
                .order
        })
        .collect();
    orders.sort_unstable();

    let unique: std::collections::BTreeSet<usize> = orders.iter().copied().collect();
    assert_eq!(unique.len(), orders.len());
    assert_eq!(orders, (0..orders.len()).collect::<Vec<_>>());
}
