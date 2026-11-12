//! Port of `packages/database/__tests__/connect-session-cancel-pending.test.ts` (P3.2).
//! The captured builder calls become real rows of a migrated SQLite file (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::connect_session as cs;
use chatbotx_db::repo::{self, now_ms};
use repo_common::csess::seed_session;
use repo_common::ok;

async fn seeded() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, |tx| {
        for (id, ws, actor, provider, status) in [
            (1_i64, 2001_i64, 1001_i64, "googleAds", "pending"),
            (2, 2001, 1001, "googleAds", "pending"),
            (3, 2001, 1001, "googleAds", "authorized"),
            (4, 2001, 1001, "messenger", "pending"),
            (5, 2002, 1002, "googleAds", "pending"),
        ] {
            seed_session(tx, id, ws, actor, &format!("n{id}"))?;
            tx.execute(r#"UPDATE "ConnectSession" SET "provider" = ?1, "status" = ?2, "encryptedAuth" = '{"iv":"x"}' WHERE "id" = ?3"#, chatbotx_db::rusqlite::params![provider, status, id])?;
        }
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/connect-session-cancel-pending.test.ts:59 "connectSessionRepository.cancelPendingByProvider > scopes the update to the workspace, the provider and pending only"
#[tokio::test]
async fn connect_session_repository_cancel_pending_by_provider_scopes_the_update_to_the_workspace_the_provider_and_pending_only()
 {
    let db = seeded().await;
    assert_eq!(
        ok(repo::write(&db, |tx| cs::cancel_pending_by_provider(
            tx,
            2001,
            "googleAds",
            now_ms()
        ))
        .await),
        2
    );
    let rows = repo_common::rows(
        &db,
        r#"SELECT "id", "status" FROM "ConnectSession" ORDER BY "id""#,
        vec![],
    )
    .await;
    let statuses: Vec<String> = rows
        .iter()
        .map(|r| match &r[1] {
            chatbotx_db::rusqlite::types::Value::Text(t) => t.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        statuses,
        vec!["cancelled", "cancelled", "authorized", "pending", "pending"]
    );
}

/// Port of: packages/database/__tests__/connect-session-cancel-pending.test.ts:78 "connectSessionRepository.cancelPendingByProvider > marks rows cancelled, consumed, and clears the ciphertext"
#[tokio::test]
async fn connect_session_repository_cancel_pending_by_provider_marks_rows_cancelled_consumed_and_clears_the_ciphertext()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| {
        cs::cancel_pending_by_provider(tx, 2001, "googleAds", now_ms())
    })
    .await);
    let s = ok(repo::read(&db, |c| cs::find_by_id(c, 1)).await).expect("row");
    assert_eq!(s.status, "cancelled");
    assert!(s.consumed_at.is_some());
    assert_eq!(s.encrypted_auth, None);
    assert_eq!(
        ok(repo::write(&db, |tx| cs::cancel_pending_by_provider(
            tx,
            2001,
            "googleAds",
            now_ms()
        ))
        .await),
        0
    );
}
