//! Port of `packages/database/__tests__/coexist-sync-run-repository.test.ts` (P3.2b).
//! The TS mocks the Drizzle chain and checks the `where` / `set` objects; here the
//! repository runs on a migrated temp file (adapted): the filters show in which rows change.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::coexist_sync_run::{self as csr, CreateRunInput, Increments, WriteGuard};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::types::Value;
use repo_common::err_message;
use repo10::{
    INSTAGRAM, MESSENGER, WHATSAPP, WS, coexist_db, exec, put_runs, read, run_row, run_rows, write,
};

const MIN: i64 = 60_000;

fn create_input() -> CreateRunInput<'static> {
    CreateRunInput {
        workspace_id: WS,
        integration_id: INSTAGRAM,
        channel: "instagram",
        trigger_source: "popup-enable",
    }
}

#[tokio::test]
async fn coexist_sync_run_repository_claim_run_with_new_token_only_claims_active_init_running_runs()
{
    let db = coexist_db().await;
    let now = now_ms();
    let stale = format!("\"lastHeartbeatAt\" = {}", now - 20 * MIN);
    let fresh = format!("\"lastHeartbeatAt\" = {}", now - MIN);
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "init"), now, ""),
            (
                2,
                MESSENGER,
                "messenger",
                ("coexist", "running"),
                now,
                &stale,
            ),
            (3, MESSENGER, "messenger", ("coexist", "succeeded"), now, ""),
            (4, MESSENGER, "messenger", ("coexist", "failed"), now, ""),
            (5, WHATSAPP, "whatsapp", ("coexist", "waiting"), now, ""),
            (
                6,
                INSTAGRAM,
                "instagram",
                ("coexist", "running"),
                now,
                &fresh,
            ),
        ],
    )
    .await;
    for id in [1, 2] {
        let claimed = write(&db, move |c| csr::claim_run_with_new_token(c, id, None)).await;
        let claimed = claimed.expect("an init / stale running run is claimed");
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.status, "running");
        assert!(claimed.claim_token.is_some());
    }
    // Terminal and waiting runs are outside ("init", "running"); a live running run is held.
    for (id, status) in [
        (3, "succeeded"),
        (4, "failed"),
        (5, "waiting"),
        (6, "running"),
    ] {
        let claimed = write(&db, move |c| csr::claim_run_with_new_token(c, id, None)).await;
        assert!(claimed.is_none(), "run {id} must not be claimed");
        let row = run_row(&db, id).await.expect("row");
        assert_eq!(row.status, status);
        assert_eq!(row.claim_token, None);
    }
}

#[tokio::test]
async fn coexist_sync_run_repository_mark_max_attempts_failed_spares_a_run_still_heart_beating() {
    let db = coexist_db().await;
    let now = now_ms();
    let fresh = format!("\"attempts\" = 5, \"lastHeartbeatAt\" = {}", now - MIN);
    let stale = format!(
        "\"attempts\" = 5, \"lastHeartbeatAt\" = {}, \"resumeCursor\" = 'c'",
        now - 20 * MIN
    );
    put_runs(
        &db,
        &[
            (
                1,
                MESSENGER,
                "messenger",
                ("coexist", "running"),
                now,
                &fresh,
            ),
            (2, WHATSAPP, "whatsapp", ("coexist", "running"), now, &stale),
            (
                3,
                MESSENGER,
                "messenger",
                ("coexist", "init"),
                now,
                "\"attempts\" = 5",
            ),
            (
                4,
                INSTAGRAM,
                "instagram",
                ("coexist", "init"),
                now,
                "\"attempts\" = 4",
            ),
            (5, 71, "messenger", ("contact_scan", "running"), now, &stale),
            (
                6,
                72,
                "whatsapp",
                ("coexist", "waiting"),
                now,
                "\"attempts\" = 5",
            ),
        ],
    )
    .await;
    write(&db, |c| csr::mark_max_attempts_failed(c, "coexist", 5)).await;
    let rows = run_rows(&db).await;
    let status: Vec<(i64, &str)> = rows.iter().map(|r| (r.id, r.status.as_str())).collect();
    assert_eq!(
        status,
        vec![
            (1, "running"),
            (2, "failed"),
            (3, "failed"),
            (4, "init"),
            (5, "running"),
            (6, "waiting"),
        ]
    );
    for row in rows.iter().filter(|r| r.status == "failed") {
        assert_eq!(
            row.current_error.as_deref(),
            Some("Max scheduler retries exceeded")
        );
        assert!(row.finished_at.is_some());
        assert_eq!(row.resume_cursor, None);
        assert!(row.updated_at >= now);
    }
}

