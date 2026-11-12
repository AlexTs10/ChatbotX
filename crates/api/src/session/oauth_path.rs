//! `isGenericOAuthPath` / `isSharedSocialPath` of `packages/auth/src/generic-oauth.ts`.

/// `GENERIC_OAUTH_PROVIDER_ID`.
pub const GENERIC_OAUTH_PROVIDER_ID: &str = "oidc";

/// Social routes shared by first-party providers and the generic provider.
pub const SHARED_SOCIAL_PATH_SUFFIXES: &[&str] =
    &["/sign-in/social", "/sign-up/social", "/link-social"];

/// `isSharedSocialPath(pathname)`.
pub fn is_shared_social_path(pathname: &str) -> bool {
    let _ = pathname;
    todo!("P3.5d")
}

/// `isGenericOAuthPath(pathname, provider)`.
pub fn is_generic_oauth_path(pathname: &str, provider: Option<&str>) -> bool {
    let _ = (pathname, provider);
    todo!("P3.5d")
}
