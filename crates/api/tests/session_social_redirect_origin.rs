//! Port of `packages/auth/__tests__/social-redirect-origin.test.ts`.
//!
//! The original reads `socialProviders` and `hooks.before` out of the mocked
//! `betterAuth(config)` call (`NEXT_PUBLIC_BROKER_URL` stubbed). Here they are
//! `build_social_providers(...)`, `map_facebook_profile_to_user` and
//! `email_request_before` with the broker URL as an argument.

use axum::http::HeaderMap;
use chatbotx_api::session::{
    SignUpGateDeps, SocialCredential, SocialProviderOptions, build_social_providers,
    email_request_before, map_facebook_profile_to_user,
};
use serde_json::json;

const BROKER_URL: &str = "https://broker.example.com";

fn credential(id: &str, secret: &str) -> SocialCredential {
    SocialCredential {
        client_id: id.to_string(),
        client_secret: secret.to_string(),
    }
}

fn google_only() -> Vec<(String, SocialCredential)> {
    vec![("google".to_string(), credential("id-1", "secret-1"))]
}

fn google_and_facebook() -> Vec<(String, SocialCredential)> {
    vec![
        ("google".to_string(), credential("id-1", "secret-1")),
        ("facebook".to_string(), credential("id-2", "secret-2")),
    ]
}

fn provider<'a>(providers: &'a [SocialProviderOptions], name: &str) -> &'a SocialProviderOptions {
    providers
        .iter()
        .find(|options| options.provider == name)
        .unwrap_or_else(|| panic!("{name} not configured"))
}

#[test]
fn pins_redirecturi_to_the_broker_origin_when_socialredirectorigin_is_omitted() {
    let providers =
        build_social_providers(Some(&google_only()), None, None, BROKER_URL, Some("")).unwrap();

    assert_eq!(
        provider(&providers, "google").redirect_uri,
        format!("{BROKER_URL}/api/auth/callback/google")
    );
}

#[test]
fn pins_redirecturi_to_the_caller_supplied_socialredirectorigin_tenant_custom_domain() {
    let providers = build_social_providers(
        Some(&google_only()),
        None,
        Some("https://chat.acme.com"),
        BROKER_URL,
        Some(""),
    )
    .unwrap();

    assert_eq!(
        provider(&providers, "google").redirect_uri,
        "https://chat.acme.com/api/auth/callback/google"
    );
}

#[test]
fn applies_socialredirectorigin_to_every_enabled_provider() {
    let providers = build_social_providers(
        Some(&google_and_facebook()),
        None,
        Some("https://chat.acme.com"),
        BROKER_URL,
        Some(""),
    )
    .unwrap();

    assert_eq!(
        provider(&providers, "google").redirect_uri,
        "https://chat.acme.com/api/auth/callback/google"
    );
    assert_eq!(
        provider(&providers, "facebook").redirect_uri,
        "https://chat.acme.com/api/auth/callback/facebook"
    );
}

#[test]
fn gives_only_facebook_a_placeholder_email_for_phone_only_accounts() {
    let providers = build_social_providers(
        Some(&google_and_facebook()),
        None,
        None,
        BROKER_URL,
        Some(""),
    )
    .unwrap();

    assert!(!provider(&providers, "google").maps_facebook_profile);
    assert!(provider(&providers, "facebook").maps_facebook_profile);
    assert_eq!(
        map_facebook_profile_to_user(&json!({ "id": "42" })),
        json!({ "email": "fb-42@facebook.placeholder.invalid", "emailVerified": false })
    );
}

struct OpenGate;

impl SignUpGateDeps for OpenGate {
    fn is_sign_up_disabled(&self) -> bool {
        false
    }

    fn tenant_id(&self) -> String {
        "1".to_string()
    }

    async fn find_active_invitation(&self, _code: &str, _tenant_id: &str) -> bool {
        false
    }
}

#[tokio::test]
async fn rejects_an_email_password_sign_up_using_the_facebook_placeholder_domain() {
    let headers = HeaderMap::new();
    let sign_up = |path: &'static str, email: &'static str| {
        let headers = &headers;
        async move { email_request_before(&OpenGate, path, Some(email), headers).await }
    };

    let error = sign_up("/sign-up/email", "fb-42@facebook.placeholder.invalid")
        .await
        .expect_err("rejected");
    assert!(error.message.contains("Invalid email"), "{error:?}");
    assert!(sign_up("/sign-up/email", "a@example.com").await.is_ok());
    assert!(
        sign_up("/callback/facebook", "fb-42@facebook.placeholder.invalid")
            .await
            .is_ok()
    );
}
