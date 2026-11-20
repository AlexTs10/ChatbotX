//! Port of `packages/business/src/connection/__tests__/record-refreshed-auth.test.ts`
//! over a migrated database: the mocked lookup and `recordAuthSaved` are the
//! one guarded `UPDATE` of `record_refreshed_auth`.

mod common;

use chatbotx_business::channel_token_refresh::record_refreshed_auth;
use chatbotx_db::rusqlite::params;
use common::{WS, inbox, setup, two_workspaces};
use serde_json::{Value, json};

const EXPIRES_AT_MS: i64 = 1_791_590_400_000; // 2026-10-10T00:00:00.000Z

fn oauth2_auth() -> Value {
    json!({
        "authType": "oauth2",
        "clientId": "client-1",
        "clientSecret": "secret-1",
        "redirectUrl": "https://example.com",
        "tokens": { "accessToken": "token-1", "expiresAt": "2026-10-10T00:00:00.000Z" },
    })
}

async fn db_with_connection(status: &'static str) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        inbox(tx, 11, WS, "messenger")?;
        let (reason, disconnected_at): (Option<&str>, Option<i64>) = match status {
            "connected" => (None, None),
            "disconnected" => (Some("manual"), Some(1)),
            _ => (Some("refresh_failed"), None),
        };
        tx.execute(
            r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "channel", "inboxId",
                   "sourceId", "displayName", "status", "statusReason", "disconnectedAt", "lastError")
               VALUES (1, ?1, 'messenger', 'channel', 'messenger', 11, 'page-1', 'Page', ?2, ?3, ?4, 'boom')"#,
            params![WS, status, reason, disconnected_at],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn connection(db: &chatbotx_db::Db) -> (String, Option<String>, Option<i64>) {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT "status", "statusReason", "authExpiresAt" FROM "Connection" WHERE "id" = 1"#,
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    })
    .await
    .expect("read connection")
}

async fn record(db: &chatbotx_db::Db, source_id: &'static str) -> usize {
    db.write(move |tx| {
        Ok(record_refreshed_auth(
            tx,
            WS,
            "messenger",
            source_id,
            &oauth2_auth(),
        )?)
    })
    .await
    .expect("record")
}

#[tokio::test]
async fn heals_a_degraded_connection_and_sets_auth_expires_at_from_the_refreshed_oauth2_auth() {
    let db = db_with_connection("degraded").await;
    assert_eq!(record(&db, "page-1").await, 1);
    assert_eq!(
        connection(&db).await,
        ("connected".into(), None, Some(EXPIRES_AT_MS))
    );
}

#[tokio::test]
async fn no_ops_when_no_connection_row_exists_for_this_provider_source_id() {
    let db = db_with_connection("degraded").await;
    assert_eq!(record(&db, "page-2").await, 0);
    assert_eq!(connection(&db).await.0, "degraded");
}

#[tokio::test]
async fn no_ops_when_the_connection_is_inactive() {
    let db = db_with_connection("disconnected").await;
    assert_eq!(record(&db, "page-1").await, 0);
    assert_eq!(
        connection(&db).await,
        ("disconnected".into(), Some("manual".into()), None)
    );
}

#[tokio::test]
async fn tolerates_a_connection_that_went_inactive_before_the_transition() {
    // The status guard is in the UPDATE itself, so a lost race is a no-op,
    // never an error that fails the auth save.
    let db = db_with_connection("needs_reauth").await;
    assert_eq!(record(&db, "page-1").await, 0);
    assert_eq!(connection(&db).await.0, "needs_reauth");
}

#[tokio::test]
async fn runs_in_the_caller_supplied_transaction() {
    let db = db_with_connection("connected").await;
    let after = db
        .write(|tx| {
            record_refreshed_auth(tx, WS, "messenger", "page-1", &oauth2_auth())?;
            Ok(tx.query_row(
                r#"SELECT "authExpiresAt" FROM "Connection" WHERE "id" = 1"#,
                [],
                |r| r.get::<_, Option<i64>>(0),
            )?)
        })
        .await
        .expect("tx");
    assert_eq!(after, Some(EXPIRES_AT_MS));
}
