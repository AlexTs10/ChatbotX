//! Social and generic OAuth provider configuration of `createAuth`
//! (`packages/auth/src/server.ts` `buildSocialProviders`, the `genericOAuth`
//! plugin entry, `account.accountLinking.trustedProviders`;
//! `generic-oauth.ts` `resolveGenericOAuthConfig`).

use chatbotx_core::utils::placeholder_email::facebook_placeholder_email;
use serde_json::{Value, json};

use super::GENERIC_OAUTH_PROVIDER_ID;
use super::PHASE_PRODUCTION_BUILD;

/// `SOCIAL_PROVIDERS`.
pub const SOCIAL_PROVIDERS: [&str; 2] = ["google", "facebook"];
/// Default of `AUTH_OAUTH_SCOPES`.
pub const DEFAULT_GENERIC_OAUTH_SCOPES: &str = "openid,email,profile";

/// A resolved social credential (`SocialAuthCredential`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialCredential {
    pub client_id: String,
    pub client_secret: String,
}

/// One entry of `socialProviders`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialProviderOptions {
    pub provider: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    /// `scope` (with `disableDefaultScope: true`) when configured.
    pub scope: Option<Vec<String>>,
    /// `mapProfileToUser: mapFacebookProfileToUser` (Facebook only).
    pub maps_facebook_profile: bool,
}

/// `buildSocialProviders(socialCredentials, socialScopes, redirectOrigin)`:
/// `None` during the production build or when nothing resolved.
pub fn build_social_providers(
    credentials: Option<&[(String, SocialCredential)]>,
    scopes: Option<&[(String, Vec<String>)]>,
    redirect_origin: Option<&str>,
    broker_url: &str,
    next_phase: Option<&str>,
) -> Option<Vec<SocialProviderOptions>> {
    if next_phase == Some(PHASE_PRODUCTION_BUILD) {
        return None;
    }
    let credentials = credentials?;
    let pinned = url::Url::parse(redirect_origin.unwrap_or(broker_url))
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let mut providers = Vec::new();
    for provider in SOCIAL_PROVIDERS {
        let Some((_, credential)) = credentials.iter().find(|(name, _)| name == provider) else {
            continue;
        };
        if credential.client_id.is_empty() || credential.client_secret.is_empty() {
            continue;
        }
        let scope = scopes
            .and_then(|scopes| scopes.iter().find(|(name, _)| name == provider))
            .map(|(_, scope)| scope.clone());
        providers.push(SocialProviderOptions {
            provider: provider.to_string(),
            client_id: credential.client_id.clone(),
            client_secret: credential.client_secret.clone(),
            redirect_uri: format!("{pinned}/api/auth/callback/{provider}"),
            scope,
            maps_facebook_profile: provider == "facebook",
        });
    }
    (!providers.is_empty()).then_some(providers)
}

/// `mapFacebookProfileToUser(profile)`: a placeholder email for a profile
/// without one (`{}` otherwise).
pub fn map_facebook_profile_to_user(profile: &Value) -> Value {
    let text = |name: &str| {
        profile
            .get(name)
            .and_then(|value| match value {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
    };
    let facebook_id = text("id").or_else(|| text("sub"));
    match (text("email"), facebook_id) {
        (None, Some(id)) => json!({
            "email": facebook_placeholder_email(&id),
            "emailVerified": false,
        }),
        _ => json!({}),
    }
}

/// The `AUTH_OAUTH_*` environment.
#[derive(Debug, Clone, Default)]
pub struct GenericOAuthEnv {
    pub enabled: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub auth_url: Option<String>,
    pub token_url: Option<String>,
    pub discovery_url: Option<String>,
    pub userinfo_url: Option<String>,
    pub scopes: Option<String>,
    pub trust_email: Option<String>,
}

/// `resolveGenericOAuthConfig()` plus the plugin entry of `createAuth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericOAuthProvider {
    pub provider_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub discovery_url: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub user_info_url: Option<String>,
    pub scopes: Vec<String>,
    pub pkce: bool,
    pub redirect_uri: String,
}

/// The `genericOAuth` plugin's provider, `None` when the plugin is not registered.
pub fn generic_oauth_provider(
    env: &GenericOAuthEnv,
    broker_url: &str,
) -> Option<GenericOAuthProvider> {
    if !env_bool(env.enabled.as_deref()) {
        return None;
    }
    let present = |value: &Option<String>| value.clone().filter(|value| !value.is_empty());
    let (Some(client_id), Some(client_secret)) =
        (present(&env.client_id), present(&env.client_secret))
    else {
        tracing::error!("AUTH_OAUTH_ENABLED is set but the provider is incomplete");
        return None;
    };
    let discovery_url = present(&env.discovery_url);
    let (authorization_url, token_url) = match (
        &discovery_url,
        present(&env.auth_url),
        present(&env.token_url),
    ) {
        (Some(_), _, _) => (None, None),
        (None, Some(auth), Some(token)) => (Some(auth), Some(token)),
        _ => {
            tracing::error!("AUTH_OAUTH_ENABLED is set but the provider is incomplete");
            return None;
        }
    };
    let scopes = env
        .scopes
        .as_deref()
        .filter(|scopes| !scopes.trim().is_empty())
        .unwrap_or(DEFAULT_GENERIC_OAUTH_SCOPES)
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect();
    let origin = url::Url::parse(broker_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    Some(GenericOAuthProvider {
        provider_id: GENERIC_OAUTH_PROVIDER_ID.to_string(),
        client_id,
        client_secret,
        discovery_url,
        authorization_url,
        token_url,
        user_info_url: present(&env.userinfo_url),
        scopes,
        pkce: true,
        redirect_uri: format!("{origin}/api/auth/callback/{GENERIC_OAUTH_PROVIDER_ID}"),
    })
}

/// `account.accountLinking.trustedProviders`.
pub fn trusted_providers(env: &GenericOAuthEnv) -> Vec<String> {
    let mut providers: Vec<String> = SOCIAL_PROVIDERS.iter().map(|p| (*p).to_string()).collect();
    if env_bool(env.trust_email.as_deref()) {
        providers.push(GENERIC_OAUTH_PROVIDER_ID.to_string());
    }
    providers
}

/// `z.stringbool()` truthy values (`normalizeBoolean`); unset / empty = false.
fn env_bool(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on" | "y" | "enabled"
        )
    })
}
