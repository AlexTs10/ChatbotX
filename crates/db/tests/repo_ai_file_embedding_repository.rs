//! Port of `packages/database/__tests__/ai-file-embedding-repository.test.ts` (P3.2).
//!
//! The original mocks the Drizzle client and asserts the rendered SQL / bound params.
//! Each case is ported as the observable effect on real rows of a migrated SQLite file
//! (adapted): a workspace / status guard = a decoy row outside the guard is untouched;
//! "does not query" = an empty result for an empty id list; `limit + 1` = one extra row.

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
    FILE, WS, build_run, chunk_rows, fixture, insert_chunks, read_config, read_run,
};
use repo_common::{ok, scalar_text};
use serde_json::json;

fn chunking() -> serde_json::Value {
    json!({
        "strategy": "tabular",
        "chunkerVersion": 1,
        "chunkSize": 1000,
        "overlapSize": 0,
        "chunkCount": 3,
        "processedAt": "2026-10-24T00:00:00.000Z",
    })
}

/// A second file (71) of another workspace (2002).
async fn with_other_workspace_file(db: &Db) {
    common::seed_workspace(db, 1002, 2002).await;
    repo_common::setup(db, |tx| {
        ins!(tx, "AIFile", { "id" => 71_i64, "name" => "o", "path" => "o", "size" => 1_i64, "mimeType" => "text/csv", "workspaceId" => 2002_i64 })?;
        Ok(())
    })
    .await;
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:41 "AiFileEmbeddingRepository.markChunked > writes only the chunking column of the file in its own workspace"
#[tokio::test]
async fn ai_file_embedding_repository_mark_chunked_writes_only_the_chunking_column_of_the_file_in_its_own_workspace()
 {
    let db = fixture(None).await;
    ok(repo::write(&db, |tx| afe::mark_chunked(tx, FILE, WS, &chunking())).await);
    let stored = scalar_text(
        &db,
        r#"SELECT "chunking" FROM "AIFile" WHERE "id" = 70"#,
        vec![],
    )
    .await;
    assert_eq!(
        stored.map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap()),
        Some(chunking())
    );
    assert_eq!(read_run(&db).await, None);
    assert_eq!(read_config(&db).await, None);
    // Another workspace: nothing written.
    with_other_workspace_file(&db).await;
    ok(repo::write(&db, |tx| afe::mark_chunked(tx, 71, WS, &chunking())).await);
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "chunking" FROM "AIFile" WHERE "id" = 71"#,
            vec![]
        )
        .await,
        None
    );
}

fn blob(x: f32, y: f32) -> Vec<u8> {
    let mut v = vec![0.0_f32; 1536];
    v[0] = x;
    v[1] = y;
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:59 "AiFileEmbeddingRepository query boundaries > binds vectors and scopes vector search to the workspace and ready chunks"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_binds_vectors_and_scopes_vector_search_to_the_workspace_and_ready_chunks()
 {
    let db = fixture(None).await;
    with_other_workspace_file(&db).await;
    repo_common::setup(&db, |tx| {
        for (id, file, ws, status, emb) in [
            (1_i64, 70_i64, 2001_i64, "success", Some(blob(1.0, 0.0))),
            (2, 70, 2001, "pending", Some(blob(1.0, 0.0))),
            (3, 70, 2001, "success", None),
            (4, 71, 2002, "success", Some(blob(1.0, 0.0))),
            (5, 70, 2001, "success", Some(blob(0.0, 1.0))),
        ] {
            ins!(tx, "AIEmbedding", { "id" => id, "content" => format!("c{id}"), "status" => status, "embedding" => emb, "chunkIndex" => id, "aiFileId" => file, "workspaceId" => ws })?;
        }
        Ok(())
    })
    .await;
    let mut query = vec![0.0_f32; 1536];
    query[0] = 1.0;
    let rows = ok(repo::read(&db, move |c| {
        afe::vector_search(c, WS, &[70, 71], &query, 5)
    })
    .await);
    // Only ready (success, with a vector) chunks of the workspace, nearest first.
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 5]);
    assert!((rows[0].similarity.unwrap() - 1.0).abs() < 1e-6);
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:84 "AiFileEmbeddingRepository query boundaries > does not query when vector search has no file ids"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_does_not_query_when_vector_search_has_no_file_ids()
 {
    let db = fixture(None).await;
    let rows = ok(repo::read(&db, |c| afe::vector_search(c, WS, &[], &[0.1], 5)).await);
    assert!(rows.is_empty());
}

