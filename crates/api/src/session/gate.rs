//! The sign-up lockdown (`packages/auth/src/signup-gate.ts`), the
//! `user.create.before` database hook and the `/sign-up/email` request hook
//! (`guardEmailSignUp`) of `packages/auth/src/server.ts`.

use std::future::Future;

use axum::http::HeaderMap;

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
        let _ = (status_name, message);
        todo!("P3.5d")
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
    let _ = (headers, name);
    todo!("P3.5d")
}

/// `isSignUpAllowed(context)`: open sign-up, or a live invitation cookie.
pub async fn is_sign_up_allowed<D: SignUpGateDeps>(deps: &D, headers: Option<&HeaderMap>) -> bool {
    let _ = (deps, headers);
    todo!("P3.5d")
}

/// `databaseHooks.user.create.before`: anonymous users pass; others need
/// [`is_sign_up_allowed`] (else `FORBIDDEN signup_disabled`).
pub async fn user_create_before<D: SignUpGateDeps>(
    deps: &D,
    user: &NewUser,
    headers: Option<&HeaderMap>,
) -> Result<(), AuthApiError> {
    let _ = (deps, user, headers);
    todo!("P3.5d")
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
    let _ = (deps, path, body_email, headers);
    todo!("P3.5d")
}
