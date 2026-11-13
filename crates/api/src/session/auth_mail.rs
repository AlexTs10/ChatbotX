//! Auth mails of `createAuth` (`packages/auth/src/server.ts`):
//! `magicLink.sendMagicLink` and `emailAndPassword.sendResetPassword`.
//! The tenant services (`resolveTenantSettingsByDomain`, `resolveSmtpForTenant`,
//! the tenant user lookup) and the mail transport are a trait.

use std::future::Future;

use super::AuthApiError;

/// `DEFAULT_MAGIC_LINK_SUBJECT` of `@chatbotx.io/mail`.
pub const MAGIC_LINK_SUBJECT: &str = "Sign in to {{brandName}}";
/// `DEFAULT_FORGOT_PASSWORD_SUBJECT` of `@chatbotx.io/mail`.
pub const FORGOT_PASSWORD_SUBJECT: &str = "Reset your password";

/// What the mail hooks read from the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMailRequest {
    /// `getPublicOriginFromRequest(request)`.
    pub public_origin: String,
    /// `x-domain` header (tenant resolution).
    pub domain: String,
}

/// `resolveTenantSettingsByDomain(domain)` (the fields the mails use).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantBrand {
    pub name: String,
    pub logo_light_url: Option<String>,
    pub magic_link_email_template: Option<String>,
    pub forgot_password_email_template: Option<String>,
}

/// `resolveSmtpForTenant()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpResolution {
    /// The platform default transport.
    Default,
    /// A tenant / platform transport (opaque connection URL).
    Transport(String),
    /// A reseller without SMTP credential: nothing is sent.
    Blocked,
}

/// A user the mails address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailUser {
    pub name: Option<String>,
    pub email: String,
}

/// The template props of `sendMagicLink` / `sendResetPassword`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMail {
    pub brand_name: String,
    pub brand_logo_url: Option<String>,
    pub brand_url: String,
    pub subject: String,
    pub user_name: String,
    /// `magicUrl` or `resetPasswordUrl`.
    pub url: String,
    pub custom_template: Option<String>,
}

/// The services the mail hooks call.
pub trait AuthMailDeps: Sync {
    fn tenant_settings(&self, domain: &str) -> impl Future<Output = TenantBrand> + Send;
    fn smtp_for_tenant(&self) -> impl Future<Output = SmtpResolution> + Send;
    /// The tenant's user (or the reseller owner) with this email.
    fn find_tenant_user(&self, email: &str) -> impl Future<Output = Option<MailUser>> + Send;
    /// `sendMagicLink(to, props, transport?)`.
    fn send_magic_link(
        &self,
        to: &str,
        mail: AuthMail,
        transport: Option<&str>,
    ) -> impl Future<Output = ()> + Send;
    /// `sendResetPassword(to, props, transport?)`.
    fn send_reset_password(
        &self,
        to: &str,
        mail: AuthMail,
        transport: Option<&str>,
    ) -> impl Future<Output = ()> + Send;
}

/// `magicLink({ sendMagicLink })`.
pub async fn send_magic_link<D: AuthMailDeps>(
    deps: &D,
    email: &str,
    url: &str,
    request: Option<&AuthMailRequest>,
) -> Result<(), AuthApiError> {
    let _ = (deps, email, url, request);
    todo!("P3.5d")
}

/// `emailAndPassword.sendResetPassword`.
pub async fn send_reset_password<D: AuthMailDeps>(
    deps: &D,
    user: &MailUser,
    url: &str,
    request: Option<&AuthMailRequest>,
) -> Result<(), AuthApiError> {
    let _ = (deps, user, url, request);
    todo!("P3.5d")
}