fn queued_run() -> serde_json::Value {
    json!({
        "id": "run-1", "status": "queued", "error": null, "queuedAt": "2026-10-24T01:00:00.000Z",
        "startedAt": null, "heartbeatAt": null, "finishedAt": null,
    })
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:101 "AiFileEmbeddingRepository query boundaries > claims a processing run with a workspace-scoped stale lease condition"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_claims_a_processing_run_with_a_workspace_scoped_stale_lease_condition()
 {
    // A processing run with a heartbeat after the stale bound is not claimable; with an
    // older heartbeat it is (the lease compares ISO text, no timestamptz cast).
    let db = fixture(Some(build_run(json!({
        "status": "processing", "queuedAt": "2026-10-23T00:00:00.000Z", "heartbeatAt": "2026-10-24T00:30:00.000Z",
    }))))
    .await;
    let (ws, stale, run) = (WS, "2026-10-24T00:00:00.000Z", queued_run());
    let r = run.clone();
    assert!(!ok(repo::write(&db, move |tx| afe::claim_processing_run(
        tx,
        FILE,
        ws,
        &r,
        stale,
        &ChunkingConfigUpdate::Keep
    ))
    .await));
    let r = run.clone();
    assert!(!ok(repo::write(&db, move |tx| afe::claim_processing_run(
        tx,
        FILE,
        999,
        &r,
        "2026-10-25T00:00:00.000Z",
        &ChunkingConfigUpdate::Keep
    ))
    .await));
    assert!(ok(repo::write(&db, move |tx| afe::claim_processing_run(
        tx,
        FILE,
        WS,
        &run,
        "2026-10-25T00:00:00.000Z",
        &ChunkingConfigUpdate::Keep
    ))
    .await));
    assert_eq!(read_run(&db).await, Some(queued_run()));
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:152 "AiFileEmbeddingRepository query boundaries > start, lease and finish guard on run id, workspace and status, and merge with ||"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_start_lease_and_finish_guard_on_run_id_workspace_and_status_and_merge_with()
 {
    let db = fixture(Some(build_run(json!({ "id": "run-1", "extra": "kept" })))).await;
    // Wrong workspace / run id: no update.
    assert!(!ok(repo::write(
        &db,
        |tx| afe::mark_processing_run_started(tx, FILE, 999, "run-1", "2026-10-24T00:00:00.000Z")
    )
    .await));
    assert!(!ok(repo::write(
        &db,
        |tx| afe::mark_processing_run_started(tx, FILE, WS, "run-x", "2026-10-24T00:00:00.000Z")
    )
    .await));
    assert!(ok(repo::write(&db, |tx| afe::mark_processing_run_started(
        tx,
        FILE,
        WS,
        "run-1",
        "2026-10-24T00:00:00.000Z"
    ))
    .await));
    assert!(ok(repo::write(
        &db,
        |tx| afe::refresh_processing_run_lease(tx, FILE, WS, "run-1", "2026-10-24T00:01:00.000Z")
    )
    .await));
    assert!(ok(repo::write(&db, |tx| afe::finish_processing_run(
        tx,
        FILE,
        WS,
        "run-1",
        "error",
        Some("unknown"),
        "2026-10-24T00:02:00.000Z"
    ))
    .await));
    let run = read_run(&db).await.expect("run");
    // Merged: keys outside the patch survive.
    assert_eq!(run["extra"], "kept");
    assert_eq!(run["status"], "error");
    assert_eq!(run["error"], "unknown");
    assert_eq!(run["startedAt"], "2026-10-24T00:00:00.000Z");
    assert_eq!(run["heartbeatAt"], "2026-10-24T00:01:00.000Z");
    assert_eq!(run["finishedAt"], "2026-10-24T00:02:00.000Z");
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:197 "AiFileEmbeddingRepository query boundaries > reports whether a run row was updated"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_reports_whether_a_run_row_was_updated() {
    let db = fixture(None).await;
    assert!(!ok(repo::write(&db, |tx| {
        afe::refresh_processing_run_lease(tx, FILE, WS, "run-1", "2026-10-24T00:00:00.000Z")
    })
    .await));
}

async fn list(
    db: &Db,
    cursor: Option<AiFileChunkCursor>,
    from: Option<i64>,
    limit: i64,
) -> Vec<i64> {
    ok(repo::read(db, move |c| {
        afe::list_chunks(c, WS, FILE, limit, cursor, from)
    })
    .await)
    .into_iter()
    .map(|r| r.id)
    .collect()
}

async fn numbered(db: &Db) {
    // ids 100.. with chunkIndex 0..; 200, 201 unnumbered.
    repo_common::setup(db, |tx| {
        for i in 0..6_i64 {
            insert_chunks(tx, &[(100 + i, Some(i), "x", "success")]);
        }
        insert_chunks(
            tx,
            &[(200, None, "x", "success"), (201, None, "x", "success")],
        );
        Ok(())
    })
    .await;
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:239 "AiFileEmbeddingRepository query boundaries > lists chunks in document order, scoped to the file and workspace"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_lists_chunks_in_document_order_scoped_to_the_file_and_workspace()
 {
    let db = fixture(None).await;
    numbered(&db).await;
    // limit 2 -> 3 rows (limit + 1), numbered rows in order.
    assert_eq!(list(&db, None, None, 2).await, vec![100, 101, 102]);
    assert_eq!(
        list(&db, None, None, 50).await,
        vec![100, 101, 102, 103, 104, 105, 200, 201]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:254 "AiFileEmbeddingRepository query boundaries > continues after a numbered row with a keyset over (chunkIndex, id)"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_continues_after_a_numbered_row_with_a_keyset_over_chunk_index_id()
 {
    let db = fixture(None).await;
    numbered(&db).await;
    assert_eq!(
        list(
            &db,
            Some(AiFileChunkCursor {
                chunk_index: Some(4),
                id: 104
            }),
            None,
            50
        )
        .await,
        vec![105, 200, 201]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:272 "AiFileEmbeddingRepository query boundaries > continues inside the unnumbered rows after a null position"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_continues_inside_the_unnumbered_rows_after_a_null_position()
 {
    let db = fixture(None).await;
    numbered(&db).await;
    assert_eq!(
        list(
            &db,
            Some(AiFileChunkCursor {
                chunk_index: None,
                id: 200
            }),
            None,
            50
        )
        .await,
        vec![201]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:287 "AiFileEmbeddingRepository query boundaries > opens the first page at a position and keeps unnumbered rows reachable"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_opens_the_first_page_at_a_position_and_keeps_unnumbered_rows_reachable()
 {
    let db = fixture(None).await;
    numbered(&db).await;
    assert_eq!(list(&db, None, Some(4), 50).await, vec![104, 105, 200, 201]);
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:302 "AiFileEmbeddingRepository query boundaries > ignores fromChunkIndex when a cursor is present"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_ignores_from_chunk_index_when_a_cursor_is_present()
 {
    let db = fixture(None).await;
    numbered(&db).await;
    assert_eq!(
        list(
            &db,
            Some(AiFileChunkCursor {
                chunk_index: Some(1),
                id: 101
            }),
            Some(5),
            50
        )
        .await,
        vec![102, 103, 104, 105, 200, 201]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:316 "AiFileEmbeddingRepository query boundaries > finds chunks containing text with a bound, escaped ILIKE pattern"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_finds_chunks_containing_text_with_a_bound_escaped_ilike_pattern()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (1, Some(0), "Only 50% left", "success"),
                (2, Some(1), "only 500 left", "success"),
            ],
        );
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| {
        afe::find_chunks_containing(c, WS, &[FILE, 2], "%50\\%%", 20)
    })
    .await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
    // The limit is applied.
    let one = ok(repo::read(&db, |c| {
        afe::find_chunks_containing(c, WS, &[FILE], "%only%", 1)
    })
    .await);
    assert_eq!(one.len(), 1);
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:335 "AiFileEmbeddingRepository query boundaries > does not query for chunks containing text without files"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_does_not_query_for_chunks_containing_text_without_files()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(tx, &[(1, Some(0), "x", "success")]);
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| afe::find_chunks_containing(c, WS, &[], "%x%", 20)).await);
    assert!(rows.is_empty());
}

async fn claim_with(config: ChunkingConfigUpdate) -> Option<serde_json::Value> {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "AIFile" SET "chunkingConfig" = '{"chunkSize":800}' WHERE "id" = 70"#,
            [],
        )?;
        Ok(())
    })
    .await;
    assert!(ok(repo::write(&db, move |tx| afe::claim_processing_run(
        tx,
        FILE,
        WS,
        &queued_run(),
        "2026-10-24T00:00:00.000Z",
        &config
    ))
    .await));
    read_config(&db).await
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:356 "AiFileEmbeddingRepository query boundaries > claim %s when chunkingConfig is %j" (leaves the column alone, undefined)
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_claim_leaves_the_column_alone_when_chunking_config_is_undefined()
 {
    assert_eq!(
        claim_with(ChunkingConfigUpdate::Keep).await,
        Some(json!({ "chunkSize": 800 }))
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:356 "AiFileEmbeddingRepository query boundaries > claim %s when chunkingConfig is %j" (resets it to the defaults, null)
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_claim_resets_it_to_the_defaults_when_chunking_config_is_null()
 {
    assert_eq!(claim_with(ChunkingConfigUpdate::Reset).await, None);
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:356 "AiFileEmbeddingRepository query boundaries > claim %s when chunkingConfig is %j" (replaces it, {"chunkSize":500})
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_claim_replaces_it_when_chunking_config_is_chunk_size_500()
 {
    assert_eq!(
        claim_with(ChunkingConfigUpdate::Set(json!({ "chunkSize": 500 }))).await,
        Some(json!({ "chunkSize": 500 }))
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:381 "AiFileEmbeddingRepository query boundaries > reconcile resets only kept error chunks to pending"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_reconcile_resets_only_kept_error_chunks_to_pending()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[
                (1, None, "a", "error"),
                (2, Some(1), "b", "success"),
                (3, None, "z", "error"),
                (4, None, "p", "processing"),
            ],
        );
        Ok(())
    })
    .await;
    let chunks = vec![
        AiFileEmbeddingChunk {
            id: 1,
            content: "a".into(),
            chunk_index: 0,
        },
        AiFileEmbeddingChunk {
            id: 2,
            content: "b".into(),
            chunk_index: 1,
        },
        AiFileEmbeddingChunk {
            id: 4,
            content: "p".into(),
            chunk_index: 3,
        },
        AiFileEmbeddingChunk {
            id: 5,
            content: "n".into(),
            chunk_index: 2,
        },
    ];
    let pending = ok(repo::write(&db, move |tx| {
        afe::reconcile_pending_chunks(tx, FILE, WS, &chunks)
    })
    .await);
    let mut pending = pending;
    pending.sort_unstable();
    assert_eq!(pending, vec![1, 5]);
    assert_eq!(
        chunk_rows(&db).await,
        vec![
            (1, Some(0), "pending".to_owned()),
            (2, Some(1), "success".to_owned()),
            (4, Some(3), "processing".to_owned()),
            (5, Some(2), "pending".to_owned()),
        ]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:455 "AiFileEmbeddingRepository query boundaries > rejects an empty chunk set without touching the existing index"
#[tokio::test]
async fn ai_file_embedding_repository_query_boundaries_rejects_an_empty_chunk_set_without_touching_the_existing_index()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(tx, &[(1, Some(0), "a", "success")]);
        Ok(())
    })
    .await;
    let message = repo_common::err_message(
        repo::write(&db, |tx| afe::reconcile_pending_chunks(tx, FILE, WS, &[])).await,
    );
    assert!(message.contains("at least one chunk"), "{message}");
    assert_eq!(
        chunk_rows(&db).await,
        vec![(1, Some(0), "success".to_owned())]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:491 "AiFileEmbeddingRepository.markEmbeddingFailed > marks only a pending chunk of the workspace as error"
#[tokio::test]
async fn ai_file_embedding_repository_mark_embedding_failed_marks_only_a_pending_chunk_of_the_workspace_as_error()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(
            tx,
            &[(1, Some(0), "a", "pending"), (2, Some(1), "b", "pending")],
        );
        Ok(())
    })
    .await;
    assert!(!ok(repo::write(&db, |tx| afe::mark_embedding_failed(
        tx, 2, 999
    ))
    .await));
    assert!(ok(repo::write(&db, |tx| afe::mark_embedding_failed(
        tx, 1, WS
    ))
    .await));
    assert_eq!(
        chunk_rows(&db).await,
        vec![
            (1, Some(0), "error".to_owned()),
            (2, Some(1), "pending".to_owned())
        ]
    );
}

/// Port of: packages/database/__tests__/ai-file-embedding-repository.test.ts:511 "AiFileEmbeddingRepository.markEmbeddingFailed > reports false when the chunk is no longer pending"
#[tokio::test]
async fn ai_file_embedding_repository_mark_embedding_failed_reports_false_when_the_chunk_is_no_longer_pending()
 {
    let db = fixture(None).await;
    repo_common::setup(&db, |tx| {
        insert_chunks(tx, &[(1, Some(0), "a", "success")]);
        Ok(())
    })
    .await;
    assert!(!ok(repo::write(&db, |tx| afe::mark_embedding_failed(
        tx, 1, WS
    ))
    .await));
}
