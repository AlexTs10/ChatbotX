//! Port of `packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts`
//! (P3.2). Runs against a migrated SQLite temp file (a fresh file per test instead of a
//! rolled-back transaction on an existing Page). `NOW() - INTERVAL 'n'` in the setup SQL
//! becomes `now - n ms`.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::ai_handover_bulk_run::{self as runs, FinishInput, ProgressInput};
use chatbotx_db::repo::ai_handover_settings::{self as settings, SetApplyToAllInput};
use chatbotx_db::repo::{self, now_ms};
use repo_common::handover::{INBOX, MINUTE, WS, claim, create, exec, find, page};
use repo_common::{int, ok};

fn set_input(customers: bool, message: Option<&str>) -> SetApplyToAllInput {
    SetApplyToAllInput {
        workspace_id: WS,
        inbox_id: INBOX,
        apply_to_all_customers: customers,
        apply_to_all_message: message.map(str::to_owned),
        requested_by_user_id: None,
    }
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:103 "aiHandoverBulkRunRepository on Postgres > a Page has one live run, and one run per revision"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_page_has_one_live_run_and_one_run_per_revision()
 {
    let db = page().await;
    let first = create(&db, 1, "disable").await.expect("first");
    assert_eq!(first.status, "pending");
    assert!(create(&db, 2, "enable").await.is_none());
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET status = 'completed' WHERE id = ?1"#,
        vec![int(first.id)],
    )
    .await;
    assert!(create(&db, 1, "enable").await.is_none());
    assert!(create(&db, 2, "enable").await.is_some());
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:120 "aiHandoverBulkRunRepository on Postgres > the settings row is created on demand, locked, and its revision counts real changes"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_the_settings_row_is_created_on_demand_locked_and_its_revision_counts_real_changes()
 {
    let db = page().await;
    let result = ok(repo::write(&db, |tx| {
        let none = settings::lock_existing(tx, WS, INBOX)?;
        let locked = settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")?;
        let first = settings::set_apply_to_all(tx, &set_input(true, None))?;
        let second = settings::set_apply_to_all(tx, &set_input(false, Some("bye")))?;
        let again = settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")?;
        Ok((none, locked, first, second, again))
    })
    .await);
    let (none, locked, first, second, again) = result;
    assert!(none.is_none());
    assert!(!locked.inbox_status.is_empty());
    assert!(!locked.settings.apply_to_all_customers);
    assert_eq!(locked.settings.apply_to_all_revision, 0);
    assert!(!locked.settings.enabled);
    assert_eq!(
        [first.apply_to_all_revision, second.apply_to_all_revision],
        [1, 2]
    );
    assert!(!second.apply_to_all_customers);
    assert_eq!(second.apply_to_all_message.as_deref(), Some("bye"));
    assert_eq!(again.settings.id, locked.settings.id);
}

async fn awaiting(db: &chatbotx_db::Db, after: Option<i64>) -> bool {
    ok(repo::read(db, move |c| runs::list_inboxes_awaiting_run(c, 1000, after)).await)
        .iter()
        .any(|(_, inbox)| *inbox == INBOX)
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:173 "aiHandoverBulkRunRepository on Postgres > the sweeper lists only Pages a run is DUE for"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_the_sweeper_lists_only_pages_a_run_is_due_for()
{
    let db = page().await;
    ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")?;
        settings::set_apply_to_all(tx, &set_input(false, Some("bye")))?;
        Ok(())
    })
    .await);
    assert!(awaiting(&db, None).await);
    ok(repo::write(&db, |tx| {
        settings::set_apply_to_all(tx, &set_input(true, None))
    })
    .await);
    assert!(!awaiting(&db, None).await);
    exec(
        &db,
        r#"UPDATE "AIHandoverSettings" SET enabled = 1 WHERE "inboxId" = ?1"#,
        vec![int(INBOX)],
    )
    .await;
    assert!(awaiting(&db, None).await);
    assert!(!awaiting(&db, Some(INBOX)).await);
    exec(
        &db,
        r#"UPDATE "Inbox" SET status = 'disconnected' WHERE id = ?1"#,
        vec![int(INBOX)],
    )
    .await;
    assert!(!awaiting(&db, None).await);
    exec(
        &db,
        r#"UPDATE "Inbox" SET status = 'connected' WHERE id = ?1"#,
        vec![int(INBOX)],
    )
    .await;
    let run = create(&db, 2, "enable").await.expect("run");
    assert!(!awaiting(&db, None).await);
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET status = 'failed' WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert!(!awaiting(&db, None).await);
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:241 "aiHandoverBulkRunRepository on Postgres > a disconnect cancelling under the settings lock waits for a run being created, and then sees it"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_disconnect_cancelling_under_the_settings_lock_waits_for_a_run_being_created_and_then_sees_it()
 {
    let db = page().await;
    ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
    })
    .await);
    let run = create(&db, 1, "disable").await.expect("run");
    let (locked, cancelled) = ok(repo::write(&db, |tx| {
        let locked = settings::lock_existing(tx, WS, INBOX)?;
        let cancelled = runs::cancel_live(tx, WS, INBOX, None, now_ms())?;
        Ok((locked, cancelled))
    })
    .await);
    assert!(locked.is_some());
    assert_eq!(cancelled.map(|r| r.id), Some(run.id));
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:256 "aiHandoverBulkRunRepository on Postgres > claim, progress, yield and cancel move the run through the enum states"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_claim_progress_yield_and_cancel_move_the_run_through_the_enum_states()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    let claimed = claim(&db, run.id, WS).await.expect("claimed");
    assert_eq!(claimed.status, "running");
    let token = claimed.claim_token.clone().unwrap_or_default();
    assert!(claim(&db, run.id, 0).await.is_none());
    assert!(claim(&db, run.id, WS).await.is_none());
    let t = token.clone();
    let progress = ProgressInput {
        add_processed: Some(3),
        cursor_contact_inbox_id: Some(Some("ci-1".into())),
        ..Default::default()
    };
    let status = ok(repo::write(&db, move |tx| {
        runs::record_progress(tx, run.id, &t, &progress, now_ms())
    })
    .await);
    assert_eq!(status.as_deref(), Some("running"));
    let stale = ok(repo::write(&db, move |tx| {
        runs::record_progress(tx, run.id, "stale", &ProgressInput::default(), now_ms())
    })
    .await);
    assert_eq!(stale, None);
    let t = token.clone();
    let seq = ok(repo::write(&db, move |tx| {
        runs::yield_for_continuation(tx, run.id, &t, None, now_ms())
    })
    .await);
    assert_eq!(seq, Some(1));
    assert_eq!(
        claim(&db, run.id, WS).await.map(|r| r.status).as_deref(),
        Some("running")
    );
}

