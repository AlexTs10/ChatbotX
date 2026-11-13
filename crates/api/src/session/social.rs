//! Social and generic OAuth provider configuration of `createAuth`
//! (`packages/auth/src/server.ts` `buildSocialProviders`, the `genericOAuth`
//! plugin entry, `account.accountLinking.trustedProviders`;
//! `generic-oauth.ts` `resolveGenericOAuthConfig`).

use serde_json::Value;

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
    let _ = (credentials, scopes, redirect_origin, broker_url, next_phase);
    todo!("P3.5d")
}

/// `mapFacebookProfileToUser(profile)`: a placeholder email for a profile
/// without one (`{}` otherwise).
pub fn map_facebook_profile_to_user(profile: &Value) -> Value {
    let _ = profile;
    todo!("P3.5d")
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
    let _ = (env, broker_url);
    todo!("P3.5d")
}

/// `account.accountLinking.trustedProviders`.
pub fn trusted_providers(env: &GenericOAuthEnv) -> Vec<String> {
    let _ = env;
    todo!("P3.5d")
}