#[tokio::test]
async fn coexist_sync_run_repository_create_run_returns_the_inserted_run_on_the_happy_path() {
    let db = coexist_db().await;
    let run = write(&db, |c| csr::create_run(c, &create_input())).await;
    assert_eq!(run.workspace_id, WS);
    assert_eq!(run.integration_id, INSTAGRAM);
    assert_eq!(run.channel, "instagram");
    assert_eq!(run.r#type, "coexist");
    assert_eq!(run.status, "init");
    assert_eq!(run.trigger_source, "popup-enable");
    let rows = run_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, run.id);
}

#[tokio::test]
async fn coexist_sync_run_repository_create_run_returns_the_existing_init_run_on_duplicate_enable_without_throwing()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(7, INSTAGRAM, "instagram", ("coexist", "init"), now_ms(), "")],
    )
    .await;
    let run = write(&db, |c| csr::create_run(c, &create_input())).await;
    assert_eq!(run.id, 7);
    assert_eq!(run_rows(&db).await.len(), 1);
}

#[tokio::test]
async fn coexist_sync_run_repository_create_run_throws_when_a_conflict_yields_no_re_selectable_init_run()
 {
    let db = coexist_db().await;
    // A lost race: the insert writes nothing and no init run is there to re-select.
    exec(
        &db,
        r#"CREATE TRIGGER "test_swallow_run" BEFORE INSERT ON "CoexistSyncRun"
           BEGIN SELECT RAISE(IGNORE); END"#,
    )
    .await;
    let message = err_message(repo::write(&db, |tx| csr::create_run(tx, &create_input())).await);
    assert!(message.contains("no active init run"), "{message}");
}

#[tokio::test]
async fn coexist_sync_run_repository_find_integration_for_coexist_admits_a_facebook_linked_instagram_row_without_a_type_filter()
 {
    let db = coexist_db().await;
    exec(
        &db,
        r#"UPDATE "IntegrationInstagram" SET "type" = 'facebook' WHERE "id" = 62"#,
    )
    .await;
    let row = read(&db, |c| {
        csr::find_integration_for_coexist(c, WS, INSTAGRAM, "instagram")
    })
    .await;
    let row = row.expect("the facebook-linked row is admitted");
    assert_eq!(row.channel(), "instagram");
    assert_eq!(row.id(), INSTAGRAM);
    match row {
        csr::CoexistIntegrationRow::Instagram(ig) => {
            assert_eq!(ig.r#type, "facebook");
            assert_eq!(ig.workspace_id, WS);
        }
        other => panic!("expected an Instagram row, got {other:?}"),
    }
    // The lookup is workspace-scoped.
    let fenced = read(&db, |c| {
        csr::find_integration_for_coexist(c, 2002, INSTAGRAM, "instagram")
    })
    .await;
    assert!(fenced.is_none());
}

#[tokio::test]
async fn coexist_sync_run_repository_reclaim_run_for_retry_does_not_filter_status_in_init_running()
{
    let db = coexist_db().await;
    let now = now_ms();
    let fresh = format!("\"lastHeartbeatAt\" = {}", now - MIN);
    let stale = format!(
        "\"lastHeartbeatAt\" = {}, \"startedAt\" = 1000",
        now - 20 * MIN
    );
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "succeeded"), now, ""),
            (2, MESSENGER, "messenger", ("coexist", "failed"), now, ""),
            (3, WHATSAPP, "whatsapp", ("coexist", "waiting"), now, ""),
            (
                4,
                INSTAGRAM,
                "instagram",
                ("coexist", "running"),
                now,
                &stale,
            ),
            (5, 72, "whatsapp", ("coexist", "running"), now, &fresh),
        ],
    )
    .await;
    for id in [1, 2, 3, 4] {
        let run = write(&db, move |c| csr::reclaim_run_for_retry(c, id, true)).await;
        let run = run.expect("any non-running or stale run is reclaimed");
        assert_eq!(run.status, "running");
        assert!(run.last_heartbeat_at.unwrap_or_default() >= now);
        assert!(run.started_at.is_some());
    }
    // `startedAt` is kept when set (COALESCE).
    assert_eq!(run_row(&db, 4).await.expect("row").started_at, Some(1000));
    // A running run with a live heartbeat is not reclaimed.
    let held = write(&db, |c| csr::reclaim_run_for_retry(c, 5, true)).await;
    assert!(held.is_none());
}

