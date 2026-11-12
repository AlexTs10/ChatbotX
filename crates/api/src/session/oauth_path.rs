//! `isGenericOAuthPath` / `isSharedSocialPath` of `packages/auth/src/generic-oauth.ts`.

/// `GENERIC_OAUTH_PROVIDER_ID`.
pub const GENERIC_OAUTH_PROVIDER_ID: &str = "oidc";

/// Social routes shared by first-party providers and the generic provider.
pub const SHARED_SOCIAL_PATH_SUFFIXES: &[&str] =
    &["/sign-in/social", "/sign-up/social", "/link-social"];

/// `isSharedSocialPath(pathname)`.
pub fn is_shared_social_path(pathname: &str) -> bool {
    SHARED_SOCIAL_PATH_SUFFIXES
        .iter()
        .any(|suffix| pathname.ends_with(suffix))
}

/// `isGenericOAuthPath(pathname, provider)`.
pub fn is_generic_oauth_path(pathname: &str, provider: Option<&str>) -> bool {
    pathname.ends_with(&format!("/callback/{GENERIC_OAUTH_PROVIDER_ID}"))
        || (provider == Some(GENERIC_OAUTH_PROVIDER_ID) && is_shared_social_path(pathname))
}
