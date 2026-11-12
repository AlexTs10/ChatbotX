//! Port of `packages/database/__tests__/ai-handover-bulk-run-repository.test.ts` (P3.2).
//! The original mocks the Drizzle chains and inspects the `set` / `where` arguments. Each
//! case is ported as the effect on a real run row of a migrated SQLite file (adapted):
//! "conditional on X" = the write is refused when X does not hold; "a column is not in
//! the patch" = the stored value survives the write.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::ai_handover_bulk_run::{self as runs, FinishInput, ProgressInput};
use chatbotx_db::repo::models::AiHandoverBulkRun;
use chatbotx_db::repo::{self, now_ms};
use repo_common::handover::{INBOX, MINUTE, WS, claim, create, create_input, exec, find, page};
use repo_common::{int, ok, text};

/// A claimed (running) run of revision 1 and its token.
async fn claimed(db: &Db) -> (AiHandoverBulkRun, String) {
    let run = create(db, 1, "disable").await.expect("run");
    let c = claim(db, run.id, WS).await.expect("claimed");
    let token = c.claim_token.clone().unwrap_or_default();
    (c, token)
}

async fn progress(db: &Db, id: i64, token: &str, p: ProgressInput) -> Option<String> {
    let token = token.to_owned();
    ok(repo::write(db, move |tx| {
        runs::record_progress(tx, id, &token, &p, now_ms())
    })
    .await)
}

async fn yield_(db: &Db, id: i64, token: &str, until: Option<i64>) -> Option<i64> {
    let token = token.to_owned();
    ok(repo::write(db, move |tx| {
        runs::yield_for_continuation(tx, id, &token, until, now_ms())
    })
    .await)
}

async fn finish(db: &Db, id: i64, token: &str, outcome: FinishInput) -> usize {
    let token = token.to_owned();
    ok(repo::write(db, move |tx| {
        runs::finish(tx, id, &token, &outcome, now_ms())
    })
    .await)
}

