//! Port of `packages/database/__tests__/user-repository.test.ts` (P3.3). The mocked
//! `findFirst` / `insert` chain becomes real rows in a migrated SQLite file; the asserted
//! insert values are read back from `User` and `Account`.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::user::{
    InsertWithCredentialInput, InsertWithCredentialResult, find_id_by_email, insert_with_credential,
};
use chatbotx_db::repo::{self, find_one, models};
use common::fresh_db;
use repo_common::{ok, setup};

const TENANT: i64 = 7;

async fn db_with_tenant() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        ins!(tx, "Tenant", { "id" => TENANT })?;
        ins!(tx, "Tenant", { "id" => 8_i64 })?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/user-repository.test.ts:31 "userRepository > finds an id within the supplied tenant"
#[tokio::test]
async fn user_repository_finds_an_id_within_the_supplied_tenant() {
    let db = db_with_tenant().await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => 11_i64, "email" => "admin@example.com", "tenantId" => TENANT })?;
        // Same email in another tenant: not returned.
        ins!(tx, "User", { "id" => 12_i64, "email" => "admin@example.com", "tenantId" => 8_i64 })?;
        Ok(())
    })
    .await;

    let id = ok(repo::read(&db, |c| find_id_by_email(c, "admin@example.com", TENANT)).await);
    assert_eq!(id, Some(11));
    let missing = ok(repo::read(&db, |c| find_id_by_email(c, "other@example.com", TENANT)).await);
    assert_eq!(missing, None);
}

/// Port of: packages/database/__tests__/user-repository.test.ts:47 "userRepository > creates a verified credential account with a forced password change"
#[tokio::test]
async fn user_repository_creates_a_verified_credential_account_with_a_forced_password_change() {
    let db = db_with_tenant().await;
    let result = ok(insert_with_credential(
        &db,
        InsertWithCredentialInput {
            email: "new@example.com".to_owned(),
            name: Some("New User".to_owned()),
            hashed_password: "hashed-password".to_owned(),
            tenant_id: TENANT,
        },
    )
    .await);
    assert!(result.created);

    let user_id = result.user_id;
    let user: models::User = ok(repo::read(&db, move |c| {
        find_one::<models::User>(c, "\"id\" = ?1", &[&user_id])
    })
    .await)
    .expect("user row");
    assert_eq!(user.email, "new@example.com");
    assert!(user.email_verified);
    assert!(user.must_change_password);
    assert_eq!(user.name.as_deref(), Some("New User"));
    assert_eq!(user.tenant_id, TENANT);

    let accounts: Vec<models::Account> = ok(repo::read(&db, move |c| {
        repo::find_many::<models::Account>(c, "WHERE \"userId\" = ?1", &[&user_id])
    })
    .await);
    assert_eq!(accounts.len(), 1);
    let account = &accounts[0];
    assert_eq!(account.account_id, user_id.to_string());
    assert_eq!(account.password.as_deref(), Some("hashed-password"));
    assert_eq!(account.provider_id, "credential");
    assert_eq!(account.tenant_id, TENANT);
    assert_eq!(account.user_id, user_id);
}

/// Port of: packages/database/__tests__/user-repository.test.ts:80 "userRepository > returns the concurrent winner when the tenant-scoped email is taken"
///
/// The mocked unique violation is a real one: the concurrent winner's row exists first.
#[tokio::test]
async fn user_repository_returns_the_concurrent_winner_when_the_tenant_scoped_email_is_taken() {
    let db = db_with_tenant().await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => 21_i64, "email" => "raced@example.com", "tenantId" => TENANT })?;
        Ok(())
    })
    .await;

    let result = ok(insert_with_credential(
        &db,
        InsertWithCredentialInput {
            email: "raced@example.com".to_owned(),
            name: None,
            hashed_password: "hashed-password".to_owned(),
            tenant_id: TENANT,
        },
    )
    .await);
    assert_eq!(
        result,
        InsertWithCredentialResult {
            user_id: 21,
            created: false
        }
    );
    // The failed transaction left no account behind.
    let accounts = repo_common::scalar_i64(&db, "SELECT count(*) FROM \"Account\"", vec![]).await;
    assert_eq!(accounts, Some(0));
}
