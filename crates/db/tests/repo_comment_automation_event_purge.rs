//! Port of `packages/database/__tests__/comment-automation-event-purge.test.ts` (P3.2).
//! The rendered SQL checks become the effect on real rows of a migrated SQLite file
//! (adapted): only failed rows past the window go, oldest first, in full chunks.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::chunked_purge::ChunkedPurgeStopReason;
use chatbotx_db::repo::comment_automation_event::{
    PurgeOptions, purge_failed_comment_automation_events,
};
use repo_common::{NOW, ok};

const DAY: i64 = 86_400_000;

fn options(chunk_size: i64) -> PurgeOptions {
    PurgeOptions {
        retention_days: 30,
        chunk_size,
        inter_chunk_delay_ms: 0,
        max_chunks: 10,
        max_run_duration_ms: None,
    }
}

/// Events `(id, status, createdAt)`.
async fn seeded(rows: Vec<(i64, &'static str, i64)>) -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    repo_common::setup(&db, move |tx| {
        ins!(tx, "CommentAutomation", { "id" => 1_i64, "name" => "a", "workspaceId" => 2001_i64 })?;
        for (id, status, created) in rows {
            ins!(tx, "CommentAutomationEvent", { "id" => id, "workspaceId" => 2001_i64, "automationId" => 1_i64, "postId" => "p", "commentId" => format!("c{id}"), "replyChannel" => "public", "replyType" => "text", "status" => status, "occurredAt" => created, "createdAt" => created })?;
        }
        Ok(())
    })
    .await;
    db
}

async fn left(db: &chatbotx_db::Db) -> Vec<i64> {
    repo_common::rows(
        db,
        r#"SELECT "id" FROM "CommentAutomationEvent" ORDER BY "id""#,
        vec![],
    )
    .await
    .into_iter()
    .map(|r| match r[0] {
        chatbotx_db::rusqlite::types::Value::Integer(i) => i,
        _ => 0,
    })
    .collect()
}

/// Port of: packages/database/__tests__/comment-automation-event-purge.test.ts:48 "purgeFailedCommentAutomationEvents > deletes only failed rows past the retention window"
#[tokio::test]
async fn purge_failed_comment_automation_events_deletes_only_failed_rows_past_the_retention_window()
{
    let db = seeded(vec![
        (1, "failed", NOW - 31 * DAY),
        (2, "sent", NOW - 31 * DAY),
        (3, "failed", NOW - 29 * DAY),
    ])
    .await;
    let out = ok(purge_failed_comment_automation_events(&db, options(1000), NOW).await);
    assert_eq!(out, (1, ChunkedPurgeStopReason::Drained));
    assert_eq!(left(&db).await, vec![2, 3]);
}

/// Port of: packages/database/__tests__/comment-automation-event-purge.test.ts:62 "purgeFailedCommentAutomationEvents > takes the oldest failed rows first, skipping locked ones"
#[tokio::test]
async fn purge_failed_comment_automation_events_takes_the_oldest_failed_rows_first_skipping_locked_ones()
 {
    let db = seeded(vec![
        (1, "failed", NOW - 40 * DAY),
        (2, "failed", NOW - 50 * DAY),
        (3, "failed", NOW - 45 * DAY),
    ])
    .await;
    let mut o = options(1);
    o.max_chunks = 1;
    ok(purge_failed_comment_automation_events(&db, o, NOW).await);
    // The oldest (2) went first.
    assert_eq!(left(&db).await, vec![1, 3]);
}

/// Port of: packages/database/__tests__/comment-automation-event-purge.test.ts:72 "purgeFailedCommentAutomationEvents > keeps chunking while a full chunk comes back"
#[tokio::test]
async fn purge_failed_comment_automation_events_keeps_chunking_while_a_full_chunk_comes_back() {
    let rows: Vec<(i64, &'static str, i64)> =
        (1..=7).map(|i| (i, "failed", NOW - 31 * DAY)).collect();
    let db = seeded(rows).await;
    let out = ok(purge_failed_comment_automation_events(&db, options(3), NOW).await);
    assert_eq!(out, (7, ChunkedPurgeStopReason::Drained));
    assert!(left(&db).await.is_empty());
}