fn completed() -> FinishInput {
    FinishInput {
        status: "completed".into(),
        current_error: None,
        total_count: None,
    }
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:45 "aiHandoverBulkRunRepository.createForRevision > writes every column explicitly as a pending run"
#[tokio::test]
async fn ai_handover_bulk_run_repository_create_for_revision_writes_every_column_explicitly_as_a_pending_run()
 {
    let db = page().await;
    let mut input = create_input(3, "disable");
    input.message = Some("We are back".into());
    input.requested_by_user_id = Some(1001);
    input.requested_at = repo_common::ms("2026-10-02T00:00:00Z");
    let row =
        ok(repo::write(&db, move |tx| runs::create_for_revision(tx, &input)).await).expect("row");
    assert_eq!(
        (row.workspace_id, row.inbox_id, row.revision),
        (WS, INBOX, 3)
    );
    assert_eq!(
        (
            row.channel.as_str(),
            row.action.as_str(),
            row.status.as_str()
        ),
        ("messenger", "disable", "pending")
    );
    assert_eq!(row.message.as_deref(), Some("We are back"));
    assert_eq!(row.requested_by_user_id, Some(1001));
    assert_eq!(row.requested_at, repo_common::ms("2026-10-02T00:00:00Z"));
    assert_eq!(
        (
            row.started_at,
            row.finished_at,
            row.last_heartbeat_at,
            row.paused_until
        ),
        (None, None, None, None)
    );
    assert_eq!(row.claim_token, None);
    assert_eq!((row.attempts, row.chunk_seq, row.total_count), (0, 0, None));
    assert_eq!(
        (row.processed_count, row.skipped_count, row.failed_count),
        (0, 0, 0)
    );
    assert_eq!(
        (
            row.cursor_contact_inbox_id,
            row.in_flight_from_id,
            row.in_flight_to_id,
            row.current_error
        ),
        (None, None, None, None)
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:78 "aiHandoverBulkRunRepository.createForRevision > returns null when the Page has a live run or already ran this revision"
#[tokio::test]
async fn ai_handover_bulk_run_repository_create_for_revision_returns_null_when_the_page_has_a_live_run_or_already_ran_this_revision()
 {
    let db = page().await;
    let first = create(&db, 1, "disable").await.expect("first");
    assert!(create(&db, 2, "disable").await.is_none());
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET status = 'completed' WHERE id = ?1"#,
        vec![int(first.id)],
    )
    .await;
    assert!(create(&db, 1, "disable").await.is_none());
}

async fn guard_case(name: &str) {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    // Wrong token, wrong id, terminal status: refused.
    let refused = |r: Option<i64>| r.is_none();
    match name {
        "recordProgress" => {
            let p = || ProgressInput {
                add_processed: Some(3),
                ..Default::default()
            };
            assert!(progress(&db, run.id, "token-1", p()).await.is_none());
            assert!(progress(&db, run.id + 1, &token, p()).await.is_none());
            assert_eq!(
                progress(&db, run.id, &token, p()).await.as_deref(),
                Some("running")
            );
        }
        "yieldForContinuation" => {
            assert!(refused(yield_(&db, run.id, "token-1", None).await));
            assert!(refused(yield_(&db, run.id + 1, &token, None).await));
            assert_eq!(yield_(&db, run.id, &token, None).await, Some(1));
        }
        _ => {
            assert_eq!(finish(&db, run.id, "token-1", completed()).await, 0);
            assert_eq!(finish(&db, run.id + 1, &token, completed()).await, 0);
            assert_eq!(finish(&db, run.id, &token, completed()).await, 1);
            // Now terminal: the same token no longer matches a live status.
            assert_eq!(finish(&db, run.id, &token, completed()).await, 0);
        }
    }
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:117 "guarded writes > %s is conditional on the run id, a live status and the claim token" (recordProgress)
#[tokio::test]
async fn guarded_writes_record_progress_is_conditional_on_the_run_id_a_live_status_and_the_claim_token()
 {
    guard_case("recordProgress").await;
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:117 "guarded writes > %s is conditional on the run id, a live status and the claim token" (yieldForContinuation)
#[tokio::test]
async fn guarded_writes_yield_for_continuation_is_conditional_on_the_run_id_a_live_status_and_the_claim_token()
 {
    guard_case("yieldForContinuation").await;
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:117 "guarded writes > %s is conditional on the run id, a live status and the claim token" (finish)
#[tokio::test]
async fn guarded_writes_finish_is_conditional_on_the_run_id_a_live_status_and_the_claim_token() {
    guard_case("finish").await;
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:129 "guarded writes > recordProgress reports null when the claim was lost and the status when it landed"
#[tokio::test]
async fn guarded_writes_record_progress_reports_null_when_the_claim_was_lost_and_the_status_when_it_landed()
 {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    assert_eq!(
        progress(&db, run.id, "lost", ProgressInput::default()).await,
        None
    );
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET status = 'cancelling' WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert_eq!(
        progress(&db, run.id, &token, ProgressInput::default())
            .await
            .as_deref(),
        Some("cancelling")
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:147 "guarded writes > recordProgress only touches the cursors it was given"
#[tokio::test]
async fn guarded_writes_record_progress_only_touches_the_cursors_it_was_given() {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "inFlightFromId" = 'a', "inFlightToId" = 'b', "totalCount" = 9, "currentError" = 'kept' WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    let p = ProgressInput {
        cursor_contact_inbox_id: Some(Some("ci-5".into())),
        in_flight_from_id: Some(None),
        ..Default::default()
    };
    progress(&db, run.id, &token, p).await;
    let row = find(&db, 1).await.expect("run");
    assert_eq!(row.cursor_contact_inbox_id.as_deref(), Some("ci-5"));
    assert_eq!(row.in_flight_from_id, None);
    assert_eq!(row.in_flight_to_id.as_deref(), Some("b"));
    assert_eq!(row.total_count, Some(9));
    assert_eq!(row.current_error.as_deref(), Some("kept"));
}

async fn attempts_after(p: ProgressInput) -> i64 {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 3 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    progress(&db, run.id, &token, p).await;
    find(&db, 1).await.expect("run").attempts
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:176 "guarded writes > %s is not progress: it leaves the sweeper's retry budget alone, or a run whose every call fails would never run out of attempts" (a marker write)
#[tokio::test]
async fn guarded_writes_a_marker_write_is_not_progress_it_leaves_the_sweeper_s_retry_budget_alone_or_a_run_whose_every_call_fails_would_never_run_out_of_attempts()
 {
    let p = ProgressInput {
        in_flight_from_id: Some(Some("a".into())),
        in_flight_to_id: Some(Some("b".into())),
        ..Default::default()
    };
    assert_eq!(attempts_after(p).await, 3);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:176 "guarded writes > %s is not progress: it leaves the sweeper's retry budget alone, or a run whose every call fails would never run out of attempts" (a cursor write)
#[tokio::test]
async fn guarded_writes_a_cursor_write_is_not_progress_it_leaves_the_sweeper_s_retry_budget_alone_or_a_run_whose_every_call_fails_would_never_run_out_of_attempts()
 {
    let p = ProgressInput {
        cursor_contact_inbox_id: Some(Some("ci-1".into())),
        ..Default::default()
    };
    assert_eq!(attempts_after(p).await, 3);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:176 "guarded writes > %s is not progress: it leaves the sweeper's retry budget alone, or a run whose every call fails would never run out of attempts" (a total count)
#[tokio::test]
async fn guarded_writes_a_total_count_is_not_progress_it_leaves_the_sweeper_s_retry_budget_alone_or_a_run_whose_every_call_fails_would_never_run_out_of_attempts()
 {
    let p = ProgressInput {
        total_count: Some(40),
        ..Default::default()
    };
    assert_eq!(attempts_after(p).await, 3);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:176 "guarded writes > %s is not progress: it leaves the sweeper's retry budget alone, or a run whose every call fails would never run out of attempts" (a settled count of zero)
#[tokio::test]
async fn guarded_writes_a_settled_count_of_zero_is_not_progress_it_leaves_the_sweeper_s_retry_budget_alone_or_a_run_whose_every_call_fails_would_never_run_out_of_attempts()
 {
    let p = ProgressInput {
        add_processed: Some(0),
        add_skipped: Some(0),
        add_failed: Some(0),
        ..Default::default()
    };
    assert_eq!(attempts_after(p).await, 3);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:191 "guarded writes > a settled thread (%s) resets the retry budget" (processed)
#[tokio::test]
async fn guarded_writes_a_settled_thread_processed_resets_the_retry_budget() {
    assert_eq!(
        attempts_after(ProgressInput {
            add_processed: Some(1),
            ..Default::default()
        })
        .await,
        0
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:191 "guarded writes > a settled thread (%s) resets the retry budget" (skipped)
#[tokio::test]
async fn guarded_writes_a_settled_thread_skipped_resets_the_retry_budget() {
    assert_eq!(
        attempts_after(ProgressInput {
            add_skipped: Some(1),
            ..Default::default()
        })
        .await,
        0
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:191 "guarded writes > a settled thread (%s) resets the retry budget" (failed)
#[tokio::test]
async fn guarded_writes_a_settled_thread_failed_resets_the_retry_budget() {
    assert_eq!(
        attempts_after(ProgressInput {
            add_failed: Some(1),
            ..Default::default()
        })
        .await,
        0
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:202 "guarded writes > recordProgress resets the retry budget: attempts count dispatches WITHOUT progress"
#[tokio::test]
async fn guarded_writes_record_progress_resets_the_retry_budget_attempts_count_dispatches_without_progress()
 {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 3 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    progress(
        &db,
        run.id,
        &token,
        ProgressInput {
            add_processed: Some(1),
            current_error: Some("pageSkipped".into()),
            ..Default::default()
        },
    )
    .await;
    let row = find(&db, 1).await.expect("run");
    assert_eq!(
        (row.attempts, row.current_error.as_deref()),
        (0, Some("pageSkipped"))
    );
    assert_eq!(row.processed_count, 1);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:220 "guarded writes > a plain continuation (no pause) leaves the retry budget alone"
#[tokio::test]
async fn guarded_writes_a_plain_continuation_no_pause_leaves_the_retry_budget_alone() {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 3 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    yield_(&db, run.id, &token, None).await;
    assert_eq!(find(&db, 1).await.expect("run").attempts, 3);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:231 "guarded writes > finish without a reason keeps the one recorded while the run was going"
#[tokio::test]
async fn guarded_writes_finish_without_a_reason_keeps_the_one_recorded_while_the_run_was_going() {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "currentError" = 'pageSkipped' WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    finish(&db, run.id, &token, completed()).await;
    assert_eq!(
        find(&db, 1).await.expect("run").current_error.as_deref(),
        Some("pageSkipped")
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:242 "guarded writes > yieldForContinuation returns the next revision, or null when the guard failed"
#[tokio::test]
async fn guarded_writes_yield_for_continuation_returns_the_next_revision_or_null_when_the_guard_failed()
 {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "chunkSeq" = 3 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert_eq!(yield_(&db, run.id, &token, None).await, Some(4));
    assert_eq!(find(&db, 1).await.expect("run").claim_token, None);
    assert_eq!(yield_(&db, run.id, &token, None).await, None);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:261 "guarded writes > yieldForContinuation can park the run: pausedUntil is written, the heartbeat stays real"
#[tokio::test]
async fn guarded_writes_yield_for_continuation_can_park_the_run_paused_until_is_written_the_heartbeat_stays_real()
 {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    let until = repo_common::ms("2027-10-02T13:00:00Z");
    let before = now_ms();
    yield_(&db, run.id, &token, Some(until)).await;
    let row = find(&db, 1).await.expect("run");
    assert_eq!((row.claim_token, row.paused_until), (None, Some(until)));
    let hb = row.last_heartbeat_at.expect("heartbeat");
    assert!(hb != until && hb >= before && hb <= now_ms());
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:277 "guarded writes > finish clears the lease and the in-flight markers"
#[tokio::test]
async fn guarded_writes_finish_clears_the_lease_and_the_in_flight_markers() {
    let db = page().await;
    let (run, token) = claimed(&db).await;
    exec(&db, r#"UPDATE "AIHandoverBulkRun" SET "inFlightFromId" = 'a', "inFlightToId" = 'b' WHERE id = ?1"#, vec![int(run.id)]).await;
    let outcome = FinishInput {
        status: "failed".into(),
        current_error: Some(Some("token revoked".into())),
        total_count: Some(7),
    };
    assert_eq!(finish(&db, run.id, &token, outcome).await, 1);
    let row = find(&db, 1).await.expect("run");
    assert_eq!(row.status, "failed");
    assert_eq!(row.current_error.as_deref(), Some("token revoked"));
    assert_eq!(row.total_count, Some(7));
    assert_eq!(
        (row.claim_token, row.in_flight_from_id, row.in_flight_to_id),
        (None, None, None)
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:305 "claim > only a pending, released or stale-lease live run can be claimed"
#[tokio::test]
async fn claim_only_a_pending_released_or_stale_lease_live_run_can_be_claimed() {
    let db = page().await;
    let (run, _) = claimed(&db).await;
    // Held with a fresh heartbeat: refused; other workspace: refused.
    assert!(claim(&db, run.id, WS).await.is_none());
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "lastHeartbeatAt" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 16 * MINUTE), int(run.id)],
    )
    .await;
    assert!(claim(&db, run.id, 9999).await.is_none());
    // Stale lease: claimable.
    assert!(claim(&db, run.id, WS).await.is_some());
    // Released (no token): claimable.
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "claimToken" = NULL WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert!(claim(&db, run.id, WS).await.is_some());
    // Terminal statuses never.
    for status in ["completed", "failed", "cancelled"] {
        exec(
            &db,
            r#"UPDATE "AIHandoverBulkRun" SET status = ?1, "claimToken" = NULL WHERE id = ?2"#,
            vec![text(status), int(run.id)],
        )
        .await;
        assert!(claim(&db, run.id, WS).await.is_none(), "{status}");
    }
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:328 "claim > a paused run cannot be claimed until the pause is over, but a cancelling one can"
#[tokio::test]
async fn claim_a_paused_run_cannot_be_claimed_until_the_pause_is_over_but_a_cancelling_one_can() {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "pausedUntil" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() + 60 * MINUTE), int(run.id)],
    )
    .await;
    assert!(claim(&db, run.id, WS).await.is_none());
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET status = 'cancelling' WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert!(claim(&db, run.id, WS).await.is_some());
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:342 "claim > a cancelling run keeps its status and a pending one starts running"
#[tokio::test]
async fn claim_a_cancelling_run_keeps_its_status_and_a_pending_one_starts_running() {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    let c = claim(&db, run.id, WS).await.expect("claimed");
    assert_eq!(c.status, "running");
    assert!(c.claim_token.as_deref().is_some_and(|t| !t.is_empty()));
    exec(&db, r#"UPDATE "AIHandoverBulkRun" SET status = 'cancelling', "claimToken" = NULL WHERE id = ?1"#, vec![int(run.id)]).await;
    assert_eq!(
        claim(&db, run.id, WS).await.map(|r| r.status).as_deref(),
        Some("cancelling")
    );
}

async fn cancel(db: &Db) -> Option<AiHandoverBulkRun> {
    ok(repo::write(db, |tx| runs::cancel_live(tx, WS, INBOX, None, now_ms())).await)
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:358 "cancelLive > a run nobody holds (pending, or released and parked) is cancelled outright, a claimed one winds down"
#[tokio::test]
async fn cancel_live_a_run_nobody_holds_pending_or_released_and_parked_is_cancelled_outright_a_claimed_one_winds_down()
 {
    let db = page().await;
    create(&db, 1, "disable").await;
    assert_eq!(
        cancel(&db).await.map(|r| r.status).as_deref(),
        Some("cancelled")
    );
    let (run, _) = {
        let r = create(&db, 2, "disable").await.expect("run");
        let c = claim(&db, r.id, WS).await.expect("claimed");
        (c, ())
    };
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "pausedUntil" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() + MINUTE), int(run.id)],
    )
    .await;
    let winding = cancel(&db).await.expect("winding");
    assert_eq!(winding.status, "cancelling");
    assert_eq!(winding.paused_until, None);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:378 "cancelLive > cancelLive targets the Page's live run, scoped by workspace and inbox"
#[tokio::test]
async fn cancel_live_cancel_live_targets_the_page_s_live_run_scoped_by_workspace_and_inbox() {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    let other = ok(repo::write(&db, |tx| runs::cancel_live(tx, 9999, INBOX, None, now_ms())).await);
    assert!(other.is_none());
    assert_eq!(cancel(&db).await.map(|r| r.id), Some(run.id));
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:391 "cancelLive > returns null when nothing live is left to cancel"
#[tokio::test]
async fn cancel_live_returns_null_when_nothing_live_is_left_to_cancel() {
    let db = page().await;
    assert!(cancel(&db).await.is_none());
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:404 "sweeper queries > reopenReleased only matches a running run with no token"
#[tokio::test]
async fn sweeper_queries_reopen_released_only_matches_a_running_run_with_no_token() {
    let db = page().await;
    let (run, _) = claimed(&db).await;
    let id = run.id;
    assert_eq!(
        ok(repo::write(&db, move |tx| runs::reopen_released(tx, id, now_ms())).await),
        0
    );
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "claimToken" = NULL WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert_eq!(
        ok(repo::write(&db, move |tx| runs::reopen_released(tx, id, now_ms())).await),
        1
    );
    assert_eq!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("pending")
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:416 "sweeper queries > refundDispatch undoes pickDue's counters for exactly the picked values"
#[tokio::test]
async fn sweeper_queries_refund_dispatch_undoes_pick_due_s_counters_for_exactly_the_picked_values()
{
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 3, "chunkSeq" = 4 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert!(ok(repo::write(&db, move |tx| runs::refund_dispatch(
        tx, run.id, 3, 4
    ))
    .await));
    let row = find(&db, 1).await.expect("run");
    assert_eq!((row.attempts, row.chunk_seq), (2, 3));
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:434 "sweeper queries > refundDispatch does nothing when the run moved on since it was picked"
#[tokio::test]
async fn sweeper_queries_refund_dispatch_does_nothing_when_the_run_moved_on_since_it_was_picked() {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 4, "chunkSeq" = 5 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    assert!(!ok(repo::write(&db, move |tx| runs::refund_dispatch(
        tx, run.id, 3, 4
    ))
    .await));
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:445 "sweeper queries > neither reopenReleased, markMaxAttemptsFailed nor pickDue touches a run that is paused on purpose"
#[tokio::test]
async fn sweeper_queries_neither_reopen_released_mark_max_attempts_failed_nor_pick_due_touches_a_run_that_is_paused_on_purpose()
 {
    let db = page().await;
    let (run, _) = claimed(&db).await;
    let old = now_ms() - 30 * MINUTE;
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "claimToken" = NULL, "pausedUntil" = ?1, attempts = 5, "lastHeartbeatAt" = ?2, "updatedAt" = ?2 WHERE id = ?3"#,
        vec![int(now_ms() + 60 * MINUTE), int(old), int(run.id)],
    )
    .await;
    assert_eq!(
        ok(repo::write(&db, move |tx| runs::reopen_released(tx, run.id, now_ms())).await),
        0
    );
    ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 5, now_ms())).await);
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET attempts = 0 WHERE id = ?1"#,
        vec![int(run.id)],
    )
    .await;
    let picked = ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 10, now_ms())).await);
    assert!(picked.is_empty());
    assert_eq!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("running")
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:470 "sweeper queries > markMaxAttemptsFailed ends exhausted live runs with a stale lease"
#[tokio::test]
async fn sweeper_queries_mark_max_attempts_failed_ends_exhausted_live_runs_with_a_stale_lease() {
    let db = page().await;
    let (run, _) = claimed(&db).await;
    exec(&db, r#"UPDATE "AIHandoverBulkRun" SET attempts = 5, "lastHeartbeatAt" = ?1, "updatedAt" = ?1 WHERE id = ?2"#, vec![int(now_ms() - 20 * MINUTE), int(run.id)]).await;
    ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 6, now_ms())).await);
    assert_eq!(
        find(&db, 1).await.map(|r| r.status).as_deref(),
        Some("running")
    );
    ok(repo::write(&db, |tx| runs::mark_max_attempts_failed(tx, 5, now_ms())).await);
    let row = find(&db, 1).await.expect("run");
    assert_eq!(
        (row.status.as_str(), row.current_error.as_deref()),
        ("failed", Some("maxAttempts"))
    );
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:489 "sweeper queries > pickDue burns an attempt, skips locked rows and returns the revision"
#[tokio::test]
async fn sweeper_queries_pick_due_burns_an_attempt_skips_locked_rows_and_returns_the_revision() {
    let db = page().await;
    let run = create(&db, 1, "disable").await.expect("run");
    exec(
        &db,
        r#"UPDATE "AIHandoverBulkRun" SET "chunkSeq" = 1, "updatedAt" = ?1 WHERE id = ?2"#,
        vec![int(now_ms() - 3 * MINUTE), int(run.id)],
    )
    .await;
    let picked = ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 10, now_ms())).await);
    assert_eq!(picked.len(), 1);
    let p = &picked[0];
    assert_eq!(
        (
            p.id,
            p.workspace_id,
            p.attempts,
            p.chunk_seq,
            p.status.as_str()
        ),
        (run.id, WS, 1, 2, "pending")
    );
    // A pending run touched less than 2 minutes ago is not due.
    assert!(ok(repo::write(&db, |tx| runs::pick_due(tx, 5, 10, now_ms())).await).is_empty());
}

async fn due_page(db: &Db, inbox: i64, revision: i64) {
    repo_common::setup(db, move |tx| {
        if inbox != INBOX {
            repo_common::insert_inbox(tx, inbox, WS, "messenger", &format!("page-{inbox}"))?;
        }
        ins!(tx, "AIHandoverSettings", { "id" => inbox, "workspaceId" => WS, "inboxId" => inbox, "channel" => "messenger", "timeRanges" => "[]", "applyToAllRevision" => revision })?;
        Ok(())
    })
    .await;
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:519 "finders > listInboxesAwaitingRun resumes after the given Page id"
#[tokio::test]
async fn finders_list_inboxes_awaiting_run_resumes_after_the_given_page_id() {
    let db = page().await;
    due_page(&db, INBOX, 1).await;
    due_page(&db, 3500, 1).await;
    let after = ok(repo::read(&db, |c| runs::list_inboxes_awaiting_run(c, 20, Some(INBOX))).await);
    assert_eq!(after, vec![(WS, 3500)]);
}

/// Port of: packages/database/__tests__/ai-handover-bulk-run-repository.test.ts:541 "finders > listInboxesAwaitingRun only lists Pages a run is DUE for, oldest change first, so waiting Pages cannot crowd them out"
#[tokio::test]
async fn finders_list_inboxes_awaiting_run_only_lists_pages_a_run_is_due_for_oldest_change_first_so_waiting_pages_cannot_crowd_them_out()
 {
    let db = page().await;
    due_page(&db, INBOX, 1).await; // due
    due_page(&db, 3500, 0).await; // never changed
    due_page(&db, 3600, 2).await; // has a live run
    due_page(&db, 3700, 1).await; // ON without the automation on
    exec(
        &db,
        r#"UPDATE "AIHandoverSettings" SET "applyToAllCustomers" = 1 WHERE "inboxId" = 3700"#,
        vec![],
    )
    .await;
    let mut live = create_input(2, "enable");
    live.inbox_id = 3600;
    ok(repo::write(&db, move |tx| runs::create_for_revision(tx, &live)).await);
    let rows = ok(repo::read(&db, |c| runs::list_inboxes_awaiting_run(c, 20, None)).await);
    assert_eq!(rows, vec![(WS, INBOX)]);
    // The limit is applied.
    due_page(&db, 3800, 1).await;
    assert_eq!(
        ok(repo::read(&db, |c| runs::list_inboxes_awaiting_run(c, 1, None)).await).len(),
        1
    );
}
