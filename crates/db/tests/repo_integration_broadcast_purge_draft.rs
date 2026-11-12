//! Port of `packages/database/__tests__/integration/broadcast-purge-draft.test.ts` (P3.2).
//! Runs against a migrated SQLite temp file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::broadcast_purge::{
    self as purge, PurgeInput, PurgeOutcome, PurgeStopReason,
};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::bcast::{insert, recipients};
use repo_common::ok;

async fn new_broadcast_with_recipients(status: &'static str, n: i64) -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::setup(&db, move |tx| {
        repo_common::insert_inbox(tx, 3100, 2001, "webchat", "purge-draft")?;
        insert(tx, 1, "purge-draft", status)?;
        recipients(tx, 1, 3100, 100, n)?;
        Ok(())
    })
    .await;
    db
}

fn input() -> PurgeInput {
    PurgeInput {
        broadcast_id: 1,
        chunk_size: 2,
        inter_chunk_delay_ms: 0,
        max_run_duration_ms: 30_000,
        max_chunks: None,
        only_while_draft: true,
    }
}

/// Port of: packages/database/__tests__/integration/broadcast-purge-draft.test.ts:114 "purgeBroadcastRecipients against Postgres > drains a draft's recipients chunk by chunk"
#[tokio::test]
async fn purge_broadcast_recipients_against_postgres_drains_a_draft_s_recipients_chunk_by_chunk() {
    let db = new_broadcast_with_recipients("draft", 3).await;
    let out = ok(purge::purge_broadcast_recipients(&db, &input()).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 3,
            stop_reason: PurgeStopReason::Drained
        }
    );
    assert!(!ok(repo::read(&db, |c| purge::has_broadcast_recipients(
        c, 1
    ))
    .await));
}

/// Port of: packages/database/__tests__/integration/broadcast-purge-draft.test.ts:129 "purgeBroadcastRecipients against Postgres > leaves a broadcast that is no longer a draft untouched"
#[tokio::test]
async fn purge_broadcast_recipients_against_postgres_leaves_a_broadcast_that_is_no_longer_a_draft_untouched()
 {
    let db = new_broadcast_with_recipients("scheduled", 2).await;
    let out = ok(purge::purge_broadcast_recipients(&db, &input()).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 0,
            stop_reason: PurgeStopReason::Drained
        }
    );
    assert!(ok(repo::read(&db, |c| purge::has_broadcast_recipients(
        c, 1
    ))
    .await));
}