async fn park(db: &chatbotx_db::Db, run_id: i64) {
    let claimed = claim(db, run_id, WS).await.expect("claimed");
    let token = claimed.claim_token.unwrap_or_default();
    let until = now_ms() + 60 * MINUTE;
    ok(repo::write(db, move |tx| {
        runs::yield_for_continuation(tx, run_id, &token, Some(until), now_ms())
    })
    .await);
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:302 "aiHandoverBulkRunRepository on Postgres > a run parked for a quota pause cannot be claimed or re-dispatched until the pause ends"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_run_parked_for_a_quota_pause_cannot_be_claimed_or_re_dispatched_until_the_pause_ends()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    park(&db, run.id).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "lastHeartbeatAt" = ?1, attempts = 0 WHERE id = ?2"#,
        vec![int(now_ms() - 5 * MINUTE), int(run.id)],
    )
    .await;
    assert!(claim(&db, run.id, WS).await.is_none());
    assert_eq!(
        ok(repo::write(&db, move |tx| runs::reopen_released(tx, run.id, now_ms())).await),
        0
    );
    let picked = ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 50, now_ms())).await);
    assert!(!picked.iter().any(|p| p.id == run.id));
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "pausedUntil" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 1000), int(run.id)],
    )
    .await;
    assert_eq!(
        claim(&db, run.id, WS).await.map(|r| r.status).as_deref(),
        Some("running")
    );
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:340 "aiHandoverBulkRunRepository on Postgres > quota pauses in a row never fail a run: each one resets the retry budget even though nothing settles"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_quota_pauses_in_a_row_never_fail_a_run_each_one_resets_the_retry_budget_even_though_nothing_settles()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    for _ in 0..6 {
        park(&db, run.id).await;
        exec(
            &db,
            r#"UPDATE "AIHandoverBulkRun" SET "pausedUntil" = ?1, "lastHeartbeatAt" = ?2 WHERE id = ?3"#,
            vec![int(now_ms() - 1000), int(now_ms() - 5 * MINUTE), int(run.id)],
        )
        .await;
        ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 50, now_ms())).await);
        ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 5, now_ms())).await);
    }
    assert_ne!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("failed")
    );
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:371 "aiHandoverBulkRunRepository on Postgres > a pending or released run is cancelled outright, a claimed one winds down, and a cancel ends a pause"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_pending_or_released_run_is_cancelled_outright_a_claimed_one_winds_down_and_a_cancel_ends_a_pause()
 {
    let db = page().await;
    let pending = create(&db, 1, "disable").await.expect("pending");
    let cancelled =
        ok(repo::write(&db, |tx| runs::cancel_live(tx, WS, INBOX, None, now_ms())).await)
            .expect("cancelled");
    assert_eq!(cancelled.id, pending.id);
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.finished_at.is_some());
    let claimed_run = create(&db, 2, "disable").await.expect("second");
    claim(&db, claimed_run.id, WS).await;
    let winding = ok(repo::write(&db, |tx| runs::cancel_live(tx, WS, INBOX, None, now_ms())).await);
    assert_eq!(winding.map(|r| r.status).as_deref(), Some("cancelling"));
    assert!(create(&db, 3, "enable").await.is_none());
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "claimToken" = NULL, "pausedUntil" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() + 60 * MINUTE), int(claimed_run.id)],
    )
    .await;
    assert_eq!(
        claim(&db, claimed_run.id, WS)
            .await
            .map(|r| r.status)
            .as_deref(),
        Some("cancelling")
    );
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:398 "aiHandoverBulkRunRepository on Postgres > cancelLive limited to an action leaves a live run of the other action alone"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_cancel_live_limited_to_an_action_leaves_a_live_run_of_the_other_action_alone()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    assert!(
        ok(repo::write(&db, |tx| runs::cancel_live(
            tx,
            WS,
            INBOX,
            Some("enable"),
            now_ms()
        ))
        .await)
        .is_none()
    );
    let hit = ok(repo::write(&db, |tx| {
        runs::cancel_live(tx, WS, INBOX, Some("disable"), now_ms())
    })
    .await);
    assert_eq!(hit.map(|r| r.id), Some(run.id));
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:411 "aiHandoverBulkRunRepository on Postgres > refundDispatch undoes pickDue for the picked values only, so the next sweep sees the same queued job"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_refund_dispatch_undoes_pick_due_for_the_picked_values_only_so_the_next_sweep_sees_the_same_queued_job()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 2, "chunkSeq" = 5 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    let read = || async {
        let r = find(&db, 1).await.expect("run");
        (r.attempts, r.chunk_seq)
    };
    assert!(!ok(repo::write(&db, move |tx| runs::refund_dispatch(
        tx, run.id, 3, 5
    ))
    .await));
    assert_eq!(read().await, (2, 5));
    assert!(ok(repo::write(&db, move |tx| runs::refund_dispatch(
        tx, run.id, 2, 5
    ))
    .await));
    assert_eq!(read().await, (1, 4));
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:444 "aiHandoverBulkRunRepository on Postgres > refundDispatch never takes a counter below zero"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_refund_dispatch_never_takes_a_counter_below_zero()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    assert!(!ok(repo::write(&db, move |tx| runs::refund_dispatch(
        tx, run.id, 0, 0
    ))
    .await));
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:457 "aiHandoverBulkRunRepository on Postgres > a released parked run is cancelled at once: there is no worker to wind it down"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_released_parked_run_is_cancelled_at_once_there_is_no_worker_to_wind_it_down()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    park(&db, run.id).await;
    let cancelled =
        ok(repo::write(&db, |tx| runs::cancel_live(tx, WS, INBOX, None, now_ms())).await)
            .expect("cancelled");
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(cancelled.paused_until, None);
    assert!(create(&db, 2, "enable").await.is_some());
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:480 "aiHandoverBulkRunRepository on Postgres > a terminal run is never reopened by a stale job"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_a_terminal_run_is_never_reopened_by_a_stale_job()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    let token = claim(&db, run.id, WS)
        .await
        .and_then(|r| r.claim_token)
        .unwrap_or_default();
    let outcome = FinishInput {
        status: "completed".into(),
        current_error: None,
        total_count: None,
    };
    ok(repo::write(&db, move |tx| {
        runs::finish(tx, run.id, &token, &outcome, now_ms())
    })
    .await);
    assert!(claim(&db, run.id, WS).await.is_none());
}

