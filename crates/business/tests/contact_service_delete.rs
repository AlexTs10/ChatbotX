//! Port of `packages/business/__tests__/contact-service-delete.test.ts`
//! (P3.6-D2). Adapted: the spied queries, transaction and
//! `messageCleanupService.record` become stored rows (the `MessageCleanup`
//! tombstones); the chunk count is not observable, the tombstones of all
//! chunks are. Not applicable: the five "quota release" tests (no quota
//! enforcement / usage counters in the Rust port yet).

mod common;

use chatbotx_business::contact::delete;
use common::{
    OTHER_WS, TestDb, WS, contact_inbox, contact_with, conversation, inbox, rows, scalar_i64,
    setup, two_workspaces,
};
use serde_json::Value;

const INBOX: i64 = 3002;
/// 2026-01-01T00:00:00Z / 2026-02-01T00:00:00Z.
const INBOX_CREATED_AT: i64 = 1_767_225_600_000;
const FIRST_INTERACTION_AT: i64 = 1_769_904_000_000;

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, INBOX, WS, "messenger")?;
        Ok(())
    })
    .await;
    db
}

async fn tombstones(db: &TestDb) -> Vec<serde_json::Map<String, Value>> {
    rows(
        db,
        r#"SELECT "contactId", "contactInboxId", "inboxId", "sourceId", "conversationIds", "sinceTime", "status"
           FROM "MessageCleanup" WHERE "workspaceId" = ?1 ORDER BY "contactInboxId""#,
        WS,
    )
    .await
}

#[tokio::test]
async fn chunks_deletes_and_records_tombstones_atomically_per_chunk() {
    let db = db().await;
    setup(&db, |tx| {
        for i in 0..120_i64 {
            contact_with(tx, 10_000 + i, WS, Some("C"), None, None)?;
            contact_inbox(
                tx,
                20_000 + i,
                10_000 + i,
                INBOX,
                "messenger",
                &format!("source-{i}"),
            )?;
        }
        Ok(())
    })
    .await;
    let ids: Vec<i64> = (0..120).map(|i| 10_000 + i).collect();
    let deleted = delete(&db, WS, &ids).await.expect("deleted");
    assert_eq!(deleted.len(), 120);
    let left = scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "Contact" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(left, Some(0));
    assert_eq!(tombstones(&db).await.len(), 120);
}

#[tokio::test]
async fn tombstone_entries_snapshot_inbox_identity_conversations_and_since_time() {
    let db = db().await;
    setup(&db, |tx| {
        contact_with(tx, 10_000, WS, Some("C0"), None, None)?;
        contact_with(tx, 10_001, WS, Some("C1"), None, None)?;
        for (ci, source) in [(20_000, "source-0-0"), (20_001, "source-0-1")] {
            contact_inbox(tx, ci, 10_000, INBOX, "messenger", source)?;
            tx.execute(
                r#"UPDATE "ContactInbox" SET "firstInteractionAt" = ?2, "createdAt" = ?3 WHERE "id" = ?1"#,
                chatbotx_db::rusqlite::params![ci, FIRST_INTERACTION_AT, INBOX_CREATED_AT],
            )?;
        }
        conversation(tx, 30_001, WS, 10_000)?;
        Ok(())
    })
    .await;
    delete(&db, WS, &[10_000, 10_001]).await.expect("deleted");
    let rows = tombstones(&db).await;
    // A contact without inboxes leaves no tombstone.
    assert_eq!(rows.len(), 2);
    for (row, (ci, source)) in rows
        .iter()
        .zip([(20_000, "source-0-0"), (20_001, "source-0-1")])
    {
        assert_eq!(row["contactId"], 10_000);
        assert_eq!(row["contactInboxId"], ci);
        assert_eq!(row["inboxId"], INBOX);
        assert_eq!(row["sourceId"], source);
        assert_eq!(row["sinceTime"], FIRST_INTERACTION_AT);
        assert_eq!(row["status"], "pending");
        let ids: Value =
            serde_json::from_str(row["conversationIds"].as_str().expect("json")).expect("json");
        assert_eq!(ids, serde_json::json!(["30001"]));
    }
}

#[tokio::test]
async fn falls_back_to_the_inbox_created_at_when_there_is_no_first_interaction() {
    let db = db().await;
    setup(&db, |tx| {
        contact_with(tx, 10_000, WS, Some("C0"), None, None)?;
        contact_inbox(tx, 20_000, 10_000, INBOX, "messenger", "source-0-0")?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "createdAt" = ?2 WHERE "id" = ?1"#,
            chatbotx_db::rusqlite::params![20_000, INBOX_CREATED_AT],
        )?;
        Ok(())
    })
    .await;
    delete(&db, WS, &[10_000]).await.expect("deleted");
    let rows = tombstones(&db).await;
    assert_eq!(rows[0]["sinceTime"], INBOX_CREATED_AT);
    assert_eq!(rows[0]["conversationIds"], "[]");
}

#[tokio::test]
async fn returns_early_without_touching_the_database_when_nothing_matches() {
    let db = db().await;
    setup(&db, |tx| {
        // A contact of another workspace is never deleted.
        contact_with(tx, 10_404, OTHER_WS, Some("Other"), None, None)?;
        Ok(())
    })
    .await;
    let deleted = delete(&db, WS, &[10_404, 10_405]).await.expect("deleted");
    assert!(deleted.is_empty());
    assert!(tombstones(&db).await.is_empty());
    let kept = scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "Contact" WHERE "id" = ?1"#,
        10_404,
    )
    .await;
    assert_eq!(kept, Some(1));
}
