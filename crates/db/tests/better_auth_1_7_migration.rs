//! Port of `packages/database/__tests__/better-auth-1-7-migration.test.ts`
//! (P2.3 spec section 5.1, rows 3.1-3.4). Class C: runs against a migrated SQLite file.
//!
//! The original reads the Postgres migration file `20261030151215_better_auth_1_7`. The
//! consolidated SQLite schema has no such file, so each case checks the end state that the
//! statement produced (user-approved replacement checks, DECISIONS 2026-11-11 "P2.3 disputed tests").

mod common;

use common::{column, columns, fresh_db, seed, SEED_CLIENT_ID};
use chatbotx_db::rusqlite::params;

/// Port of: packages/database/__tests__/better-auth-1-7-migration.test.ts:21 "better-auth 1.7 migration > casts Verification.id to text before anything depends on it"
///
/// Replacement: `Verification.id` is a TEXT primary key.
#[tokio::test]
async fn verification_id_is_text() {
    let db = fresh_db().await;
    let id = column(&db, "Verification", "id").await;
    assert_eq!(id.decl_type, "TEXT");
    assert_eq!(id.pk, 1);
}

/// Port of: packages/database/__tests__/better-auth-1-7-migration.test.ts:27 "better-auth 1.7 migration > deletes declined consents before dropping consentGiven"
///
/// Replacement: `OAuthConsent` has no column `consentGiven`.
#[tokio::test]
async fn oauth_consent_has_no_consent_given_column() {
    let db = fresh_db().await;
    let cols = columns(&db, "OAuthConsent").await;
    assert!(!cols.is_empty(), "OAuthConsent is missing");
    assert!(cols.iter().all(|c| c.name != "consentGiven"), "OAuthConsent still has consentGiven");
}

/// Port of: packages/database/__tests__/better-auth-1-7-migration.test.ts:39 "better-auth 1.7 migration > converts OAuthConsent.scopes from a space-separated string to text[]"
///
/// Replacement: `scopes` is `TEXT NOT NULL` with the JSON-array CHECK; a space-separated
/// string is refused, a JSON array is stored.
#[tokio::test]
async fn oauth_consent_scopes_is_a_json_array() {
    let db = fresh_db().await;
    let scopes = column(&db, "OAuthConsent", "scopes").await;
    assert_eq!(scopes.decl_type, "TEXT");
    assert!(scopes.notnull);
    seed(&db).await;

    let refused = db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "OAuthConsent" ("id", "clientId", "scopes") VALUES (7001, ?1, 'openid profile')"#,
                params![SEED_CLIENT_ID],
            )?;
            Ok(())
        })
        .await
        .expect_err("a space-separated scopes string must be refused");
    assert_eq!(refused.sqlstate(), Some("23514"));

    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "OAuthConsent" ("id", "clientId", "scopes") VALUES (7002, ?1, '["openid","profile"]')"#,
            params![SEED_CLIENT_ID],
        )?;
        Ok(())
    })
    .await
    .expect("a JSON array of scopes must be stored");
}

/// Port of: packages/database/__tests__/better-auth-1-7-migration.test.ts:51 "better-auth 1.7 migration > allows a consent without a user"
///
/// Adapted: `OAuthConsent.userId` is nullable and a consent without a user is stored.
#[tokio::test]
async fn allows_a_consent_without_a_user() {
    let db = fresh_db().await;
    assert!(!column(&db, "OAuthConsent", "userId").await.notnull);
    seed(&db).await;

    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "OAuthConsent" ("id", "clientId", "userId", "scopes") VALUES (7003, ?1, NULL, '["openid"]')"#,
            params![SEED_CLIENT_ID],
        )?;
        Ok(())
    })
    .await
    .expect("a consent with userId NULL must be stored");
}
