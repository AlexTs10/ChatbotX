//! Port of `packages/database/__tests__/coexist-sync-run-type-scoping.test.ts` (P3.2b).
//! The TS checks that every query of the mocked Drizzle client carries the `type`
//! discriminator; here each query runs on a migrated temp file that holds both a `coexist`
//! and a `contact_scan` run (adapted), so the filter shows as the run it leaves alone.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::coexist_sync_run::{
    self as csr, CreateContactScanRunInput, ListContactScanRunsInput, StrandedIntegration,
    WriteGuard,
};
use chatbotx_db::repo::now_ms;
use repo_common::ms;
use repo10::{
    MESSENGER, OTHER_WS, WHATSAPP, WS, coexist_db, exec, put_runs, put_staging, read, run_row,
    write,
};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

fn ids(runs: &[chatbotx_db::repo::models::CoexistSyncRun]) -> Vec<i64> {
    runs.iter().map(|r| r.id).collect()
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_find_active_init_run_filters_type_coexist() {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(1, 62, "instagram", ("contact_scan", "init"), now_ms(), "")],
    )
    .await;
    let none = read(&db, |c| csr::find_active_init_run(c, 62, "instagram")).await;
    assert!(none.is_none());
    put_runs(
        &db,
        &[(2, 62, "instagram", ("coexist", "init"), now_ms(), "")],
    )
    .await;
    let found = read(&db, |c| csr::find_active_init_run(c, 62, "instagram")).await;
    assert_eq!(found.expect("coexist init run").id, 2);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_find_live_run_filters_type_coexist() {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                WHATSAPP,
                "whatsapp",
                ("coexist", "waiting"),
                now - HOUR,
                "",
            ),
            (
                2,
                WHATSAPP,
                "whatsapp",
                ("contact_scan", "running"),
                now,
                "",
            ),
        ],
    )
    .await;
    let found = read(&db, |c| csr::find_live_run(c, WHATSAPP, "whatsapp")).await;
    assert_eq!(found.expect("live coexist run").id, 1);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_find_resume_ceiling_filters_type_coexist() {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                MESSENGER,
                "messenger",
                ("coexist", "succeeded"),
                now,
                "\"startedAt\" = 1000",
            ),
            (
                2,
                MESSENGER,
                "messenger",
                ("contact_scan", "succeeded"),
                now,
                "\"startedAt\" = 5000",
            ),
            (3, MESSENGER, "messenger", ("coexist", "running"), now, ""),
        ],
    )
    .await;
    let ceiling = read(&db, |c| {
        csr::find_resume_ceiling(c, MESSENGER, "messenger", 3)
    })
    .await;
    assert_eq!(ceiling, Some(1000));
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_tear_down_active_runs_for_integration_filters_type_coexist()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (1, WHATSAPP, "whatsapp", ("coexist", "running"), now, ""),
            (
                2,
                WHATSAPP,
                "whatsapp",
                ("contact_scan", "running"),
                now,
                "",
            ),
        ],
    )
    .await;
    write(&db, |c| {
        csr::tear_down_active_runs_for_integration(c, "whatsapp", WHATSAPP, "Coexist disabled")
    })
    .await;
    assert_eq!(run_row(&db, 1).await.expect("run 1").status, "failed");
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "running");
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_mark_max_attempts_failed_applies_the_caller_s_type_filter()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                MESSENGER,
                "messenger",
                ("coexist", "running"),
                now,
                "\"attempts\" = 5",
            ),
            (
                2,
                MESSENGER,
                "messenger",
                ("contact_scan", "running"),
                now,
                "\"attempts\" = 5",
            ),
        ],
    )
    .await;
    write(&db, |c| csr::mark_max_attempts_failed(c, "coexist", 5)).await;
    assert_eq!(run_row(&db, 1).await.expect("run 1").status, "failed");
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "running");
    write(&db, |c| csr::mark_max_attempts_failed(c, "contact_scan", 5)).await;
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "failed");
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_pick_due_runs_binds_the_caller_s_type_as_a_sql_parameter()
 {
    let db = coexist_db().await;
    let old = now_ms() - HOUR;
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "init"), old, ""),
            (2, MESSENGER, "messenger", ("contact_scan", "init"), old, ""),
        ],
    )
    .await;
    let coexist = write(&db, |c| csr::pick_due_runs(c, "coexist", 10, 5)).await;
    assert_eq!(coexist.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1]);
    let scan = write(&db, |c| csr::pick_due_runs(c, "contact_scan", 10, 5)).await;
    assert_eq!(scan.iter().map(|p| p.id).collect::<Vec<_>>(), vec![2]);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_revive_waiting_runs_with_pending_staging_scopes_type_coexist_inline_in_the_sql_text()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            WHATSAPP,
            "whatsapp",
            ("contact_scan", "waiting"),
            now_ms() - HOUR,
            "",
        )],
    )
    .await;
    db.write(|tx| put_staging(tx, 1, "phone-63", "pending"))
        .await
        .expect("staging");
    let revived = write(&db, |c| {
        csr::revive_waiting_runs_with_pending_staging(c, 10)
    })
    .await;
    assert!(revived.is_empty());
    assert_eq!(run_row(&db, 1).await.expect("run").status, "waiting");
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_finalize_timed_out_waiting_runs_scopes_type_coexist_inline_in_the_sql_text()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            WHATSAPP,
            "whatsapp",
            ("contact_scan", "waiting"),
            now_ms() - 2 * HOUR,
            "",
        )],
    )
    .await;
    let finalized = write(&db, |c| {
        csr::finalize_timed_out_waiting_runs(c, MIN, "history_timeout", 10)
    })
    .await;
    assert!(finalized.is_empty());
    assert_eq!(run_row(&db, 1).await.expect("run").status, "waiting");
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_find_stranded_coexist_whatsapp_integrations_scopes_type_coexist_inline_in_the_sql_text()
 {
    let db = coexist_db().await;
    exec(
        &db,
        r#"UPDATE "IntegrationWhatsapp" SET "coexistEnabled" = 1 WHERE "id" = 63"#,
    )
    .await;
    db.write(|tx| put_staging(tx, 1, "phone-63", "pending"))
        .await
        .expect("staging");
    // A live scan run does not count as a live coexist run.
    put_runs(
        &db,
        &[(
            1,
            WHATSAPP,
            "whatsapp",
            ("contact_scan", "running"),
            now_ms(),
            "",
        )],
    )
    .await;
    let stranded = write(&db, |c| {
        csr::find_stranded_coexist_whatsapp_integrations(c, 10)
    })
    .await;
    assert_eq!(
        stranded,
        vec![StrandedIntegration {
            integration_id: WHATSAPP,
            workspace_id: WS,
        }]
    );
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_create_contact_scan_run_inserts_type_contact_scan()
 {
    let db = coexist_db().await;
    let input = CreateContactScanRunInput {
        workspace_id: WS,
        integration_id: WHATSAPP,
        channel: "whatsapp",
        requested_by_user_id: Some(1001),
        scan_from_at: ms("2026-01-01T00:00:00Z"),
        trigger_source: "manual",
    };
    let run = write(&db, move |c| csr::create_contact_scan_run(c, &input)).await;
    let run = run.expect("inserted");
    assert_eq!(run.r#type, "contact_scan");
    assert_eq!(run.status, "init");
    assert_eq!(run.scan_from_at, Some(ms("2026-01-01T00:00:00Z")));
    assert_eq!(run.requested_by_user_id, Some(1001));
    assert_eq!(run.trigger_source, "manual");
    assert_eq!(run.pending_patches, None);
    assert_eq!(run.claim_token, None);
    assert_eq!(run.resume_cursor, None);
    // The active-scan unique index refuses a second live scan of the integration.
    let second = write(&db, move |c| csr::create_contact_scan_run(c, &input)).await;
    assert!(second.is_none());
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_find_latest_contact_scan_run_filters_type_contact_scan()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                MESSENGER,
                "messenger",
                ("contact_scan", "succeeded"),
                now - HOUR,
                "",
            ),
            (2, MESSENGER, "messenger", ("coexist", "succeeded"), now, ""),
        ],
    )
    .await;
    let found = read(&db, |c| csr::find_latest_contact_scan_run(c, WS, MESSENGER)).await;
    assert_eq!(found.expect("scan run").id, 1);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_claim_contact_scan_run_filters_type_contact_scan_in_its_update_where()
 {
    let db = coexist_db().await;
    let now = now_ms();
    let released = format!("\"lastHeartbeatAt\" = {now}");
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "init"), now, ""),
            (2, MESSENGER, "messenger", ("contact_scan", "init"), now, ""),
            // Released by `yieldForContinuation` (running, no token): claimable at once.
            (
                3,
                WHATSAPP,
                "whatsapp",
                ("contact_scan", "running"),
                now,
                &released,
            ),
        ],
    )
    .await;
    let coexist = write(&db, |c| csr::claim_contact_scan_run(c, 1)).await;
    assert!(coexist.is_none());
    assert_eq!(run_row(&db, 1).await.expect("run 1").status, "init");
    for id in [2, 3] {
        let claimed = write(&db, move |c| csr::claim_contact_scan_run(c, id)).await;
        let claimed = claimed.expect("scan run claimed");
        assert_eq!(claimed.status, "running");
        assert!(claimed.claim_token.is_some());
    }
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_yield_for_continuation_releases_the_token_keeps_running_and_is_type_claim_fenced()
 {
    let db = coexist_db().await;
    let old = now_ms() - HOUR;
    let token1 = format!("\"claimToken\" = 'tok-1', \"lastHeartbeatAt\" = {old}");
    let token2 = "\"claimToken\" = 'tok-2'";
    put_runs(
        &db,
        &[
            (
                1,
                71,
                "messenger",
                ("contact_scan", "running"),
                old,
                &token1,
            ),
            (2, 72, "messenger", ("coexist", "running"), old, &token1),
            (3, 73, "messenger", ("contact_scan", "running"), old, token2),
        ],
    )
    .await;
    let guard = || WriteGuard {
        status: "running",
        claim_token: Some("tok-1"),
    };
    for (id, expected) in [(1, 1), (2, 0), (3, 0)] {
        let affected = write(&db, move |c| csr::yield_for_continuation(c, id, &guard())).await;
        assert_eq!(affected, expected, "run {id}");
    }
    let row = run_row(&db, 1).await.expect("run 1");
    assert_eq!(row.status, "running");
    assert_eq!(row.claim_token, None);
    assert!(row.last_heartbeat_at.unwrap_or_default() > old);
    assert_eq!(
        run_row(&db, 2).await.expect("run 2").claim_token.as_deref(),
        Some("tok-1")
    );
    assert_eq!(
        run_row(&db, 3).await.expect("run 3").claim_token.as_deref(),
        Some("tok-2")
    );
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_reopen_released_only_reopens_a_released_running_null_token_contact_scan_run()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (1, 71, "messenger", ("contact_scan", "running"), now, ""),
            (
                2,
                72,
                "messenger",
                ("contact_scan", "running"),
                now,
                "\"claimToken\" = 'tok'",
            ),
            (3, 73, "messenger", ("coexist", "running"), now, ""),
            (4, 74, "messenger", ("contact_scan", "succeeded"), now, ""),
        ],
    )
    .await;
    for (id, expected) in [(1, 1), (2, 0), (3, 0), (4, 0)] {
        let affected = write(&db, move |c| csr::reopen_released(c, id)).await;
        assert_eq!(affected, expected, "run {id}");
    }
    assert_eq!(run_row(&db, 1).await.expect("run 1").status, "init");
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "running");
    assert_eq!(run_row(&db, 3).await.expect("run 3").status, "running");
    assert_eq!(run_row(&db, 4).await.expect("run 4").status, "succeeded");
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_list_contact_scan_runs_filters_type_contact_scan_and_stays_workspace_scoped()
 {
    let db = coexist_db().await;
    // Nothing yet: an empty page and pageCount 0.
    let empty = read(&db, |c| {
        csr::list_contact_scan_runs(
            c,
            &ListContactScanRunsInput {
                workspace_id: WS,
                page: Some(2),
                per_page: Some(10),
                ..Default::default()
            },
        )
    })
    .await;
    assert!(empty.data.is_empty());
    assert_eq!(empty.page_count, 0);
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                71,
                "messenger",
                ("contact_scan", "succeeded"),
                now - 3 * MIN,
                "",
            ),
            (
                2,
                72,
                "messenger",
                ("contact_scan", "succeeded"),
                now - 2 * MIN,
                "",
            ),
            (
                3,
                73,
                "messenger",
                ("contact_scan", "succeeded"),
                now - MIN,
                &format!("\"workspaceId\" = {OTHER_WS}"),
            ),
            (4, 74, "messenger", ("coexist", "succeeded"), now, ""),
        ],
    )
    .await;
    let page = read(&db, |c| {
        csr::list_contact_scan_runs(
            c,
            &ListContactScanRunsInput {
                workspace_id: WS,
                ..Default::default()
            },
        )
    })
    .await;
    assert_eq!(ids(&page.data), vec![2, 1]);
    assert_eq!(page.page_count, 1);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_list_contact_scan_runs_narrows_to_one_integration_when_asked()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (
                1,
                71,
                "messenger",
                ("contact_scan", "succeeded"),
                now - MIN,
                "",
            ),
            (2, 72, "messenger", ("contact_scan", "succeeded"), now, ""),
        ],
    )
    .await;
    let page = read(&db, |c| {
        csr::list_contact_scan_runs(
            c,
            &ListContactScanRunsInput {
                workspace_id: WS,
                integration_id: Some(71),
                ..Default::default()
            },
        )
    })
    .await;
    assert_eq!(ids(&page.data), vec![1]);
    assert_eq!(page.page_count, 1);
}

#[tokio::test]
async fn coexist_sync_run_type_discriminator_scoping_list_contact_scan_runs_paginates_and_defaults_to_created_at_desc()
 {
    let db = coexist_db().await;
    let base = now_ms() - HOUR;
    let specs: Vec<(i64, i64)> = (1..=25).map(|i| (i, base + i * 1000)).collect();
    db.write(move |tx| {
        for (id, at) in specs {
            repo10::put_run(
                tx,
                id,
                70,
                "messenger",
                ("contact_scan", "succeeded"),
                at,
                "",
            )?;
        }
        Ok(())
    })
    .await
    .expect("scan runs");
    let page = read(&db, |c| {
        csr::list_contact_scan_runs(
            c,
            &ListContactScanRunsInput {
                workspace_id: WS,
                page: Some(2),
                per_page: Some(10),
                ..Default::default()
            },
        )
    })
    .await;
    assert_eq!(ids(&page.data), (6..=15).rev().collect::<Vec<i64>>());
    assert_eq!(page.page_count, 3);
}
