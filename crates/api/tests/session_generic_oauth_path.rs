//! Port of `packages/auth/__tests__/generic-oauth-path.test.ts`.

use chatbotx_api::session::is_generic_oauth_path;

#[test]
fn matches_the_generic_provider_callback() {
    assert!(is_generic_oauth_path("/api/auth/callback/oidc", None));
}

#[test]
fn matches_a_social_sign_in_selecting_the_generic_provider() {
    assert!(is_generic_oauth_path(
        "/api/auth/sign-in/social",
        Some("oidc")
    ));
    assert!(is_generic_oauth_path(
        "/api/auth/sign-up/social",
        Some("oidc")
    ));
}

#[test]
fn matches_linking_an_account_with_the_generic_provider() {
    assert!(is_generic_oauth_path("/api/auth/link-social", Some("oidc")));
    assert!(!is_generic_oauth_path(
        "/api/auth/link-social",
        Some("google")
    ));
}

#[test]
fn does_not_match_unrelated_or_first_party_social_auth_paths() {
    assert!(!is_generic_oauth_path("/api/auth/get-session", None));
    assert!(!is_generic_oauth_path(
        "/api/auth/sign-in/social",
        Some("google")
    ));
    assert!(!is_generic_oauth_path("/api/auth/callback/google", None));
}
