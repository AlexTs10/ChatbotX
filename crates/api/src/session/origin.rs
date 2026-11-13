//! Trusted origins and the better-auth origin / CSRF checks.
//!
//! - `createAuth(...).trustedOrigins` (`packages/auth/src/server.ts`): broker,
//!   builder, the mobile app schemes and (outside `next build`) every active
//!   custom domain as `https://<domain>`.
//! - better-auth `getTrustedOrigins`: the `BETTER_AUTH_URL` origin, the app
//!   list and `BETTER_AUTH_TRUSTED_ORIGINS` (comma list).
//! - better-auth `originCheckMiddleware` (every non-GET/HEAD/OPTIONS auth
//!   endpoint) and `formCsrfMiddleware` (sign-in / sign-up email).

use std::future::Future;

use axum::http::HeaderMap;
use serde_json::Value;

use super::AuthApiError;

/// `PHASE_PRODUCTION_BUILD` of `next/constants`.
pub const PHASE_PRODUCTION_BUILD: &str = "phase-production-build";

/// Mobile app deep-link schemes (`DEFAULT_MOBILE_APP_SCHEMES`).
pub const MOBILE_APP_ORIGINS: [&str; 2] = ["chatconnectxapp://", "chatbotxmobileapp://"];

/// The environment `trustedOrigins` reads.
#[derive(Debug, Clone, Default)]
pub struct TrustedOriginSettings {
    /// `NEXT_PUBLIC_BROKER_URL` (`getBrokerUrl()` falls back to the builder URL).
    pub broker_url: Option<String>,
    /// `NEXT_PUBLIC_BUILDER_URL`.
    pub builder_url: String,
    /// `NEXT_PHASE`.
    pub next_phase: Option<String>,
}

/// `customDomainService.listActiveDomains()`.
pub trait CustomDomainSource: Sync {
    /// The hostnames of every active custom domain.
    fn list_active_domains(&self) -> impl Future<Output = Vec<String>> + Send;
}

/// The `trustedOrigins` thunk of `createAuth`: static origins first, then the
/// active custom domains (skipped during the production build); no duplicates.
pub async fn trusted_origins<D: CustomDomainSource>(
    settings: &TrustedOriginSettings,
    domains: &D,
) -> Vec<String> {
    let _ = (settings, domains);
    todo!("P3.5d")
}

/// better-auth `getTrustedOrigins`: the base URL origin, the app origins and
/// `BETTER_AUTH_TRUSTED_ORIGINS`; empty entries dropped.
pub fn context_trusted_origins(
    base_url: &str,
    app_origins: Vec<String>,
    env_origins: Option<&str>,
) -> Vec<String> {
    let _ = (base_url, app_origins, env_origins);
    todo!("P3.5d")
}

/// better-auth `matchesOriginPattern(url, pattern, { allowRelativePaths })`.
pub fn matches_origin_pattern(url: &str, pattern: &str, allow_relative_paths: bool) -> bool {
    let _ = (url, pattern, allow_relative_paths);
    todo!("P3.5d")
}

/// The request parts the origin checks read.
#[derive(Debug, Clone, Copy)]
pub struct OriginCheckRequest<'a> {
    pub method: &'a str,
    pub headers: &'a HeaderMap,
    /// The JSON body (`callbackURL`, `redirectTo`, `errorCallbackURL`, `newUserCallbackURL`).
    pub body: Option<&'a Value>,
    /// `?callbackURL=` of the query string.
    pub query_callback_url: Option<&'a str>,
}

/// better-auth `originCheckMiddleware`.
pub fn origin_check(
    request: &OriginCheckRequest<'_>,
    trusted: &[String],
) -> Result<(), AuthApiError> {
    let _ = (request, trusted);
    todo!("P3.5d")
}

/// better-auth `formCsrfMiddleware` (sign-in / sign-up email).
pub fn form_csrf_check(headers: &HeaderMap, trusted: &[String]) -> Result<(), AuthApiError> {
    let _ = (headers, trusted);
    todo!("P3.5d")
}
