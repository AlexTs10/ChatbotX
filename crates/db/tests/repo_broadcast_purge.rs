//! Port of `packages/database/__tests__/broadcast-purge.test.ts` (P3.2).
//! SQL-text cases are ported as the effect on real rows of a migrated SQLite file; the
//! loop-mechanics cases (mocked `execute` results, `Date.now` spies) drive
//! [`run_purge_loop`] with an injected clock and chunk step (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_db::repo::broadcast_purge::{
    self as purge, DEFAULT_PURGE_MAX_CHUNKS, PurgeInput, PurgeOutcome, PurgeStopReason,
};
use chatbotx_db::repo::{self, Error};
use chatbotx_db::rusqlite::{self, ffi};
use common::{fresh_db, seed};
use repo_common::bcast::{insert, recipients};
use repo_common::{int, ok};

async fn with_broadcast(status: &str, n: i64) -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    let status = status.to_owned();
    repo_common::setup(&db, move |tx| {
        repo_common::insert_inbox(tx, 3100, 2001, "webchat", "wc")?;
        insert(tx, 1, "b", &status)?;
        recipients(tx, 1, 3100, 100, n)?;
        Ok(())
    })
    .await;
    db
}

fn input(chunk_size: i64) -> PurgeInput {
    PurgeInput {
        broadcast_id: 1,
        chunk_size,
        inter_chunk_delay_ms: 0,
        max_run_duration_ms: 60_000,
        max_chunks: None,
        only_while_draft: false,
    }
}

async fn recipients_left(db: &Db) -> i64 {
    repo_common::scalar_i64(
        db,
        r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "broadcastId" = 1"#,
        vec![],
    )
    .await
    .unwrap_or(0)
}

fn ok_chunk(n: usize) -> repo::Result<usize> {
    Ok(n)
}

/// A fake chunk step returning `results` in order (then empty chunks).
fn scripted(results: Vec<repo::Result<usize>>) -> Arc<Mutex<Vec<repo::Result<usize>>>> {
    let mut r = results;
    r.reverse();
    Arc::new(Mutex::new(r))
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:59 "listPurgeableBroadcasts > selects soft-deleted, non-sending broadcasts ordered oldest-deleted-first, bound limit"
#[tokio::test]
async fn list_purgeable_broadcasts_selects_soft_deleted_non_sending_broadcasts_ordered_oldest_deleted_first_bound_limit()
 {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::setup(&db, |tx| {
        for (id, status, deleted) in [
            (1, "sent", Some(30)),
            (2, "sending", Some(10)),
            (3, "draft", Some(20)),
            (4, "sent", None),
            (5, "sent", Some(20)),
        ] {
            insert(tx, id, "b", status)?;
            tx.execute(
                r#"UPDATE "Broadcast" SET "deletedAt" = ?1 WHERE "id" = ?2"#,
                rusqlite::params![deleted, id],
            )?;
        }
        Ok(())
    })
    .await;
    assert_eq!(
        ok(repo::read(&db, |c| purge::list_purgeable_broadcasts(c, 50)).await),
        vec![3, 5, 1]
    );
    assert_eq!(
        ok(repo::read(&db, |c| purge::list_purgeable_broadcasts(c, 1)).await),
        vec![3]
    );
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:74 "purgeBroadcastRecipients > issues a composite-key SKIP LOCKED chunk delete with bound broadcastId/chunkSize"
#[tokio::test]
async fn purge_broadcast_recipients_issues_a_composite_key_skip_locked_chunk_delete_with_bound_broadcast_id_chunk_size()
 {
    let db = with_broadcast("sent", 1).await;
    // Another broadcast's recipients are not touched.
    repo_common::setup(&db, |tx| {
        insert(tx, 2, "other", "sent")?;
        recipients(tx, 2, 3100, 500, 2)?;
        Ok(())
    })
    .await;
    let mut i = input(500);
    i.inter_chunk_delay_ms = 10;
    let out = ok(purge::purge_broadcast_recipients(&db, &i).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 1,
            stop_reason: PurgeStopReason::Drained
        }
    );
    assert_eq!(recipients_left(&db).await, 0);
    let other = repo_common::scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ContactOnBroadcast" WHERE "broadcastId" = 2"#,
        vec![],
    )
    .await;
    assert_eq!(other, Some(2));
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:95 "purgeBroadcastRecipients > with onlyWhileDraft, deletes a chunk only while the broadcast is still a draft"
#[tokio::test]
async fn purge_broadcast_recipients_with_only_while_draft_deletes_a_chunk_only_while_the_broadcast_is_still_a_draft()
 {
    let db = with_broadcast("scheduled", 1).await;
    let mut i = input(500);
    i.only_while_draft = true;
    let out = ok(purge::purge_broadcast_recipients(&db, &i).await);
    assert_eq!(out.deleted, 0);
    assert_eq!(recipients_left(&db).await, 1);
    repo_common::handover::exec(
        &db,
        r#"UPDATE "Broadcast" SET "status" = 'draft' WHERE "id" = ?1"#,
        vec![int(1)],
    )
    .await;
    let out = ok(purge::purge_broadcast_recipients(&db, &i).await);
    assert_eq!(out.deleted, 1);
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:116 "purgeBroadcastRecipients > caps each chunk at the time left in the run"
#[tokio::test]
async fn purge_broadcast_recipients_caps_each_chunk_at_the_time_left_in_the_run() {
    let seen = Arc::new(Mutex::new(Vec::<i64>::new()));
    let s = Arc::clone(&seen);
    let out = ok(purge::run_purge_loop(
        500,
        10,
        60_000,
        DEFAULT_PURGE_MAX_CHUNKS,
        || 1_000_000,
        move |remaining| {
            s.lock().unwrap().push(remaining);
            async { ok_chunk(1) }
        },
    )
    .await);
    assert_eq!(out.stop_reason, PurgeStopReason::Drained);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0] > 0 && seen[0] <= 60_000);
    // The real chunk step turns it into a `<n>ms` statement timeout literal.
    assert!(chatbotx_db::StatementTimeout::parse(&format!("{}ms", seen[0])).is_ok());
}

