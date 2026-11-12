//! `claimUnverifiedAccountBeforeLink` (`packages/auth/src/claim-unverified-account.ts`):
//! the `account.create.before` hook that lets an email-attesting social
//! provider claim an unverified (placeholder) user.

use std::future::Future;

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
    let _ = (account, adapter);
    todo!("P3.5d")
}
