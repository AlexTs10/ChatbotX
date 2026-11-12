//! Port of `packages/auth/__tests__/claim-unverified-account.test.ts`.

use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_api::session::{
    ClaimAdapter, ClaimUser, CreatedAt, ExistingAccount, LinkAccount,
    claim_unverified_account_before_link,
};

const USER_ID: &str = "11728999944477963";
const NOW_MS: i64 = 1_800_000_000_000;
const TEN_MINUTES_MS: i64 = 600_000;

struct Adapter {
    user: Mutex<Option<ClaimUser>>,
    accounts: Mutex<Vec<ExistingAccount>>,
    trust_email: bool,
    fail_next_delete: Mutex<Option<String>>,
    find_user_calls: Mutex<usize>,
    find_accounts_calls: Mutex<usize>,
    deleted_accounts: Mutex<Vec<String>>,
    session_deletes: Mutex<Vec<String>>,
    verified: Mutex<Vec<String>>,
}

impl Adapter {
    /// `beforeEach`: an old unverified user with a password and a Facebook login.
    fn new() -> Adapter {
        Adapter {
            user: Mutex::new(Some(ClaimUser {
                id: USER_ID.to_string(),
                email: Some("owner@example.com".to_string()),
                email_verified: false,
                created_at: Some(CreatedAt::Millis(NOW_MS - TEN_MINUTES_MS)),
            })),
            accounts: Mutex::new(vec![
                existing("acc-pwd", "credential"),
                existing("acc-fb", "facebook"),
            ]),
            trust_email: false,
            fail_next_delete: Mutex::new(None),
            find_user_calls: Mutex::new(0),
            find_accounts_calls: Mutex::new(0),
            deleted_accounts: Mutex::new(Vec::new()),
            session_deletes: Mutex::new(Vec::new()),
            verified: Mutex::new(Vec::new()),
        }
    }

    fn with_accounts(self, accounts: Vec<ExistingAccount>) -> Adapter {
        *self.accounts.lock().unwrap() = accounts;
        self
    }

    fn with_user(self, user: Option<ClaimUser>) -> Adapter {
        *self.user.lock().unwrap() = user;
        self
    }

    fn deleted(&self) -> Vec<String> {
        self.deleted_accounts.lock().unwrap().clone()
    }

    fn session_deletes(&self) -> Vec<String> {
        self.session_deletes.lock().unwrap().clone()
    }
}

impl ClaimAdapter for Adapter {
    async fn find_user_by_id(&self, _id: &str) -> Result<Option<ClaimUser>, String> {
        *self.find_user_calls.lock().unwrap() += 1;
        Ok(self.user.lock().unwrap().clone())
    }

    async fn find_accounts(&self, _user_id: &str) -> Result<Vec<ExistingAccount>, String> {
        *self.find_accounts_calls.lock().unwrap() += 1;
        Ok(self.accounts.lock().unwrap().clone())
    }

    async fn delete_account(&self, id: &str) -> Result<(), String> {
        if let Some(error) = self.fail_next_delete.lock().unwrap().take() {
            return Err(error);
        }
        self.deleted_accounts.lock().unwrap().push(id.to_string());
        Ok(())
    }

    async fn mark_email_verified(&self, user_id: &str) -> Result<(), String> {
        self.verified.lock().unwrap().push(user_id.to_string());
        Ok(())
    }

    async fn delete_sessions(&self, user_id: &str) -> Result<u64, String> {
        self.session_deletes
            .lock()
            .unwrap()
            .push(user_id.to_string());
        Ok(2)
    }

    fn trust_oauth_email(&self) -> bool {
        self.trust_email
    }

    fn now_ms(&self) -> i64 {
        NOW_MS
    }
}

fn existing(id: &str, provider_id: &str) -> ExistingAccount {
    ExistingAccount {
        id: id.to_string(),
        provider_id: provider_id.to_string(),
    }
}

fn id_token_with(claims: &str) -> String {
    format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims))
}

