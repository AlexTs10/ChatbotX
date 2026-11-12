//! `claimUnverifiedAccountBeforeLink` (`packages/auth/src/claim-unverified-account.ts`):
//! the `account.create.before` hook that lets an email-attesting social
//! provider claim an unverified (placeholder) user.

use std::future::Future;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_core::utils::placeholder_email::is_placeholder_email;
use serde_json::Value;

use super::GENERIC_OAUTH_PROVIDER_ID;

const CREDENTIAL_PROVIDER: &str = "credential";
const FRESH_SIGN_UP_WINDOW_MS: i64 = 60_000;

/// The account better-auth is about to link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkAccount {
    pub user_id: String,
    pub provider_id: String,
    pub id_token: Option<String>,
}

/// `user.createdAt` as the adapter hands it over (a `Date` or an ISO string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatedAt {
    Millis(i64),
    Iso(String),
}

/// The user fields the hook reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimUser {
    pub id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub created_at: Option<CreatedAt>,
}

/// One existing account of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingAccount {
    pub id: String,
    pub provider_id: String,
}

/// `internalAdapter` / `adapter` calls of the hook, plus its settings.
pub trait ClaimAdapter: Sync {
    fn find_user_by_id(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<Option<ClaimUser>, String>> + Send;
    fn find_accounts(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Vec<ExistingAccount>, String>> + Send;
    fn delete_account(&self, id: &str) -> impl Future<Output = Result<(), String>> + Send;
    /// `updateUser(userId, { emailVerified: true })`.
    fn mark_email_verified(&self, user_id: &str)
    -> impl Future<Output = Result<(), String>> + Send;
    /// `adapter.deleteMany({ model: "session", where: userId })`; the number removed.
    fn delete_sessions(&self, user_id: &str) -> impl Future<Output = Result<u64, String>> + Send;
    /// `AUTH_OAUTH_TRUST_EMAIL`.
    fn trust_oauth_email(&self) -> bool;
    /// `Date.now()`.
    fn now_ms(&self) -> i64;
}

/// Error messages of the hook (better-auth refuses the link).
pub const NO_CONTEXT_ERROR: &str = "Cannot inspect the user without an endpoint context";
pub const UNKNOWN_USER_ERROR: &str = "Cannot link an account to an unknown user";
pub const NOT_EMAIL_ATTESTING_ERROR: &str =
    "Only an email-attesting provider can claim an unverified account";
pub const DID_NOT_ATTEST_ERROR: &str = "The provider did not attest the email address";

/// `claimUnverifiedAccountBeforeLink(account, context)`; `adapter` is `None`
/// when better-auth gives no endpoint context.
pub async fn claim_unverified_account_before_link<A: ClaimAdapter>(
    account: &LinkAccount,
    adapter: Option<&A>,
) -> Result<(), String> {
    if account.provider_id == CREDENTIAL_PROVIDER {
        return Ok(());
    }
    let adapter = adapter.ok_or_else(|| NO_CONTEXT_ERROR.to_string())?;
    let user = adapter
        .find_user_by_id(&account.user_id)
        .await?
        .ok_or_else(|| UNKNOWN_USER_ERROR.to_string())?;
    if user.email_verified {
        return Ok(());
    }
    let others = adapter.find_accounts(&account.user_id).await?;
    let provider = account.provider_id.as_str();
    let attests =
        matches!(provider, "google" | "facebook") || provider == GENERIC_OAUTH_PROVIDER_ID;
    let attested = match provider {
        "google" => is_email_attested_by_id_token(account.id_token.as_deref()),
        "facebook" => true,
        _ if provider == GENERIC_OAUTH_PROVIDER_ID => {
            adapter.trust_oauth_email()
                || is_email_attested_by_id_token(account.id_token.as_deref())
        }
        _ => false,
    };
    let promotes = provider == "google" || provider == GENERIC_OAUTH_PROVIDER_ID;
    let should_promote = attested
        && promotes
        && user
            .email
            .as_deref()
            .is_some_and(|email| !is_placeholder_email(Some(email)));
    if others.is_empty() && is_fresh_sign_up(user.created_at.as_ref(), adapter.now_ms()) {
        if should_promote {
            adapter.mark_email_verified(&account.user_id).await?;
        }
        return Ok(());
    }
    if !attests {
        return Err(NOT_EMAIL_ATTESTING_ERROR.to_string());
    }
    if !attested {
        return Err(DID_NOT_ATTEST_ERROR.to_string());
    }
    for row in &others {
        adapter.delete_account(&row.id).await?;
    }
    let revoked_sessions = adapter.delete_sessions(&account.user_id).await?;
    if should_promote {
        adapter.mark_email_verified(&account.user_id).await?;
    }
    tracing::info!(
        user_id = %account.user_id,
        provider_id = %account.provider_id,
        removed_accounts = others.len(),
        revoked_sessions,
        "Unverified placeholder account claimed by a trusted social sign-in"
    );
    Ok(())
}

/// `isEmailAttestedByIdToken`: the unverified id token payload has `email_verified: true`.
fn is_email_attested_by_id_token(id_token: Option<&str>) -> bool {
    let Some(segment) = id_token.and_then(|token| token.split('.').nth(1)) else {
        return false;
    };
    URL_SAFE_NO_PAD
        .decode(segment.trim_end_matches('='))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|payload| payload["email_verified"] == Value::Bool(true))
}

/// `isFreshSignUp(createdAt)`: created at most one minute ago.
fn is_fresh_sign_up(created_at: Option<&CreatedAt>, now_ms: i64) -> bool {
    let created = match created_at {
        Some(CreatedAt::Millis(ms)) => Some(*ms),
        Some(CreatedAt::Iso(text)) => parse_iso_millis(text),
        None => None,
    };
    created.is_some_and(|created| now_ms - created <= FRESH_SIGN_UP_WINDOW_MS)
}

/// Milliseconds of `YYYY-MM-DDTHH:MM:SS[.fff]Z` (the `toISOString` form).
fn parse_iso_millis(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
    );
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut clock_parts = clock.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock_parts.next()?.ok()?,
        clock_parts.next()?.ok()?,
        clock_parts.next()?.ok()?,
    );
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    // Days from civil (Howard Hinnant).
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}
