//! Port of `packages/auth/__tests__/generic-oauth-config.test.ts`.
//!
//! The original stubs the `AUTH_OAUTH_*` environment and reads the
//! `genericOAuth` plugin and `account.accountLinking.trustedProviders` out of
//! the mocked `betterAuth(config)` call. Here the environment is a
//! `GenericOAuthEnv` value; `generic_oauth_provider` is the plugin's provider
//! (`None` = not registered) and `trusted_providers` the linking list.

use chatbotx_api::session::{GenericOAuthEnv, generic_oauth_provider, trusted_providers};

const BROKER_URL: &str = "https://broker.example.com";

fn some(value: &str) -> Option<String> {
    Some(value.to_string())
}

fn enabled_with_urls() -> GenericOAuthEnv {
    GenericOAuthEnv {
        enabled: some("true"),
        client_id: some("test-client"),
        client_secret: some("test-secret"),
        auth_url: some("https://idp.example.com/authorize"),
        token_url: some("https://idp.example.com/token"),
        ..GenericOAuthEnv::default()
    }
}

#[test]
fn does_not_register_the_plugin_when_oauth_is_disabled() {
    let env = GenericOAuthEnv {
        enabled: some("false"),
        ..GenericOAuthEnv::default()
    };
    assert!(generic_oauth_provider(&env, BROKER_URL).is_none());
}

#[test]
fn registers_an_enabled_provider_with_parsed_scopes_pkce_and_broker_callback() {
    let env = GenericOAuthEnv {
        scopes: some("openid, email, profile"),
        ..enabled_with_urls()
    };

    let provider = generic_oauth_provider(&env, BROKER_URL).expect("registered");

    assert!(provider.pkce);
    assert_eq!(
        provider.redirect_uri,
        "https://broker.example.com/api/auth/callback/oidc"
    );
    assert_eq!(provider.scopes, ["openid", "email", "profile"]);
}

#[test]
fn registers_an_enabled_provider_configured_with_oidc_discovery() {
    let env = GenericOAuthEnv {
        enabled: some("true"),
        client_id: some("test-client"),
        client_secret: some("test-secret"),
        discovery_url: some("https://idp.example.com/.well-known/openid-configuration"),
        ..GenericOAuthEnv::default()
    };

    let provider = generic_oauth_provider(&env, BROKER_URL).expect("registered");

    assert_eq!(
        provider.discovery_url.as_deref(),
        Some("https://idp.example.com/.well-known/openid-configuration")
    );
    assert!(provider.pkce);
    assert_eq!(
        provider.redirect_uri,
        "https://broker.example.com/api/auth/callback/oidc"
    );
}

#[test]
fn uses_default_scopes_when_scopes_are_omitted() {
    let provider = generic_oauth_provider(&enabled_with_urls(), BROKER_URL).expect("registered");
    assert_eq!(provider.scopes, ["openid", "email", "profile"]);
}

#[test]
fn does_not_register_an_incomplete_provider() {
    let env = GenericOAuthEnv {
        client_id: None,
        ..enabled_with_urls()
    };
    assert!(generic_oauth_provider(&env, BROKER_URL).is_none());
}

#[test]
fn trusts_the_provider_email_only_when_explicitly_configured() {
    let trusting = GenericOAuthEnv {
        trust_email: some("true"),
        ..GenericOAuthEnv::default()
    };
    assert!(trusted_providers(&trusting).contains(&"oidc".to_string()));

    let unset = GenericOAuthEnv {
        trust_email: some(""),
        ..GenericOAuthEnv::default()
    };
    assert!(!trusted_providers(&unset).contains(&"oidc".to_string()));
}

#[test]
fn does_not_trust_the_provider_email_when_email_trust_is_disabled() {
    let env = GenericOAuthEnv {
        trust_email: some("false"),
        ..GenericOAuthEnv::default()
    };
    assert!(!trusted_providers(&env).contains(&"oidc".to_string()));
}
