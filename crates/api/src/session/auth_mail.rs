//! Auth mails of `createAuth` (`packages/auth/src/server.ts`):
//! `magicLink.sendMagicLink` and `emailAndPassword.sendResetPassword`.
//! The tenant services (`resolveTenantSettingsByDomain`, `resolveSmtpForTenant`,
//! the tenant user lookup) and the mail transport are a trait.

use std::future::Future;

use chatbotx_core::utils::placeholder_email::is_placeholder_email;

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
    if is_placeholder_email(Some(email)) {
        return Ok(());
    }
    let request = request.ok_or_else(unknown_request)?;
    let brand = deps.tenant_settings(&request.domain).await;
    let smtp = deps.smtp_for_tenant().await;
    if smtp == SmtpResolution::Blocked {
        return Ok(());
    }
    let magic_url = with_public_host(url, &request.public_origin)?;
    let Some(user) = deps.find_tenant_user(email).await else {
        return Err(AuthApiError::new(
            "BAD_REQUEST",
            &format!("Your email is not registered with {}", brand.name),
        ));
    };
    let mail = AuthMail {
        brand_name: brand.name.clone(),
        brand_logo_url: brand.logo_light_url.clone(),
        brand_url: brand_url(&request.public_origin),
        subject: MAGIC_LINK_SUBJECT.to_string(),
        user_name: user.name.unwrap_or_else(|| email.to_string()),
        url: magic_url,
        custom_template: brand.magic_link_email_template,
    };
    deps.send_magic_link(email, mail, transport(&smtp)).await;
    Ok(())
}

/// `emailAndPassword.sendResetPassword`.
pub async fn send_reset_password<D: AuthMailDeps>(
    deps: &D,
    user: &MailUser,
    url: &str,
    request: Option<&AuthMailRequest>,
) -> Result<(), AuthApiError> {
    if is_placeholder_email(Some(&user.email)) {
        return Ok(());
    }
    let request = request.ok_or_else(unknown_request)?;
    let brand = deps.tenant_settings(&request.domain).await;
    let smtp = deps.smtp_for_tenant().await;
    if smtp == SmtpResolution::Blocked {
        return Ok(());
    }
    let mail = AuthMail {
        brand_name: brand.name.clone(),
        brand_logo_url: brand.logo_light_url.clone(),
        brand_url: brand_url(&request.public_origin),
        subject: FORGOT_PASSWORD_SUBJECT.to_string(),
        user_name: user.name.clone().unwrap_or_else(|| user.email.clone()),
        url: with_public_host(url, &request.public_origin)?,
        custom_template: brand.forgot_password_email_template,
    };
    deps.send_reset_password(&user.email, mail, transport(&smtp))
        .await;
    Ok(())
}

fn unknown_request() -> AuthApiError {
    AuthApiError::new("BAD_REQUEST", "Unknown request")
}

fn transport(smtp: &SmtpResolution) -> Option<&str> {
    match smtp {
        SmtpResolution::Transport(transport) => Some(transport),
        SmtpResolution::Default | SmtpResolution::Blocked => None,
    }
}

/// `new URL("/", originUrl).toString()`.
fn brand_url(public_origin: &str) -> String {
    url::Url::parse(public_origin)
        .and_then(|origin| origin.join("/"))
        .map(|url| url.to_string())
        .unwrap_or_else(|_| format!("{}/", public_origin.trim_end_matches('/')))
}

/// `link.hostname = new URL(originUrl).hostname`.
fn with_public_host(link: &str, public_origin: &str) -> Result<String, AuthApiError> {
    let invalid = |_| AuthApiError::new("INTERNAL_SERVER_ERROR", "Invalid URL");
    let mut link = url::Url::parse(link).map_err(invalid)?;
    let origin = url::Url::parse(public_origin).map_err(invalid)?;
    if let Some(host) = origin.host_str() {
        link.set_host(Some(host))
            .map_err(|_| AuthApiError::new("INTERNAL_SERVER_ERROR", "Invalid URL"))?;
    }
    Ok(link.to_string())
}
