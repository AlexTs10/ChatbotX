//! Port of `packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts`
//! (P3.3). Postgres runs become a fresh migrated SQLite file per test. Two client
//! connections become two concurrent `repo::write` jobs: the one writer serializes them,
//! which is what the advisory lock did; the asserted outcome is the same (`adapted`).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use std::sync::mpsc;
use std::time::Duration;

use chatbotx_db::repo::mobile_refresh_token::{
    NewMobileRefreshToken, delete_spent_in_family, insert, lock_family_by_hash,
    revoke_family_by_hash, spend_active,
};
use chatbotx_db::repo::models::MobileRefreshToken;
use chatbotx_db::repo::{self, find_many, now_ms};
use common::fresh_db;
use repo_common::{ok, setup};

const USER: i64 = 1001;

async fn db_with_user() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => USER, "email" => "mobile-refresh@example.test" })?;
        Ok(())
    })
    .await;
    db
}

fn token(
    token_hash: &str,
    family_id: &str,
    expires_at: i64,
    rotated_at: Option<i64>,
) -> NewMobileRefreshToken {
    NewMobileRefreshToken {
        user_id: USER,
        device_id: "device-a".to_owned(),
        family_id: family_id.to_owned(),
        token_hash: token_hash.to_owned(),
        expires_at,
        rotated_at,
        revoked: None,
    }
}

async fn add(db: &common::TestDb, t: NewMobileRefreshToken) {
    ok(repo::write(db, move |tx| insert(tx, &t)).await);
}

async fn family(db: &common::TestDb, family_id: &str) -> Vec<MobileRefreshToken> {
    let family_id = family_id.to_owned();
    ok(repo::read(db, move |c| {
        find_many::<MobileRefreshToken>(c, "WHERE \"familyId\" = ?1 ORDER BY \"id\"", &[&family_id])
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts:85 "mobileRefreshTokenRepository against Postgres > allows exactly one concurrent active-token spend"
#[tokio::test]
async fn mobile_refresh_token_repository_against_postgres_allows_exactly_one_concurrent_active_token_spend()
 {
    let db = db_with_user().await;
    add(
        &db,
        token("concurrent", "family-a", now_ms() + 60_000, None),
    )
    .await;
    let now = now_ms();
    let (left, right) = tokio::join!(
        repo::write(&db, move |tx| spend_active(tx, "concurrent", now)),
        repo::write(&db, move |tx| spend_active(tx, "concurrent", now)),
    );
    let spent = [ok(left), ok(right)];
    assert_eq!(spent.iter().filter(|s| s.is_some()).count(), 1);
}

/// Port of: packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts:96 "mobileRefreshTokenRepository against Postgres > revokes every token in a replayed family"
#[tokio::test]
async fn mobile_refresh_token_repository_against_postgres_revokes_every_token_in_a_replayed_family()
{
    let db = db_with_user().await;
    add(
        &db,
        token("replayed", "family-replayed", now_ms() + 60_000, None),
    )
    .await;
    add(
        &db,
        token("successor", "family-replayed", now_ms() + 60_000, None),
    )
    .await;
    add(
        &db,
        token("bystander", "family-other", now_ms() + 60_000, None),
    )
    .await;

    let now = now_ms();
    ok(repo::write(&db, move |tx| revoke_family_by_hash(tx, "replayed", now)).await);

    let rows = family(&db, "family-replayed").await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.revoked.is_some()));
    assert!(
        family(&db, "family-other")
            .await
            .iter()
            .all(|r| r.revoked.is_none())
    );
}

/// Port of: packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts:112 "mobileRefreshTokenRepository against Postgres > serializes replay revocation against successor rotation"
///
/// The replay job holds the writer (as it held the family lock) until the test releases
/// it; the successor rotation is queued behind it, then finds its token revoked.
#[tokio::test]
async fn mobile_refresh_token_repository_against_postgres_serializes_replay_revocation_against_successor_rotation()
 {
    let db = std::sync::Arc::new(db_with_user().await);
    let family_id = "family-race";
    add(
        &db,
        token(
            "replayed",
            family_id,
            now_ms() + 60_000,
            Some(now_ms() - 60_000),
        ),
    )
    .await;
    add(&db, token("active", family_id, now_ms() + 60_000, None)).await;

    let (locked_tx, locked_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let replay_db = std::sync::Arc::clone(&db);
    let replay = tokio::spawn(async move {
        repo::write(&replay_db, move |tx| {
            lock_family_by_hash(tx, "replayed")?;
            locked_tx.send(()).expect("signal locked");
            release_rx.recv().expect("wait for release");
            revoke_family_by_hash(tx, "replayed", now_ms())
        })
        .await
    });

    tokio::task::spawn_blocking(move || locked_rx.recv().expect("replay locked"))
        .await
        .expect("join");

    let successor_db = std::sync::Arc::clone(&db);
    let successor = tokio::spawn(async move {
        repo::write(&successor_db, move |tx| {
            lock_family_by_hash(tx, "active")?;
            let spent = spend_active(tx, "active", now_ms())?;
            if spent.is_none() {
                return Ok(false);
            }
            insert(
                tx,
                &token("successor", "family-race", now_ms() + 60_000, None),
            )?;
            Ok(true)
        })
        .await
    });

    // The successor is queued behind the replay job (the "waits for the family lock" check).
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!successor.is_finished());
    release_tx.send(()).expect("release");
    ok(replay.await.expect("join replay"));
    let successor_rotated = ok(successor.await.expect("join successor"));

    assert!(!successor_rotated);
    let rows = family(&db, family_id).await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.revoked.is_some()));
}

/// Port of: packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts:175 "mobileRefreshTokenRepository against Postgres > does not spend an expired token"
#[tokio::test]
async fn mobile_refresh_token_repository_against_postgres_does_not_spend_an_expired_token() {
    let db = db_with_user().await;
    add(&db, token("expired", "family-a", now_ms() - 1000, None)).await;
    let now = now_ms();
    let spent = ok(repo::write(&db, move |tx| spend_active(tx, "expired", now)).await);
    assert_eq!(spent, None);
}

/// Port of: packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts:186 "mobileRefreshTokenRepository against Postgres > deletes only spent tokens older than the cutoff"
#[tokio::test]
async fn mobile_refresh_token_repository_against_postgres_deletes_only_spent_tokens_older_than_the_cutoff()
 {
    let db = db_with_user().await;
    let cutoff = now_ms() - 60_000;
    add(
        &db,
        token(
            "spent-old",
            "family-cleanup",
            now_ms() + 60_000,
            Some(cutoff - 1),
        ),
    )
    .await;
    add(
        &db,
        token(
            "spent-new",
            "family-cleanup",
            now_ms() + 60_000,
            Some(cutoff + 1),
        ),
    )
    .await;

    ok(repo::write(&db, move |tx| {
        delete_spent_in_family(tx, "family-cleanup", cutoff)
    })
    .await);

    let rows = family(&db, "family-cleanup").await;
    assert_eq!(
        rows.iter()
            .map(|r| r.token_hash.as_str())
            .collect::<Vec<_>>(),
        vec!["spent-new"]
    );
}
