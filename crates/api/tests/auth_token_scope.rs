//! Port of `apps/builder/__tests__/token-scope.test.ts`.

use chatbotx_api::auth::{RequestApiToken, assert_token_scope, token_has_any_scope};

fn scopes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

fn token(scopes: Option<Vec<String>>) -> RequestApiToken {
    RequestApiToken {
        id: "1".to_string(),
        workspace_id: "1".to_string(),
        permission: "full".to_string(),
        scopes,
        is_default: false,
    }
}

// `requireTokenScope` delegates to `tokenHasAnyScope`, so these pin the scope
// rule every workspace-token route relies on.

// adapted: `null` and `undefined` scopes are both `None`
#[test]
fn tokenhasanyscope_null_and_undefined_scopes_mean_all_scopes() {
    assert!(token_has_any_scope(None, &["appointments"]));
    assert!(token_has_any_scope(None, &["inbox"]));
}

#[test]
fn tokenhasanyscope_is_any_of_never_all_of() {
    assert!(token_has_any_scope(
        Some(&scopes(&["inbox"])),
        &["appointments", "inbox"]
    ));
    assert!(!token_has_any_scope(
        Some(&scopes(&["contacts"])),
        &["appointments", "inbox"]
    ));
}

#[test]
fn tokenhasanyscope_an_empty_scope_list_grants_nothing() {
    assert!(!token_has_any_scope(Some(&[]), &["appointments"]));
}

#[test]
fn asserttokenscope_passes_for_a_token_with_the_scope_or_with_all_scopes() {
    assert!(assert_token_scope(Some(&token(Some(scopes(&["inbox"])))), "inbox").is_ok());
    assert!(assert_token_scope(Some(&token(None)), "inbox").is_ok());
    // Member/session callers carry no token and are scoped elsewhere.
    assert!(assert_token_scope(None, "inbox").is_ok());
}

#[test]
fn asserttokenscope_throws_forbidden_with_the_middleware_s_message_format() {
    let error =
        assert_token_scope(Some(&token(Some(scopes(&["appointments"])))), "inbox").unwrap_err();
    assert_eq!(error.code, "FORBIDDEN");
    assert_eq!(
        error.message,
        "Token is not authorized for the 'inbox' scope"
    );
}