fn verified_token() -> String {
    id_token_with(r#"{"email_verified":true}"#)
}

fn account(provider_id: &str, id_token: Option<String>) -> LinkAccount {
    LinkAccount {
        user_id: USER_ID.to_string(),
        provider_id: provider_id.to_string(),
        id_token,
    }
}

fn fresh_user(created_at: Option<CreatedAt>) -> Option<ClaimUser> {
    Some(ClaimUser {
        id: USER_ID.to_string(),
        email: None,
        email_verified: false,
        created_at,
    })
}

#[tokio::test]
async fn google_into_an_unverified_user_drops_every_other_login_method_and_its_sessions() {
    let adapter = Adapter::new();

    let result = claim_unverified_account_before_link(
        &account("google", Some(verified_token())),
        Some(&adapter),
    )
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(
        adapter.deleted(),
        vec!["acc-pwd".to_string(), "acc-fb".to_string()]
    );
    assert_eq!(adapter.session_deletes(), vec![USER_ID.to_string()]);
    assert_eq!(
        adapter.verified.lock().unwrap().clone(),
        vec![USER_ID.to_string()]
    );
}

#[tokio::test]
async fn google_is_refused_when_s() {
    for token in [
        Some(id_token_with(r#"{"email_verified":false}"#)),
        None,
        Some("not-a-jwt".to_string()),
    ] {
        let adapter = Adapter::new();
        let error =
            claim_unverified_account_before_link(&account("google", token.clone()), Some(&adapter))
                .await
                .unwrap_err();
        assert!(error.contains("did not attest"), "{token:?}: {error}");
        assert!(adapter.deleted().is_empty());
        assert!(adapter.session_deletes().is_empty());
    }
}

#[tokio::test]
async fn oidc_is_refused_when_its_id_token_does_not_attest_the_email() {
    let adapter = Adapter::new();
    let error = claim_unverified_account_before_link(
        &account("oidc", Some(id_token_with(r#"{"email_verified":false}"#))),
        Some(&adapter),
    )
    .await
    .unwrap_err();
    assert!(error.contains("did not attest"), "{error}");
    assert!(adapter.deleted().is_empty());
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn oidc_may_claim_an_unverified_account_when_email_trust_is_enabled() {
    let adapter = Adapter {
        trust_email: true,
        ..Adapter::new()
    };
    let result = claim_unverified_account_before_link(
        &account("oidc", Some(id_token_with(r#"{"email_verified":false}"#))),
        Some(&adapter),
    )
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(adapter.deleted().len(), 2);
    assert_eq!(adapter.session_deletes(), vec![USER_ID.to_string()]);
}

#[tokio::test]
async fn facebook_into_an_unverified_placeholder_claims_it() {
    let adapter = Adapter::new().with_accounts(vec![
        existing("acc-pwd", "credential"),
        existing("acc-g", "google"),
    ]);
    let result =
        claim_unverified_account_before_link(&account("facebook", None), Some(&adapter)).await;
    assert_eq!(result, Ok(()));
    assert_eq!(
        adapter.deleted(),
        vec!["acc-pwd".to_string(), "acc-g".to_string()]
    );
    assert!(!adapter.deleted().contains(&"acc-new".to_string()));
    assert_eq!(adapter.session_deletes(), vec![USER_ID.to_string()]);
}

#[tokio::test]
async fn an_unknown_provider_into_an_unverified_placeholder_is_refused() {
    let adapter = Adapter::new();
    let error = claim_unverified_account_before_link(&account("github", None), Some(&adapter))
        .await
        .unwrap_err();
    assert!(error.contains("email-attesting"), "{error}");
    assert!(adapter.deleted().is_empty());
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn is_a_no_op_for_a_user_with_no_accounts_yet_fresh_sign_up() {
    let adapter = Adapter::new()
        .with_accounts(Vec::new())
        .with_user(fresh_user(Some(CreatedAt::Millis(NOW_MS))));
    let result =
        claim_unverified_account_before_link(&account("facebook", None), Some(&adapter)).await;
    assert_eq!(result, Ok(()));
    assert!(adapter.deleted().is_empty());
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn refuses_an_unknown_provider_into_an_old_user_with_no_accounts_placeholder() {
    let adapter = Adapter::new().with_accounts(Vec::new());
    let error = claim_unverified_account_before_link(&account("github", None), Some(&adapter))
        .await
        .unwrap_err();
    assert!(error.contains("email-attesting"), "{error}");
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn google_claim_of_an_old_user_with_no_accounts_only_revokes_sessions() {
    let adapter = Adapter::new().with_accounts(Vec::new());
    let result = claim_unverified_account_before_link(
        &account("google", Some(verified_token())),
        Some(&adapter),
    )
    .await;
    assert_eq!(result, Ok(()));
    assert!(adapter.deleted().is_empty());
    assert_eq!(adapter.session_deletes(), vec![USER_ID.to_string()]);
}

#[tokio::test]
async fn fails_closed_when_createdat_is_missing() {
    let adapter = Adapter::new()
        .with_accounts(Vec::new())
        .with_user(fresh_user(None));
    let error = claim_unverified_account_before_link(&account("github", None), Some(&adapter))
        .await
        .unwrap_err();
    assert!(error.contains("email-attesting"), "{error}");
}

#[tokio::test]
async fn accepts_an_iso_string_createdat_for_a_fresh_user() {
    let adapter = Adapter::new()
        .with_accounts(Vec::new())
        .with_user(fresh_user(Some(CreatedAt::Iso(
            "2027-01-15T08:00:00.000Z".to_string(),
        ))));
    // NOW_MS is 2027-01-15T08:00:00.000Z: the user was created just now.
    let result =
        claim_unverified_account_before_link(&account("facebook", None), Some(&adapter)).await;
    assert_eq!(result, Ok(()));
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn is_a_no_op_for_an_already_verified_user() {
    let adapter = Adapter::new().with_user(Some(ClaimUser {
        id: USER_ID.to_string(),
        email: None,
        email_verified: true,
        created_at: None,
    }));
    let result =
        claim_unverified_account_before_link(&account("google", None), Some(&adapter)).await;
    assert_eq!(result, Ok(()));
    assert_eq!(*adapter.find_accounts_calls.lock().unwrap(), 0);
    assert!(adapter.session_deletes().is_empty());
}

#[tokio::test]
async fn is_a_no_op_for_the_credential_provider_itself() {
    let result =
        claim_unverified_account_before_link::<Adapter>(&account("credential", None), None).await;
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn fails_closed_without_an_endpoint_context() {
    let result =
        claim_unverified_account_before_link::<Adapter>(&account("google", None), None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn fails_closed_when_the_user_does_not_exist() {
    let adapter = Adapter::new().with_user(None);
    let result =
        claim_unverified_account_before_link(&account("google", None), Some(&adapter)).await;
    assert!(result.is_err());
    assert!(adapter.deleted().is_empty());
}

#[tokio::test]
async fn fails_closed_a_cleanup_failure_propagates_so_the_link_is_refused() {
    let adapter = Adapter::new();
    *adapter.fail_next_delete.lock().unwrap() = Some("db down".to_string());
    let error = claim_unverified_account_before_link(
        &account("google", Some(verified_token())),
        Some(&adapter),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "db down");
}
