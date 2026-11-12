//! Port of `packages/database/__tests__/integration/ai-file-processing-run.test.ts` (P3.2).
//! Runs against a migrated SQLite temp file (one fresh file per test instead of a
//! rolled-back Postgres transaction). Ids keep the TS digits.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::ai_file_embedding::{
    self as afe, AiFileChunkCursor, AiFileEmbeddingChunk, ChunkingConfigUpdate,
};
use chatbotx_db::repo::{self};
use repo_common::ai_file::{
    FILE, FRESH, NOW, OLD, STALE_BEFORE, WS, build_run, chunk_rows, fixture, insert_chunks,
    read_config, read_run,
};
use repo_common::ok;
use serde_json::{Value, json};

async fn claim(db: &Db, workspace: i64, run: Value, config: ChunkingConfigUpdate) -> bool {
    ok(repo::write(db, move |tx| {
        afe::claim_processing_run(tx, FILE, workspace, &run, STALE_BEFORE, &config)
    })
    .await)
}

async fn claim_case(existing: Option<Value>, expected: bool) {
    let db = fixture(existing.clone()).await;
    let next = build_run(json!({ "id": "run-next" }));
    assert_eq!(
        claim(&db, WS, next.clone(), ChunkingConfigUpdate::Keep).await,
        expected
    );
    let stored = read_run(&db).await;
    assert_eq!(stored, if expected { Some(next) } else { existing });
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (no run (NULL) → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_no_run_null_true()
{
    claim_case(None, true).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (a finished run → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_a_finished_run_true()
 {
    claim_case(
        Some(build_run(json!({ "status": "success", "finishedAt": NOW }))),
        true,
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (a failed run → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_a_failed_run_true()
 {
    claim_case(
        Some(build_run(json!({ "status": "error", "error": "unknown" }))),
        true,
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (an active run within the lease → false)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_an_active_run_within_the_lease_false()
 {
    claim_case(Some(build_run(json!({ "queuedAt": FRESH }))), false).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (an unstarted run stale by queuedAt → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_an_unstarted_run_stale_by_queued_at_true()
 {
    claim_case(
        Some(build_run(json!({ "queuedAt": OLD, "heartbeatAt": null }))),
        true,
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (an old run with a fresh heartbeat → false)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_an_old_run_with_a_fresh_heartbeat_false()
 {
    claim_case(
        Some(build_run(
            json!({ "status": "processing", "queuedAt": OLD, "heartbeatAt": FRESH }),
        )),
        false,
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (a run whose queuedAt and heartbeatAt are both old → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_a_run_whose_queued_at_and_heartbeat_at_are_both_old_true()
 {
    claim_case(
        Some(build_run(
            json!({ "status": "processing", "queuedAt": OLD, "heartbeatAt": OLD }),
        )),
        true,
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:165 "AiFileEmbeddingRepository processing runs against Postgres > claim over %s → %s" (jsonb without a status → true)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_over_jsonb_without_a_status_true()
 {
    claim_case(Some(json!({ "id": "broken" })), true).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:180 "AiFileEmbeddingRepository processing runs against Postgres > claim is scoped to the workspace"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_is_scoped_to_the_workspace()
 {
    let db = fixture(None).await;
    assert!(
        !claim(
            &db,
            999_999_999,
            build_run(json!({})),
            ChunkingConfigUpdate::Keep
        )
        .await
    );
}

async fn started(db: &Db, now: &str) -> bool {
    let now = now.to_owned();
    ok(repo::write(db, move |tx| {
        afe::mark_processing_run_started(tx, FILE, WS, "run-active", &now)
    })
    .await)
}

async fn lease(db: &Db, at: &str) -> bool {
    let at = at.to_owned();
    ok(repo::write(db, move |tx| {
        afe::refresh_processing_run_lease(tx, FILE, WS, "run-active", &at)
    })
    .await)
}

async fn finish(db: &Db, status: &str, error: Option<&str>, at: &str) -> bool {
    let (status, error, at) = (status.to_owned(), error.map(str::to_owned), at.to_owned());
    ok(repo::write(db, move |tx| {
        afe::finish_processing_run(tx, FILE, WS, "run-active", &status, error.as_deref(), &at)
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:193 "AiFileEmbeddingRepository processing runs against Postgres > start keeps the first startedAt, then lease and finish patch only their keys"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_start_keeps_the_first_started_at_then_lease_and_finish_patch_only_their_keys()
 {
    let db = fixture(Some(build_run(json!({})))).await;
    assert!(started(&db, NOW).await);
    assert!(started(&db, "2026-10-24T10:01:00.000Z").await);
    assert!(lease(&db, "2026-10-24T10:02:00.000Z").await);
    assert!(finish(&db, "success", None, "2026-10-24T10:03:00.000Z").await);
    assert_eq!(
        read_run(&db).await,
        Some(build_run(json!({
            "status": "success",
            "startedAt": NOW,
            "heartbeatAt": "2026-10-24T10:02:00.000Z",
            "finishedAt": "2026-10-24T10:03:00.000Z",
        })))
    );
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:231 "AiFileEmbeddingRepository processing runs against Postgres > records an error code and ignores a second finish"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_records_an_error_code_and_ignores_a_second_finish()
 {
    let db = fixture(Some(build_run(json!({ "status": "processing" })))).await;
    assert!(finish(&db, "error", Some("fileUnreadable"), NOW).await);
    assert!(!finish(&db, "success", None, NOW).await);
    let run = read_run(&db).await.expect("run");
    assert_eq!(run["status"], "error");
    assert_eq!(run["error"], "fileUnreadable");
}

async fn lease_refused(existing: Value) {
    let db = fixture(Some(existing)).await;
    assert!(!lease(&db, NOW).await);
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:264 "AiFileEmbeddingRepository processing runs against Postgres > lease refresh is refused for a %s run" (queued)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_lease_refresh_is_refused_for_a_queued_run()
 {
    lease_refused(build_run(json!({}))).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:264 "AiFileEmbeddingRepository processing runs against Postgres > lease refresh is refused for a %s run" (success)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_lease_refresh_is_refused_for_a_success_run()
 {
    lease_refused(build_run(json!({ "status": "success" }))).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:264 "AiFileEmbeddingRepository processing runs against Postgres > lease refresh is refused for a %s run" (error)
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_lease_refresh_is_refused_for_a_error_run()
 {
    lease_refused(build_run(json!({ "status": "error", "error": "unknown" }))).await;
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:276 "AiFileEmbeddingRepository processing runs against Postgres > a superseded run can no longer refresh its lease or finish"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_a_superseded_run_can_no_longer_refresh_its_lease_or_finish()
 {
    let db = fixture(Some(build_run(
        json!({ "status": "processing", "queuedAt": OLD, "heartbeatAt": OLD }),
    )))
    .await;
    assert!(
        claim(
            &db,
            WS,
            build_run(json!({ "id": "run-next" })),
            ChunkingConfigUpdate::Keep
        )
        .await
    );
    assert!(!lease(&db, NOW).await);
    assert!(!started(&db, NOW).await);
    assert!(!finish(&db, "success", None, NOW).await);
}

async fn reconcile(db: &Db, chunks: Vec<(i64, &str, i64)>) -> Vec<i64> {
    let chunks: Vec<AiFileEmbeddingChunk> = chunks
        .into_iter()
        .map(|(id, content, chunk_index)| AiFileEmbeddingChunk {
            id,
            content: content.to_owned(),
            chunk_index,
        })
        .collect();
    ok(repo::write(db, move |tx| {
        afe::reconcile_pending_chunks(tx, FILE, WS, &chunks)
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:306 "AiFileEmbeddingRepository processing runs against Postgres > reconcile retries the error chunks it keeps and leaves processing and success alone"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_reconcile_retries_the_error_chunks_it_keeps_and_leaves_processing_and_success_alone()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (910000000000000001, None, "910000000000000001", "error"),
                (910000000000000002, None, "910000000000000002", "processing"),
                (910000000000000003, None, "910000000000000003", "success"),
            ],
        );
        Ok(())
    })
    .await;
    let pending = reconcile(
        &db,
        vec![
            (910000000000000001, "a", 0),
            (910000000000000002, "b", 1),
            (910000000000000003, "c", 2),
        ],
    )
    .await;
    assert_eq!(pending, vec![910000000000000001]);
    let rows = chunk_rows(&db).await;
    let status = |id: i64| rows.iter().find(|r| r.0 == id).map(|r| r.2.clone());
    assert_eq!(status(910000000000000002).as_deref(), Some("processing"));
    assert_eq!(status(910000000000000003).as_deref(), Some("success"));
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:380 "AiFileEmbeddingRepository processing runs against Postgres > claim writes, resets or keeps the chunking parameters with the run"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_claim_writes_resets_or_keeps_the_chunking_parameters_with_the_run()
 {
    let db = fixture(None).await;
    let new_run = || build_run(json!({ "id": "run-new" }));
    assert!(
        claim(
            &db,
            WS,
            new_run(),
            ChunkingConfigUpdate::Set(json!({ "chunkSize": 500 }))
        )
        .await
    );
    assert_eq!(read_config(&db).await, Some(json!({ "chunkSize": 500 })));
    let finished = build_run(json!({ "status": "success", "finishedAt": NOW })).to_string();
    repo_common::setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "AIFile" SET "processingRun" = ?1 WHERE "id" = 70"#,
            [finished],
        )?;
        Ok(())
    })
    .await;
    assert!(claim(&db, WS, new_run(), ChunkingConfigUpdate::Keep).await);
    assert_eq!(read_config(&db).await, Some(json!({ "chunkSize": 500 })));
    let failed = build_run(json!({ "status": "error", "error": "unknown" })).to_string();
    repo_common::setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "AIFile" SET "processingRun" = ?1 WHERE "id" = 70"#,
            [failed],
        )?;
        Ok(())
    })
    .await;
    assert!(claim(&db, WS, new_run(), ChunkingConfigUpdate::Reset).await);
    assert_eq!(read_config(&db).await, None);
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:410 "AiFileEmbeddingRepository processing runs against Postgres > a claim refused by an active run leaves the parameters untouched"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_a_claim_refused_by_an_active_run_leaves_the_parameters_untouched()
 {
    let db = fixture(Some(build_run(json!({ "queuedAt": FRESH })))).await;
    assert!(
        !claim(
            &db,
            WS,
            build_run(json!({ "id": "run-new" })),
            ChunkingConfigUpdate::Set(json!({ "chunkSize": 500 }))
        )
        .await
    );
    assert_eq!(read_config(&db).await, None);
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:439 "AiFileEmbeddingRepository processing runs against Postgres > reconcile numbers kept rows without touching status or other positions"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_reconcile_numbers_kept_rows_without_touching_status_or_other_positions()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (920000000000000001, None, "920000000000000001", "success"),
                (920000000000000002, Some(7), "920000000000000002", "success"),
            ],
        );
        Ok(())
    })
    .await;
    reconcile(
        &db,
        vec![(920000000000000001, "a", 0), (920000000000000002, "b", 1)],
    )
    .await;
    assert_eq!(
        chunk_rows(&db).await,
        vec![
            (920000000000000001, Some(0), "success".to_owned()),
            (920000000000000002, Some(7), "success".to_owned()),
        ]
    );
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:476 "AiFileEmbeddingRepository processing runs against Postgres > reconcile stores the position of new chunks"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_reconcile_stores_the_position_of_new_chunks()
 {
    let db = fixture(None).await;
    reconcile(
        &db,
        vec![(930000000000000001, "a", 0), (930000000000000002, "b", 1)],
    )
    .await;
    let mut positions: Vec<Option<i64>> = chunk_rows(&db).await.into_iter().map(|r| r.1).collect();
    positions.sort();
    assert_eq!(positions, vec![Some(0), Some(1)]);
}

async fn page(
    db: &Db,
    workspace: i64,
    cursor: Option<AiFileChunkCursor>,
    from: Option<i64>,
    limit: i64,
) -> Vec<i64> {
    ok(repo::read(db, move |c| {
        afe::list_chunks(c, workspace, FILE, limit, cursor, from)
    })
    .await)
    .into_iter()
    .map(|r| r.id)
    .collect()
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:494 "AiFileEmbeddingRepository processing runs against Postgres > listChunks follows document order, unnumbered rows last, across pages"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_list_chunks_follows_document_order_unnumbered_rows_last_across_pages()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (940000000000000005, Some(2), "5", "success"),
                (940000000000000001, Some(0), "1", "success"),
                (940000000000000009, None, "9", "success"),
                (940000000000000003, Some(1), "3", "success"),
                (940000000000000007, None, "7", "success"),
            ],
        );
        Ok(())
    })
    .await;
    assert_eq!(
        page(&db, WS, None, None, 2).await,
        vec![940000000000000001, 940000000000000003, 940000000000000005]
    );
    assert_eq!(
        page(
            &db,
            WS,
            Some(AiFileChunkCursor {
                chunk_index: Some(1),
                id: 940000000000000003
            }),
            None,
            2
        )
        .await,
        vec![940000000000000005, 940000000000000007, 940000000000000009]
    );
    assert_eq!(
        page(
            &db,
            WS,
            Some(AiFileChunkCursor {
                chunk_index: None,
                id: 940000000000000007
            }),
            None,
            2
        )
        .await,
        vec![940000000000000009]
    );
    assert_eq!(
        page(&db, WS, None, Some(1), 2).await,
        vec![940000000000000003, 940000000000000005, 940000000000000007]
    );
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:547 "AiFileEmbeddingRepository processing runs against Postgres > listChunks never returns another workspace's file"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_list_chunks_never_returns_another_workspace_s_file()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(tx, &[(950000000000000001, Some(0), "x", "success")]);
        Ok(())
    })
    .await;
    assert!(page(&db, 999_999_999_999, None, None, 10).await.is_empty());
}

async fn find(db: &Db, workspace: i64, files: Vec<i64>, pattern: &str) -> Vec<(i64, Option<i64>)> {
    let pattern = pattern.to_owned();
    ok(repo::read(db, move |c| {
        afe::find_chunks_containing(c, workspace, &files, &pattern, 20)
    })
    .await)
    .into_iter()
    .map(|r| (r.id, r.chunk_index))
    .collect()
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:561 "AiFileEmbeddingRepository processing runs against Postgres > findChunksContaining matches literally and ignores case"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_find_chunks_containing_matches_literally_and_ignores_case()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (960000000000000001, Some(0), "Sale 50% OFF today", "success"),
                (960000000000000002, Some(1), "Sale 500 off today", "success"),
                (960000000000000003, Some(2), "code a_b", "success"),
                (960000000000000004, Some(3), "code axb", "success"),
            ],
        );
        Ok(())
    })
    .await;
    let ids = |v: Vec<(i64, Option<i64>)>| v.into_iter().map(|r| r.0).collect::<Vec<_>>();
    assert_eq!(
        ids(find(&db, WS, vec![FILE], "%50\\% off%").await),
        vec![960000000000000001]
    );
    assert_eq!(
        ids(find(&db, WS, vec![FILE], "%a\\_b%").await),
        vec![960000000000000003]
    );
    let positions: Vec<Option<i64>> = find(&db, WS, vec![FILE], "%SALE%")
        .await
        .into_iter()
        .map(|r| r.1)
        .collect();
    assert_eq!(positions, vec![Some(0), Some(1)]);
}

/// Port of: packages/database/__tests__/integration/ai-file-processing-run.test.ts:589 "AiFileEmbeddingRepository processing runs against Postgres > findChunksContaining is scoped to the workspace and files given"
#[tokio::test]
async fn ai_file_embedding_repository_processing_runs_against_postgres_find_chunks_containing_is_scoped_to_the_workspace_and_files_given()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(tx, &[(970000000000000001, Some(0), "secret", "success")]);
        Ok(())
    })
    .await;
    assert!(
        find(&db, 999_999_999_999, vec![FILE], "%secret%")
            .await
            .is_empty()
    );
    assert!(
        find(&db, WS, vec![999_999_999_999], "%secret%")
            .await
            .is_empty()
    );
}
