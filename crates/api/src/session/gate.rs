//! The sign-up lockdown (`packages/auth/src/signup-gate.ts`), the
//! `user.create.before` database hook and the `/sign-up/email` request hook
//! (`guardEmailSignUp`) of `packages/auth/src/server.ts`.

use std::future::Future;

use axum::http::{HeaderMap, header};
use chatbotx_core::utils::placeholder_email::is_placeholder_email;

/// `INVITATION_COOKIE_NAME`.
pub const INVITATION_COOKIE_NAME: &str = "chatbotx_invitation";
/// `INVITATION_COOKIE_MAX_AGE_SECONDS`.
pub const INVITATION_COOKIE_MAX_AGE_SECONDS: u64 = 10 * 60;
/// `SIGNUP_DISABLED_CODE`.
pub const SIGNUP_DISABLED_CODE: &str = "signup_disabled";

/// A better-auth `APIError`: HTTP status, status name and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthApiError {
    pub status: u16,
    /// `FORBIDDEN`, `BAD_REQUEST`, ...
    pub status_name: String,
    pub message: String,
    /// The better-auth error code of the body (`EMAIL_NOT_VERIFIED`, ...), if any.
    pub code: Option<String>,
}

impl AuthApiError {
    /// `new APIError(statusName, { message })`.
    pub fn new(status_name: &str, message: &str) -> AuthApiError {
        let status = match status_name {
            "BAD_REQUEST" => 400,
            "UNAUTHORIZED" => 401,
            "FORBIDDEN" => 403,
            "NOT_FOUND" => 404,
            "UNPROCESSABLE_ENTITY" => 422,
            "TOO_MANY_REQUESTS" => 429,
            _ => 500,
        };
        AuthApiError {
            status,
            status_name: status_name.to_string(),
            message: message.to_string(),
            code: None,
        }
    }
}

/// The services the gate calls.
pub trait SignUpGateDeps: Sync {
    /// `isSignUpDisabled()` (`AUTH_DISABLE_SIGNUP`).
    fn is_sign_up_disabled(&self) -> bool;
    /// `getTenantId()`.
    fn tenant_id(&self) -> String;
    /// `invitationService.findActiveByCode(code, { tenantId })` found a live invitation.
    fn find_active_invitation(
        &self,
        code: &str,
        tenant_id: &str,
    ) -> impl Future<Output = bool> + Send;
}

/// The user fields the `user.create.before` hook reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewUser {
    pub email: Option<String>,
    /// `user.isAnonymous === true`.
    pub is_anonymous: Option<bool>,
}

/// The value of cookie `name` in the `Cookie` header(s).
pub fn request_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// `isSignUpAllowed(context)`: open sign-up, or a live invitation cookie.
pub async fn is_sign_up_allowed<D: SignUpGateDeps>(deps: &D, headers: Option<&HeaderMap>) -> bool {
    if !deps.is_sign_up_disabled() {
        return true;
    }
    let Some(code) = headers
        .and_then(|headers| request_cookie(headers, INVITATION_COOKIE_NAME))
        .filter(|code| !code.is_empty())
    else {
        return false;
    };
    deps.find_active_invitation(&code, &deps.tenant_id()).await
}

/// `databaseHooks.user.create.before`: anonymous users pass; others need
/// [`is_sign_up_allowed`] (else `FORBIDDEN signup_disabled`).
pub async fn user_create_before<D: SignUpGateDeps>(
    deps: &D,
    user: &NewUser,
    headers: Option<&HeaderMap>,
) -> Result<(), AuthApiError> {
    if user.is_anonymous == Some(true) || is_sign_up_allowed(deps, headers).await {
        return Ok(());
    }
    Err(AuthApiError::new("FORBIDDEN", SIGNUP_DISABLED_CODE))
}

/// `hooks.before` (`guardEmailSignUp`) for `path` (relative to `/api/auth`):
/// `/sign-up/email` refuses a placeholder email (`BAD_REQUEST Invalid email`)
/// and a locked sign-up (`FORBIDDEN Sign-up is disabled`).
pub async fn email_request_before<D: SignUpGateDeps>(
    deps: &D,
    path: &str,
    body_email: Option<&str>,
    headers: &HeaderMap,
) -> Result<(), AuthApiError> {
    if path != "/sign-up/email" {
        return Ok(());
    }
    if is_placeholder_email(body_email) {
        return Err(AuthApiError::new("BAD_REQUEST", "Invalid email"));
    }
    if !is_sign_up_allowed(deps, Some(headers)).await {
        return Err(AuthApiError::new("FORBIDDEN", "Sign-up is disabled"));
    }
    Ok(())
}