fn timeout_error() -> Error {
    Error::from(rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_INTERRUPT),
        None,
    ))
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:136 "purgeBroadcastRecipients > ends the run as 'deadline', keeping what was deleted, when a chunk times out"
#[tokio::test]
async fn purge_broadcast_recipients_ends_the_run_as_deadline_keeping_what_was_deleted_when_a_chunk_times_out()
 {
    let script = scripted(vec![Ok(3), Err(timeout_error())]);
    let out = ok(purge::run_purge_loop(
        3,
        0,
        60_000,
        DEFAULT_PURGE_MAX_CHUNKS,
        || 1_000_000,
        move |_| {
            let next = script.lock().unwrap().pop().unwrap_or(Ok(0));
            async move { next }
        },
    )
    .await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 3,
            stop_reason: PurgeStopReason::Deadline
        }
    );
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:158 "purgeBroadcastRecipients > propagates any other database error"
#[tokio::test]
async fn purge_broadcast_recipients_propagates_any_other_database_error() {
    let db = with_broadcast("sent", 1).await;
    db.write(|tx| Ok(tx.execute_batch(r#"DROP TABLE "ContactOnBroadcast""#)?))
        .await
        .expect("drop");
    let err = purge::purge_broadcast_recipients(&db, &input(3))
        .await
        .expect_err("must fail");
    assert!(err.message().contains("no such table"), "{err}");
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:171 "purgeBroadcastRecipients > loops chunk-by-chunk and stops with 'drained' once a chunk returns fewer rows than chunkSize"
#[tokio::test]
async fn purge_broadcast_recipients_loops_chunk_by_chunk_and_stops_with_drained_once_a_chunk_returns_fewer_rows_than_chunk_size()
 {
    let db = with_broadcast("sent", 7).await;
    let out = ok(purge::purge_broadcast_recipients(&db, &input(3)).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 7,
            stop_reason: PurgeStopReason::Drained
        }
    );
    assert_eq!(recipients_left(&db).await, 0);
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:188 "purgeBroadcastRecipients > checks the deadline BEFORE starting a chunk, so a chunk never starts once the budget is spent"
#[tokio::test]
async fn purge_broadcast_recipients_checks_the_deadline_before_starting_a_chunk_so_a_chunk_never_starts_once_the_budget_is_spent()
 {
    let start = 1_000_000_i64;
    let calls = Cell::new(0);
    let clock = || {
        calls.set(calls.get() + 1);
        if calls.get() <= 3 {
            start
        } else {
            start + 1000
        }
    };
    let chunks = Arc::new(Mutex::new(0));
    let c = Arc::clone(&chunks);
    let out = ok(
        purge::run_purge_loop(3, 0, 100, DEFAULT_PURGE_MAX_CHUNKS, clock, move |_| {
            *c.lock().unwrap() += 1;
            async { ok_chunk(3) }
        })
        .await,
    );
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 6,
            stop_reason: PurgeStopReason::Deadline
        }
    );
    assert_eq!(*chunks.lock().unwrap(), 2);
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:219 "purgeBroadcastRecipients > honors a caller-provided maxChunks override as the safety cap"
#[tokio::test]
async fn purge_broadcast_recipients_honors_a_caller_provided_max_chunks_override_as_the_safety_cap()
{
    let db = with_broadcast("sent", 9).await;
    let mut i = input(3);
    i.max_chunks = Some(2);
    let out = ok(purge::purge_broadcast_recipients(&db, &i).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 6,
            stop_reason: PurgeStopReason::ChunkCap
        }
    );
    assert_eq!(recipients_left(&db).await, 3);
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:237 "purgeBroadcastRecipients > exports DEFAULT_PURGE_MAX_CHUNKS (5000) and applies it as the cap when maxChunks is omitted"
#[tokio::test]
async fn purge_broadcast_recipients_exports_default_purge_max_chunks_5000_and_applies_it_as_the_cap_when_max_chunks_is_omitted()
 {
    assert_eq!(DEFAULT_PURGE_MAX_CHUNKS, 5000);
    let db = with_broadcast("sent", 1).await;
    let out = ok(purge::purge_broadcast_recipients(&db, &input(3)).await);
    assert_eq!(
        out,
        PurgeOutcome {
            deleted: 1,
            stop_reason: PurgeStopReason::Drained
        }
    );
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:258 "hasBroadcastRecipients > issues an EXISTS-style probe bound to broadcastId, capped at 1 row"
#[tokio::test]
async fn has_broadcast_recipients_issues_an_exists_style_probe_bound_to_broadcast_id_capped_at_1_row()
 {
    let db = with_broadcast("sent", 2).await;
    assert!(ok(repo::read(&db, |c| purge::has_broadcast_recipients(
        c, 1
    ))
    .await));
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:271 "hasBroadcastRecipients > returns false when no recipient rows remain"
#[tokio::test]
async fn has_broadcast_recipients_returns_false_when_no_recipient_rows_remain() {
    let db = with_broadcast("sent", 0).await;
    assert!(!ok(repo::read(&db, |c| purge::has_broadcast_recipients(
        c, 1
    ))
    .await));
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:279 "hardDeleteBroadcast > pins the delete to deletedAt IS NOT NULL AND status <> 'sending', bound broadcastId"
#[tokio::test]
async fn hard_delete_broadcast_pins_the_delete_to_deleted_at_is_not_null_and_status_sending_bound_broadcast_id()
 {
    let db = with_broadcast("sent", 0).await;
    repo_common::handover::exec(
        &db,
        r#"UPDATE "Broadcast" SET "deletedAt" = 5 WHERE "id" = 1"#,
        vec![],
    )
    .await;
    assert!(ok(repo::write(&db, |tx| purge::hard_delete_broadcast(
        tx, 1
    ))
    .await));
    assert_eq!(
        repo_common::scalar_i64(&db, r#"SELECT count(*) FROM "Broadcast""#, vec![]).await,
        Some(0)
    );
}

/// Port of: packages/database/__tests__/broadcast-purge.test.ts:292 "hardDeleteBroadcast > returns false when nothing matched (already undeleted, or now sending)"
#[tokio::test]
async fn hard_delete_broadcast_returns_false_when_nothing_matched_already_undeleted_or_now_sending()
{
    let db = with_broadcast("sending", 0).await;
    assert!(!ok(repo::write(&db, |tx| purge::hard_delete_broadcast(
        tx, 1
    ))
    .await));
    repo_common::handover::exec(
        &db,
        r#"UPDATE "Broadcast" SET "deletedAt" = 5 WHERE "id" = 1"#,
        vec![],
    )
    .await;
    assert!(!ok(repo::write(&db, |tx| purge::hard_delete_broadcast(
        tx, 1
    ))
    .await));
    repo_common::handover::exec(
        &db,
        r#"UPDATE "Broadcast" SET "status" = 'sent', "deletedAt" = NULL WHERE "id" = 1"#,
        vec![],
    )
    .await;
    assert!(!ok(repo::write(&db, |tx| purge::hard_delete_broadcast(
        tx, 1
    ))
    .await));
}