/// Port of: packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts:498 "aiHandoverBulkRunRepository on Postgres > the sweeper ends an exhausted stale run (a cancelling one as cancelled) and redispatches a stuck pending one"
#[tokio::test]
async fn ai_handover_bulk_run_repository_on_postgres_the_sweeper_ends_an_exhausted_stale_run_a_cancelling_one_as_cancelled_and_redispatches_a_stuck_pending_one()
 {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    claim(&db, run.id, WS).await;
    exec(&db, r#"UPDATE "AIHandoverBulkRun" SET status = 'cancelling', attempts = 5, "updatedAt" = ?2 WHERE id = ?1"#, vec![int(run.id), int(now_ms())]).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "lastHeartbeatAt" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 20 * MINUTE), int(run.id)],
    )
    .await;
    ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 5, now_ms())).await);
    assert_eq!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("cancelling")
    );
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "updatedAt" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 16 * MINUTE), int(run.id)],
    )
    .await;
    ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 5, now_ms())).await);
    assert_eq!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("cancelled")
    );
    let stuck = create(&db, 2, "enable").await.expect("stuck");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "updatedAt" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 5 * MINUTE), int(stuck.id)],
    )
    .await;
    let picked = ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 10, now_ms())).await);
    let p = picked
        .into_iter()
        .find(|p| p.id == stuck.id)
        .expect("picked");
    assert_eq!((p.attempts, p.chunk_seq), (1, 1));
}