#[tokio::test]
async fn coexist_sync_run_repository_reclaim_run_for_retry_touches_updated_at_only_when_asked_messenger_sync_yes_whatsapp_flush_no()
 {
    let db = coexist_db().await;
    let old = now_ms() - 60 * MIN;
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "init"), old, ""),
            (2, WHATSAPP, "whatsapp", ("coexist", "waiting"), old, ""),
        ],
    )
    .await;
    let touched = write(&db, |c| csr::reclaim_run_for_retry(c, 1, true))
        .await
        .expect("run 1");
    assert!(touched.updated_at > old);
    let untouched = write(&db, |c| csr::reclaim_run_for_retry(c, 2, false))
        .await
        .expect("run 2");
    assert_eq!(untouched.status, "running");
    assert_eq!(untouched.updated_at, old);
}

#[tokio::test]
async fn coexist_sync_run_repository_increment_progress_uses_an_atomic_col_n_expression_never_a_read_modify_write()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now_ms(),
            "\"importedMessageCount\" = 10, \"skippedCount\" = 1",
        )],
    )
    .await;
    let increments = Increments {
        imported_message_count: Some(5),
        skipped_count: Some(2),
        ..Increments::default()
    };
    // Two writers: each adds to the stored value, so no increment is lost.
    for _ in 0..2 {
        write(&db, move |c| {
            csr::increment_progress(
                c,
                1,
                &increments,
                &vec![("currentStep", Value::Text("importing".into()))],
                None,
            )
        })
        .await;
    }
    let row = run_row(&db, 1).await.expect("row");
    assert_eq!(row.imported_message_count, 20);
    assert_eq!(row.skipped_count, 5);
    assert_eq!(row.current_step.as_deref(), Some("importing"));
}

#[tokio::test]
async fn coexist_sync_run_repository_increment_progress_skips_counters_whose_increment_is_undefined()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now_ms(),
            "\"failedCount\" = 7",
        )],
    )
    .await;
    let increments = Increments {
        current_scan: Some(1),
        failed_count: None,
        ..Increments::default()
    };
    write(&db, move |c| {
        csr::increment_progress(c, 1, &increments, &Vec::new(), None)
    })
    .await;
    let row = run_row(&db, 1).await.expect("row");
    assert_eq!(row.current_scan, 1);
    assert_eq!(row.failed_count, 7);
}

#[tokio::test]
async fn coexist_sync_run_repository_increment_progress_with_expect_fences_the_write_and_returns_the_affected_row_count()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now_ms(),
            "\"claimToken\" = 'token-1'",
        )],
    )
    .await;
    let increments = Increments {
        current_scan: Some(1),
        ..Increments::default()
    };
    let affected = write(&db, move |c| {
        let guard = WriteGuard {
            status: "running",
            claim_token: Some("token-1"),
        };
        csr::increment_progress(c, 1, &increments, &Vec::new(), Some(&guard))
    })
    .await;
    assert_eq!(affected, 1);
    assert_eq!(run_row(&db, 1).await.expect("row").current_scan, 1);
}

#[tokio::test]
async fn coexist_sync_run_repository_increment_progress_with_expect_returns_0_when_the_write_lands_on_no_rows_claim_taken_over()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now_ms(),
            "\"claimToken\" = 'token-2'",
        )],
    )
    .await;
    let increments = Increments {
        current_scan: Some(1),
        ..Increments::default()
    };
    let affected = write(&db, move |c| {
        let guard = WriteGuard {
            status: "running",
            claim_token: Some("stale-token"),
        };
        csr::increment_progress(c, 1, &increments, &Vec::new(), Some(&guard))
    })
    .await;
    assert_eq!(affected, 0);
    assert_eq!(run_row(&db, 1).await.expect("row").current_scan, 0);
}
