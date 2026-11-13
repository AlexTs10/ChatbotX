//! Port of `packages/database/__tests__/coexist-run-recovery-repository.test.ts` (P3.2b).
//! The TS checks the emitted Postgres SQL text / bind values and the Drizzle `set` objects
//! of mocked clients; here the recovery queries run on a migrated temp file (adapted), so
//! each SQL fragment shows as the rows it picks or spares. `FOR UPDATE SKIP LOCKED` has no
//! SQLite form (one writer at a time); that part of the text checks is not ported.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::coexist_sync_run::{
    self as csr, CreateRunInput, LIVE_RUN_STATUSES, PickedCoexistRun, StrandedIntegration,
    WriteGuard,
};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::types::Value;
use repo10::{
    MESSENGER, OTHER_WS, WHATSAPP, WHATSAPP_2, WHATSAPP_OTHER, WS, coexist_db, exec, put_runs,
    put_staging, run_row, write,
};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
    lens == [8, 4, 4, 4, 12]
        && parts.iter().all(|p| {
            p.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
}

async fn stage(db: &chatbotx_db::Db, rows: &'static [(i64, &'static str, &'static str)]) {
    db.write(move |tx| {
        for (id, phone, state) in rows {
            put_staging(tx, *id, phone, state)?;
        }
        Ok(())
    })
    .await
    .expect("staging rows");
}

fn step(text: &str) -> Vec<(&'static str, Value)> {
    vec![("currentStep", Value::Text(text.to_owned()))]
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_revive_waiting_runs_with_pending_staging_flips_waiting_init_without_touching_attempts()
 {
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
                "\"attempts\" = 2",
            ),
            (
                2,
                WHATSAPP_2,
                "whatsapp",
                ("coexist", "waiting"),
                now - HOUR,
                "",
            ),
            (
                3,
                MESSENGER,
                "messenger",
                ("coexist", "waiting"),
                now - HOUR,
                "",
            ),
        ],
    )
    .await;
    // phone-63 has a pending row; phone-64 only a processed and a parse-failed one.
    stage(
        &db,
        &[
            (1, "phone-63", "pending"),
            (2, "phone-64", "processed"),
            (3, "phone-64", "parse_failed"),
        ],
    )
    .await;
    let revived = write(&db, |c| {
        csr::revive_waiting_runs_with_pending_staging(c, 50)
    })
    .await;
    assert_eq!(
        revived,
        vec![PickedCoexistRun {
            id: 1,
            attempts: 2,
            channel: "whatsapp".to_owned(),
            integration_id: WHATSAPP,
            workspace_id: WS,
        }]
    );
    let row = run_row(&db, 1).await.expect("run 1");
    assert_eq!(row.status, "init");
    assert_eq!(row.attempts, 2);
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "waiting");
    assert_eq!(run_row(&db, 3).await.expect("run 3").status, "waiting");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_pick_due_runs_gates_init_rows_on_updated_at_not_created_at()
 {
    let db = coexist_db().await;
    let now = now_ms();
    let recent_update = format!(
        "\"createdAt\" = {}, \"updatedAt\" = {}",
        now - HOUR,
        now - 2_000
    );
    let old_heartbeat = format!("\"lastHeartbeatAt\" = {}", now - 2 * HOUR);
    let new_heartbeat = format!("\"lastHeartbeatAt\" = {}", now - 30 * MIN);
    put_runs(
        &db,
        &[
            // createdAt is old but updatedAt is fresh: not due.
            (
                1,
                MESSENGER,
                "messenger",
                ("coexist", "init"),
                now,
                &recent_update,
            ),
            // updatedAt older than 10 s: due.
            (2, 62, "instagram", ("coexist", "init"), now - 20_000, ""),
            // running with a heartbeat older than 1 h: due.
            (
                3,
                WHATSAPP,
                "whatsapp",
                ("coexist", "running"),
                now - 3 * HOUR,
                &old_heartbeat,
            ),
            // running with a heartbeat of 30 min: not due.
            (
                4,
                WHATSAPP_2,
                "whatsapp",
                ("coexist", "running"),
                now - 4 * HOUR,
                &new_heartbeat,
            ),
            // out of attempts: not due.
            (
                5,
                99,
                "whatsapp",
                ("coexist", "init"),
                now - HOUR,
                "\"attempts\" = 5",
            ),
        ],
    )
    .await;
    let picked = write(&db, |c| csr::pick_due_runs(c, "coexist", 10, 5)).await;
    // By createdAt ascending; attempts + 1.
    let ids: Vec<(i64, i64)> = picked.iter().map(|p| (p.id, p.attempts)).collect();
    assert_eq!(ids, vec![(3, 1), (2, 1)]);
    for id in [2, 3] {
        let row = run_row(&db, id).await.expect("picked row");
        assert_eq!(row.status, "init");
        assert_eq!(row.attempts, 1);
        assert!(row.updated_at >= now);
    }
    assert_eq!(run_row(&db, 1).await.expect("run 1").attempts, 0);
    assert_eq!(run_row(&db, 4).await.expect("run 4").status, "running");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_revive_waiting_runs_with_pending_staging_stamps_updated_at_so_the_same_tick_s_pick_skips_it()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[(
            1,
            WHATSAPP,
            "whatsapp",
            ("coexist", "waiting"),
            now - HOUR,
            "",
        )],
    )
    .await;
    stage(&db, &[(1, "phone-63", "pending")]).await;
    let revived = write(&db, |c| {
        csr::revive_waiting_runs_with_pending_staging(c, 10)
    })
    .await;
    assert_eq!(revived.len(), 1);
    assert!(run_row(&db, 1).await.expect("run").updated_at >= now);
    let picked = write(&db, |c| csr::pick_due_runs(c, "coexist", 10, 5)).await;
    assert!(picked.is_empty());
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_finalize_timed_out_waiting_runs_closes_stale_waiting_runs_as_partial_with_the_sentinel()
 {
    let db = coexist_db().await;
    let now = now_ms();
    let started_old = format!("\"startedAt\" = {}", now - 25 * HOUR);
    let started_new = format!("\"startedAt\" = {}", now - HOUR);
    put_runs(
        &db,
        &[
            (
                1,
                WHATSAPP_2,
                "whatsapp",
                ("coexist", "waiting"),
                now - 30 * HOUR,
                &started_old,
            ),
            // Old, but its staging still has a pending row (NOT EXISTS).
            (
                2,
                WHATSAPP,
                "whatsapp",
                ("coexist", "waiting"),
                now - 25 * HOUR,
                "",
            ),
            // Inside the window.
            (
                3,
                99,
                "whatsapp",
                ("coexist", "waiting"),
                now - 30 * HOUR,
                &started_new,
            ),
        ],
    )
    .await;
    stage(&db, &[(1, "phone-63", "pending")]).await;
    let finalized = write(&db, |c| {
        csr::finalize_timed_out_waiting_runs(c, 24 * HOUR, "history_timeout", 100)
    })
    .await;
    assert_eq!(finalized, vec![1]);
    let row = run_row(&db, 1).await.expect("run 1");
    assert_eq!(row.status, "partial");
    assert_eq!(row.current_error.as_deref(), Some("history_timeout"));
    assert!(row.finished_at.unwrap_or_default() >= now);
    assert_eq!(run_row(&db, 2).await.expect("run 2").status, "waiting");
    assert_eq!(run_row(&db, 3).await.expect("run 3").status, "waiting");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_find_stranded_coexist_whatsapp_integrations_excludes_integrations_with_a_live_run()
 {
    let db = coexist_db().await;
    let now = now_ms();
    exec(
        &db,
        r#"UPDATE "IntegrationWhatsapp" SET "coexistEnabled" = 1"#,
    )
    .await;
    stage(
        &db,
        &[
            (1, "phone-63", "pending"),
            (2, "phone-64", "pending"),
            (3, "phone-65", "pending"),
        ],
    )
    .await;
    put_runs(
        &db,
        &[
            (1, WHATSAPP_2, "whatsapp", ("coexist", "waiting"), now, ""),
            (
                2,
                WHATSAPP_OTHER,
                "whatsapp",
                ("coexist", "succeeded"),
                now,
                "",
            ),
        ],
    )
    .await;
    let stranded = write(&db, |c| {
        csr::find_stranded_coexist_whatsapp_integrations(c, 25)
    })
    .await;
    assert_eq!(
        stranded,
        vec![
            StrandedIntegration {
                integration_id: WHATSAPP,
                workspace_id: WS,
            },
            StrandedIntegration {
                integration_id: WHATSAPP_OTHER,
                workspace_id: OTHER_WS,
            },
        ]
    );
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_create_run_writes_pending_patches_and_claim_token_explicitly_no_phantom_defaults()
 {
    let db = coexist_db().await;
    let run = write(&db, |c| {
        csr::create_run(
            c,
            &CreateRunInput {
                workspace_id: WS,
                integration_id: WHATSAPP,
                channel: "whatsapp",
                trigger_source: "scheduler-recovery",
            },
        )
    })
    .await;
    let row = run_row(&db, run.id).await.expect("row");
    assert_eq!(row.trigger_source, "scheduler-recovery");
    assert_eq!(row.pending_patches, None);
    assert_eq!(row.claim_token, None);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_update_progress_pins_status_running_when_a_guard_is_given()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "running"), now, ""),
            (2, WHATSAPP, "whatsapp", ("coexist", "waiting"), now, ""),
        ],
    )
    .await;
    for (id, expected) in [(1, 1), (2, 0)] {
        let affected = write(&db, move |c| {
            let guard = WriteGuard {
                status: "running",
                claim_token: None,
            };
            csr::update_progress(c, id, &step("flushing batch 1"), Some(&guard))
        })
        .await;
        assert_eq!(affected, expected, "run {id}");
    }
    assert_eq!(
        run_row(&db, 1)
            .await
            .expect("run 1")
            .current_step
            .as_deref(),
        Some("flushing batch 1")
    );
    assert_eq!(run_row(&db, 2).await.expect("run 2").current_step, None);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_update_progress_also_pins_the_ownership_token_when_the_caller_holds_one()
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
            "\"claimToken\" = 'token-a'",
        )],
    )
    .await;
    for (token, expected) in [("token-b", 0), ("token-a", 1)] {
        let affected = write(&db, move |c| {
            let guard = WriteGuard {
                status: "running",
                claim_token: Some(token),
            };
            csr::update_progress(c, 1, &step("flushing batch 1"), Some(&guard))
        })
        .await;
        assert_eq!(affected, expected, "token {token}");
    }
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_a_null_absent_token_leaves_the_predicate_token_free()
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
            "\"claimToken\" = 'token-z'",
        )],
    )
    .await;
    let affected = write(&db, |c| {
        let guard = WriteGuard {
            status: "running",
            claim_token: None,
        };
        csr::update_progress(c, 1, &step("flushing batch 1"), Some(&guard))
    })
    .await;
    assert_eq!(affected, 1);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_update_progress_reports_0_when_the_guard_matches_nothing()
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
            "\"claimToken\" = 'token-b'",
        )],
    )
    .await;
    let affected = write(&db, |c| {
        let guard = WriteGuard {
            status: "running",
            claim_token: Some("token-a"),
        };
        csr::update_progress(
            c,
            1,
            &vec![("status", Value::Text("waiting".into()))],
            Some(&guard),
        )
    })
    .await;
    assert_eq!(affected, 0);
    assert_eq!(run_row(&db, 1).await.expect("run").status, "running");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_update_progress_without_a_guard_keeps_the_id_only_predicate()
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
                ("coexist", "succeeded"),
                now,
                "\"claimToken\" = 'x'",
            ),
            (2, WHATSAPP, "whatsapp", ("coexist", "running"), now, ""),
        ],
    )
    .await;
    let affected = write(&db, |c| {
        csr::update_progress(c, 1, &step("listing conversations"), None)
    })
    .await;
    assert_eq!(affected, 1);
    let row = run_row(&db, 1).await.expect("run 1");
    assert_eq!(row.current_step.as_deref(), Some("listing conversations"));
    assert!(row.updated_at >= now);
    assert_eq!(run_row(&db, 2).await.expect("run 2").current_step, None);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_mark_failed_forwards_the_guard()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now,
            "\"claimToken\" = 'token-b', \"resumeCursor\" = 'c'",
        )],
    )
    .await;
    let mark = |token: &'static str| {
        move |c: &chatbotx_db::rusqlite::Connection| {
            let guard = WriteGuard {
                status: "running",
                claim_token: Some(token),
            };
            csr::mark_failed(c, 1, "workspaceId mismatch", Some(&guard))
        }
    };
    assert_eq!(write(&db, mark("token-a")).await, 0);
    assert_eq!(run_row(&db, 1).await.expect("run").status, "running");
    assert_eq!(write(&db, mark("token-b")).await, 1);
    let row = run_row(&db, 1).await.expect("run");
    assert_eq!(row.status, "failed");
    assert_eq!(row.current_error.as_deref(), Some("workspaceId mismatch"));
    assert!(row.finished_at.unwrap_or_default() >= now);
    assert_eq!(row.resume_cursor, None);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_claim_run_with_new_token_mints_a_fresh_ownership_token_and_returns_the_claimed_row()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "init"),
            now - HOUR,
            "",
        )],
    )
    .await;
    let claimed = write(&db, |c| csr::claim_run_with_new_token(c, 1, None))
        .await
        .expect("claimed");
    assert_eq!(claimed.status, "running");
    assert!(
        is_uuid(claimed.claim_token.as_deref().unwrap_or_default()),
        "{:?}",
        claimed.claim_token
    );
    assert!(claimed.updated_at >= now);
    assert!(claimed.started_at.unwrap_or_default() >= now);
    assert!(claimed.last_heartbeat_at.unwrap_or_default() >= now);
    let row = run_row(&db, 1).await.expect("row");
    assert_eq!(row.claim_token, claimed.claim_token);
    assert_eq!(row.status, "running");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_two_claims_never_mint_the_same_token()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(1, MESSENGER, "messenger", ("coexist", "init"), now_ms(), "")],
    )
    .await;
    let mut tokens = Vec::new();
    for _ in 0..2 {
        let claimed = write(&db, |c| csr::claim_run_with_new_token(c, 1, None))
            .await
            .expect("claimed");
        tokens.push(claimed.claim_token.expect("token"));
        // Let the claim go stale so the next one can take it.
        exec(
            &db,
            r#"UPDATE "CoexistSyncRun" SET "lastHeartbeatAt" = 0 WHERE "id" = 1"#,
        )
        .await;
    }
    assert_ne!(tokens[0], tokens[1]);
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_claim_run_with_new_token_widens_to_the_live_statuses_when_asked_whatsapp()
 {
    let db = coexist_db().await;
    put_runs(
        &db,
        &[(
            1,
            WHATSAPP,
            "whatsapp",
            ("coexist", "waiting"),
            now_ms(),
            "",
        )],
    )
    .await;
    let narrow = write(&db, |c| csr::claim_run_with_new_token(c, 1, None)).await;
    assert!(narrow.is_none());
    let wide = write(&db, |c| {
        csr::claim_run_with_new_token(c, 1, Some(LIVE_RUN_STATUSES))
    })
    .await;
    assert_eq!(wide.expect("claimed from waiting").status, "running");
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_guarded_writes_and_the_ownership_claim_claim_run_with_new_token_returns_null_when_another_worker_holds_the_run()
 {
    let db = coexist_db().await;
    let now = now_ms();
    let held = format!(
        "\"claimToken\" = 'other', \"lastHeartbeatAt\" = {}",
        now - MIN
    );
    put_runs(
        &db,
        &[(
            1,
            MESSENGER,
            "messenger",
            ("coexist", "running"),
            now,
            &held,
        )],
    )
    .await;
    let claimed = write(&db, |c| csr::claim_run_with_new_token(c, 1, None)).await;
    assert!(claimed.is_none());
    assert_eq!(
        run_row(&db, 1).await.expect("run").claim_token.as_deref(),
        Some("other")
    );
}

#[tokio::test]
async fn coexist_sync_run_repository_recovery_queries_tear_down_active_runs_for_integration_also_tears_down_waiting_runs()
 {
    let db = coexist_db().await;
    let now = now_ms();
    put_runs(
        &db,
        &[
            (1, WHATSAPP, "whatsapp", ("coexist", "init"), now, ""),
            (2, WHATSAPP, "whatsapp", ("coexist", "running"), now, ""),
            (3, WHATSAPP, "whatsapp", ("coexist", "waiting"), now, ""),
            (4, WHATSAPP, "whatsapp", ("coexist", "succeeded"), now, ""),
            (5, WHATSAPP_2, "whatsapp", ("coexist", "waiting"), now, ""),
        ],
    )
    .await;
    write(&db, |c| {
        csr::tear_down_active_runs_for_integration(c, "whatsapp", WHATSAPP, "Coexist disabled")
    })
    .await;
    for id in [1, 2, 3] {
        let row = run_row(&db, id).await.expect("row");
        assert_eq!(row.status, "failed", "run {id}");
        assert_eq!(row.current_error.as_deref(), Some("Coexist disabled"));
        assert!(row.finished_at.is_some());
    }
    assert_eq!(run_row(&db, 4).await.expect("run 4").status, "succeeded");
    assert_eq!(run_row(&db, 5).await.expect("run 5").status, "waiting");
}
