//! Port of `packages/ai/__tests__/{knowledge-search,knowledge-base-search,
//! embedding-model,provider-errors}.test.ts` (Q.D6).
//! Adapted: the mocked services, repository and `embed` become a real
//! migrated database with stored vectors and a fake [`Embedder`]; a chunk
//! vector `[s, sqrt(1 - s²), 0, ...]` against the query `[1, 0, ...]` has
//! cosine similarity `s`, so the mocked similarities and ranks are real rows.
//! Not applicable: "keeps @chatbotx.io/business free of an @chatbotx.io/ai
//! dependency" (`packages/ai` is the `chatbotx_business::ai` module; there is
//! no package boundary to keep).

mod common;
mod x1_support;

use std::sync::Mutex;

use chatbotx_business::ai::knowledge::{
    self as search, EMBEDDING_DIMENSIONS, EmbedFuture, Embedder, FileSearchConfig,
    ProviderCallError, ResolvedEmbeddingModel, SearchAiFilesInput, is_invalid_api_key_error,
    rank_matches,
};
use chatbotx_business::tx;
use chatbotx_db::Db;
use chatbotx_db::repo::ai_file_embedding::AiFileVectorSearchRow;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingLog;

const FILE: i64 = 9201;
const AGENT: i64 = 9301;
const OTHER_AGENT: i64 = 9302;

/// Answers every query with `[1, 0, ...]` and records the queries.
#[derive(Default)]
struct FakeEmbedder {
    queries: Mutex<Vec<String>>,
}

impl Embedder for FakeEmbedder {
    fn embed<'a>(&'a self, _model: &'a ResolvedEmbeddingModel, value: &'a str) -> EmbedFuture<'a> {
        self.queries.lock().unwrap().push(value.to_string());
        Box::pin(async move {
            let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
            vector[0] = 1.0;
            Ok(vector)
        })
    }
}

impl FakeEmbedder {
    fn queries(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

/// The stored vector with cosine similarity `s` to the query.
fn blob(s: f32) -> Vec<u8> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[0] = s;
    vector[1] = (1.0 - s * s).max(0.0).sqrt();
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// One chunk: `(id, similarity, chunkIndex, content, status)`; a `None`
/// similarity stores no vector.
type Chunk<'a> = (i64, Option<f32>, Option<i64>, &'a str, &'a str);
/// A [`Chunk`] ready to insert (vector bytes, owned text).
type ChunkRow = (i64, Option<Vec<u8>>, Option<i64>, String, String);

async fn chunks(db: &Db, file: i64, rows: &[Chunk<'_>]) {
    let rows: Vec<ChunkRow> = rows
        .iter()
        .map(|(id, s, index, content, status)| {
            (
                *id,
                s.map(blob),
                *index,
                (*content).to_string(),
                (*status).to_string(),
            )
        })
        .collect();
    setup(db, move |tx| {
        for (id, embedding, index, content, status) in &rows {
            tx.execute(
                r#"INSERT INTO "AIEmbedding" ("id", "content", "embedding", "status", "chunkIndex", "workspaceId", "aiFileId")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![id, content, embedding, status, index, WS, file],
            )?;
        }
        Ok(())
    })
    .await;
}

/// `WS`: OpenAI connected, file `FILE` (`guide.pdf`), agent `AGENT` with
/// `tools`; `OTHER_WS`: agent `OTHER_AGENT`.
async fn fixture(tools: Value) -> common::TestDb {
    let db = two_workspaces().await;
    let tools = tools.to_string();
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (80, ?1, 'openai')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationOpenai" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
               VALUES (81, '{"authType":"secretText","secretText":"sk-test"}', 'gpt-4o', 1000, ?1, 80)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId")
               VALUES (?1, 'guide.pdf', 'workspaces/2001/ai-files/guide.pdf', 10, 'application/pdf', ?2)"#,
            params![FILE, WS],
        )?;
        for (id, ws) in [(AGENT, WS), (OTHER_AGENT, OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "AIAgent" ("id", "workspaceId", "name", "messages", "models", "temperature", "maxOutputTokens", "tools")
                   VALUES (?1, ?2, 'Agent', '[]', '[]', 0.5, 100, ?3)"#,
                params![id, ws, tools],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

/// The default fixture: two ready chunks, the first at 0.8 (index 4).
async fn ready(tools: Value) -> common::TestDb {
    let db = fixture(tools).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.8), Some(4), "The Pro plan is 499.000", "success"),
            (2, Some(0.1), Some(5), "Unrelated", "success"),
        ],
    )
    .await;
    db
}

fn files(ids: &[&str]) -> SearchAiFilesInput {
    SearchAiFilesInput {
        workspace_id: WS,
        query: "q".into(),
        file_ids: Some(ids.iter().map(|s| (*s).to_string()).collect()),
        ..SearchAiFilesInput::default()
    }
}

fn agent(id: i64) -> SearchAiFilesInput {
    SearchAiFilesInput {
        workspace_id: WS,
        query: "q".into(),
        agent_id: Some(id),
        ..SearchAiFilesInput::default()
    }
}

async fn run(db: &Db, embedder: &FakeEmbedder, input: SearchAiFilesInput) -> Value {
    search::search_ai_files(db, embedder, input)
        .await
        .expect("search")
}

fn column(result: &Value, name: &str) -> Vec<Value> {
    result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[name].clone())
        .collect()
}

fn file_tool() -> Value {
    json!([format!("file:{FILE}")])
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:68 "searchAIFiles > searches only workspace-owned file ids and exposes readiness"
#[tokio::test]
async fn search_ai_files_searches_only_workspace_owned_file_ids_and_exposes_readiness() {
    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let result = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            query: "Pro price".into(),
            ..files(&["9201"])
        },
    )
    .await;

    assert_eq!(embedder.queries(), vec!["Pro price"]);
    assert_eq!(result["results"][0]["fileId"], json!("9201"));
    assert_eq!(result["results"][0]["usedByAgent"], Value::Null);
    assert_eq!(
        result["files"],
        json!([{ "id": "9201", "name": "guide.pdf", "chunksCount": 2, "readyChunks": 2, "processingStatus": "success" }])
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:96 "searchAIFiles > uses the agent file set and keeps agent-use threshold independent"
#[tokio::test]
async fn search_ai_files_uses_the_agent_file_set_and_keeps_agent_use_threshold_independent() {
    let db = ready(file_tool()).await;
    let embedder = FakeEmbedder::default();
    let result = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            similarity_threshold: Some(0.1),
            ..agent(AGENT)
        },
    )
    .await;

    assert_eq!(result["mode"], json!("agent"));
    assert_eq!(result["results"][0]["passedThreshold"], json!(true));
    assert_eq!(result["results"][0]["usedByAgent"], json!(true));
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:114 "searchAIFiles > reports an agent without files without embedding a query"
#[tokio::test]
async fn search_ai_files_reports_an_agent_without_files_without_embedding_a_query() {
    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let result = run(&db, &embedder, agent(AGENT)).await;

    assert_eq!(result["diagnosis"], json!("agentHasNoFiles"));
    assert_eq!(result["results"], json!([]));
    assert!(embedder.queries().is_empty());
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:154 "searchAIFiles > rejects an agent from another workspace"
#[tokio::test]
async fn search_ai_files_rejects_an_agent_from_another_workspace() {
    let db = ready(file_tool()).await;
    let embedder = FakeEmbedder::default();
    let error = search::search_ai_files(&db, &embedder, agent(OTHER_AGENT))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some("notFound"));
    assert!(embedder.queries().is_empty());
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:163 "searchAIFiles > rejects a file id outside the workspace"
#[tokio::test]
async fn search_ai_files_rejects_a_file_id_outside_the_workspace() {
    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let error = search::search_ai_files(&db, &embedder, files(&["9201", "99"]))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some("notFound"));
    assert!(embedder.queries().is_empty());
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:170 "searchAIFiles > accepts a repeated file id"
#[tokio::test]
async fn search_ai_files_accepts_a_repeated_file_id() {
    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let result = run(&db, &embedder, files(&["9201", "9201"])).await;

    // Adapted: the deduplicated lookup shows as one searched file.
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:179 "searchAIFiles > requires exactly one of agentId or fileIds"
#[tokio::test]
async fn search_ai_files_requires_exactly_one_of_agent_id_or_file_ids() {
    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let neither = SearchAiFilesInput {
        workspace_id: WS,
        query: "q".into(),
        ..SearchAiFilesInput::default()
    };
    let both = SearchAiFilesInput {
        agent_id: Some(AGENT),
        ..files(&["9201"])
    };
    for input in [neither, both] {
        let error = search::search_ai_files(&db, &embedder, input)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Some("validation"));
    }
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:193 "searchAIFiles > ignores agent file tools whose file was deleted, like the runtime toolset"
#[tokio::test]
async fn search_ai_files_ignores_agent_file_tools_whose_file_was_deleted_like_the_runtime_toolset()
{
    let db = ready(json!([format!("file:{FILE}"), "file:404"])).await;
    let embedder = FakeEmbedder::default();
    let result = run(&db, &embedder, agent(AGENT)).await;

    let ids: Vec<&str> = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["9201"]);
    assert_eq!(embedder.queries().len(), 1);
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:209 "searchAIFiles > only marks the agent's top five above the default threshold as used"
#[tokio::test]
async fn search_ai_files_only_marks_the_agents_top_five_above_the_default_threshold_as_used() {
    let db = fixture(file_tool()).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), Some(0), "r1", "success"),
            (2, Some(0.89), Some(1), "r2", "success"),
            (3, Some(0.88), Some(2), "r3", "success"),
            (4, Some(0.87), Some(3), "r4", "success"),
            (5, Some(0.85), Some(4), "r5", "success"),
            (6, Some(0.84), Some(5), "r6", "success"),
            (7, Some(0.6), Some(6), "low", "success"),
        ],
    )
    .await;
    let embedder = FakeEmbedder::default();
    let trial = SearchAiFilesInput {
        similarity_threshold: Some(0.1),
        ..agent(AGENT)
    };
    let small = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            limit: Some(1),
            ..trial.clone()
        },
    )
    .await;
    // Agent mode never searches fewer rows than the agent itself would read.
    assert_eq!(small["appliedConfig"]["limit"], json!(5));
    assert_eq!(small["appliedConfig"]["similarityThreshold"], json!(0.1));

    // Adapted: a real search returns rank 6 and 7 only with a larger limit.
    let wide = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            limit: Some(10),
            ..trial
        },
    )
    .await;
    let used: Vec<(Value, Value)> = column(&wide, "content")
        .into_iter()
        .zip(column(&wide, "usedByAgent"))
        .collect();
    assert_eq!(
        used,
        vec![
            (json!("r1"), json!(true)),
            (json!("r2"), json!(true)),
            (json!("r3"), json!(true)),
            (json!("r4"), json!(true)),
            (json!("r5"), json!(true)),
            (json!("r6"), json!(false)),
            (json!("low"), json!(false)),
        ]
    );
    assert_eq!(column(&wide, "passedThreshold")[6], json!(true));
}

async fn diagnosis(rows: &[Chunk<'_>]) -> Value {
    let db = fixture(json!([])).await;
    chunks(&db, FILE, rows).await;
    let result = run(&db, &FakeEmbedder::default(), files(&["9201"])).await;
    result["diagnosis"].clone()
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:241 "searchAIFiles > diagnoses %s"
#[tokio::test]
async fn search_ai_files_diagnoses_s() {
    assert_eq!(
        diagnosis(&[(1, Some(0.9), Some(0), "a", "success")]).await,
        json!("ok")
    );
    assert_eq!(
        diagnosis(&[(1, Some(0.2), Some(0), "a", "success")]).await,
        json!("allBelowThreshold")
    );
    // Adapted: a ready chunk without a stored vector is counted but never matched.
    assert_eq!(
        diagnosis(&[(1, None, Some(0), "a", "success")]).await,
        json!("noMatches")
    );
    assert_eq!(
        diagnosis(&[
            (1, None, Some(0), "a", "pending"),
            (2, None, Some(1), "b", "pending"),
            (3, None, Some(2), "c", "pending"),
        ])
        .await,
        json!("noReadyChunks")
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:259 "searchAIFiles > agentVisible ignores the trial threshold and mirrors usedByAgent in agent mode"
#[tokio::test]
async fn search_ai_files_agent_visible_ignores_the_trial_threshold_and_mirrors_used_by_agent_in_agent_mode()
 {
    let db = fixture(file_tool()).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), Some(0), "high", "success"),
            (2, Some(0.6), Some(1), "low", "success"),
        ],
    )
    .await;
    let embedder = FakeEmbedder::default();
    let in_agent = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            similarity_threshold: Some(0.0),
            ..agent(AGENT)
        },
    )
    .await;
    let in_files = run(
        &db,
        &embedder,
        SearchAiFilesInput {
            similarity_threshold: Some(0.0),
            ..files(&["9201"])
        },
    )
    .await;

    // The trial threshold lets "low" pass, but the agent's own 0.7 still hides it.
    assert_eq!(
        column(&in_agent, "agentVisible"),
        vec![json!(true), json!(false)]
    );
    assert_eq!(
        column(&in_agent, "usedByAgent"),
        vec![json!(true), json!(false)]
    );
    assert_eq!(
        column(&in_files, "agentVisible"),
        vec![json!(true), json!(false)]
    );
    assert_eq!(
        column(&in_files, "usedByAgent"),
        vec![Value::Null, Value::Null]
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:283 "searchAIFiles > returns the chunk position of every result"
#[tokio::test]
async fn search_ai_files_returns_the_chunk_position_of_every_result() {
    let db = ready(json!([])).await;
    let result = run(&db, &FakeEmbedder::default(), files(&["9201"])).await;

    assert_eq!(result["results"][0]["chunkIndex"], json!(4));
    assert_eq!(result["expected"], Value::Null);
}

fn expecting(text: &str) -> SearchAiFilesInput {
    SearchAiFilesInput {
        expected_text: Some(text.into()),
        ..files(&["9201"])
    }
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:303 "searchAIFiles > expectedText > inTopResults when a chunk with the text is visible to the agent"
#[tokio::test]
async fn search_ai_files_expected_text_in_top_results_when_a_chunk_with_the_text_is_visible_to_the_agent()
 {
    let db = fixture(json!([])).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), Some(0), "r1", "success"),
            (2, Some(0.85), Some(1), "Pro 499.000", "success"),
        ],
    )
    .await;
    let result = run(&db, &FakeEmbedder::default(), expecting("499.000")).await;

    assert_eq!(
        result["expected"],
        json!({
            "verdict": "inTopResults",
            "matches": [{ "chunkId": "2", "fileId": "9201", "fileName": "guide.pdf", "chunkIndex": 1, "rank": 2 }],
        })
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:325 "searchAIFiles > expectedText > foundButLowRank when the chunk is below the default threshold, even if a trial threshold passes it"
#[tokio::test]
async fn search_ai_files_expected_text_found_but_low_rank_when_the_chunk_is_below_the_default_threshold_even_if_a_trial_threshold_passes_it()
 {
    let db = fixture(json!([])).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), Some(0), "r1", "success"),
            (2, Some(0.6), Some(7), "needle here", "success"),
        ],
    )
    .await;
    let result = run(
        &db,
        &FakeEmbedder::default(),
        SearchAiFilesInput {
            similarity_threshold: Some(0.0),
            ..expecting("needle")
        },
    )
    .await;

    assert_eq!(result["expected"]["verdict"], json!("foundButLowRank"));
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:340 "searchAIFiles > expectedText > foundButLowRank when the chunk was not returned at all"
#[tokio::test]
async fn search_ai_files_expected_text_found_but_low_rank_when_the_chunk_was_not_returned_at_all() {
    let db = fixture(json!([])).await;
    // Adapted: a chunk not yet embedded is found by the text lookup only.
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), Some(0), "r1", "success"),
            (2, None, Some(30), "needle elsewhere", "pending"),
        ],
    )
    .await;
    let result = run(&db, &FakeEmbedder::default(), expecting("needle")).await;

    assert_eq!(
        result["expected"],
        json!({
            "verdict": "foundButLowRank",
            "matches": [{ "chunkId": "2", "fileId": "9201", "fileName": "guide.pdf", "chunkIndex": 30, "rank": null }],
        })
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:357 "searchAIFiles > expectedText > counts a returned chunk that holds the text even when the capped lookup missed it"
#[tokio::test]
async fn search_ai_files_expected_text_counts_a_returned_chunk_that_holds_the_text_even_when_the_capped_lookup_missed_it()
 {
    let db = fixture(json!([])).await;
    // Adapted: 20 unembedded chunks fill the lookup's cap (20, position
    // order); the visible chunk sits after them.
    let far: Vec<(i64, String)> = (0..20)
        .map(|i| (100 + i, format!("far 499.000 {i}")))
        .collect();
    let mut rows: Vec<Chunk<'_>> = far
        .iter()
        .map(|(id, content)| (*id, None, Some(*id - 100), content.as_str(), "pending"))
        .collect();
    rows.push((1, Some(0.9), Some(40), "Pro plan 499.000", "success"));
    rows.push((2, Some(0.85), Some(41), "r2", "success"));
    chunks(&db, FILE, &rows).await;
    let result = run(&db, &FakeEmbedder::default(), expecting("499.000")).await;

    assert_eq!(result["expected"]["verdict"], json!("inTopResults"));
    let matches = result["expected"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 21);
    assert_eq!(matches[0]["chunkId"], json!("100"));
    assert_eq!(matches[20]["chunkId"], json!("1"));
    assert_eq!(matches[20]["rank"], json!(1));
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:381 "searchAIFiles > expectedText > notInAnyChunk when no chunk holds the text"
#[tokio::test]
async fn search_ai_files_expected_text_not_in_any_chunk_when_no_chunk_holds_the_text() {
    let db = ready(json!([])).await;
    let result = run(&db, &FakeEmbedder::default(), expecting("missing")).await;

    assert_eq!(
        result["expected"],
        json!({ "verdict": "notInAnyChunk", "matches": [] })
    );
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:394 "searchAIFiles > expectedText > searches at least the agent's top five so every visible chunk is returned"
#[tokio::test]
async fn search_ai_files_expected_text_searches_at_least_the_agents_top_five_so_every_visible_chunk_is_returned()
 {
    let db = ready(json!([])).await;
    let result = run(
        &db,
        &FakeEmbedder::default(),
        SearchAiFilesInput {
            limit: Some(1),
            ..expecting("x")
        },
    )
    .await;

    assert_eq!(result["appliedConfig"]["limit"], json!(5));
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:409 "searchAIFiles > expectedText > never logs the expected text or the query"
#[tokio::test]
async fn search_ai_files_expected_text_never_logs_the_expected_text_or_the_query() {
    let db = ready(json!([])).await;
    let log = RecordingLog::install();
    run(
        &db,
        &FakeEmbedder::default(),
        SearchAiFilesInput {
            query: "secret question".into(),
            ..expecting("secret answer")
        },
    )
    .await;

    let entries = log.entries();
    assert!(log.find("AI file search completed").is_some());
    assert!(!format!("{entries:?}").contains("secret"));
}

/// Port of: packages/ai/__tests__/knowledge-search.test.ts:423 "searchAIFiles > surfaces a missing embedding provider before embedding"
#[tokio::test]
async fn search_ai_files_surfaces_a_missing_embedding_provider_before_embedding() {
    let db = ready(json!([])).await;
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Integration""#, [])?;
        Ok(())
    })
    .await;
    let embedder = FakeEmbedder::default();
    let error = search::search_ai_files(&db, &embedder, files(&["9201"]))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some("noEmbeddingProvider"));
    assert!(embedder.queries().is_empty());
}

fn row(id: i64, similarity: Option<f64>) -> AiFileVectorSearchRow {
    AiFileVectorSearchRow {
        id,
        content: format!("content {id}"),
        ai_file_id: 1,
        chunk_index: None,
        similarity,
    }
}

fn config(ids: &[&str], max_results: i64) -> FileSearchConfig {
    FileSearchConfig {
        workspace_id: WS,
        selected_file_ids: ids.iter().map(|s| (*s).to_string()).collect(),
        similarity_threshold: None,
        max_results,
    }
}

/// Port of: packages/ai/__tests__/knowledge-base-search.test.ts:51 "searchKnowledgeChunks > keeps below-threshold chunks and ranks them, with > not >= at the threshold"
#[tokio::test]
async fn search_knowledge_chunks_keeps_below_threshold_chunks_and_ranks_them_with_not_at_the_threshold()
 {
    // Adapted: the ranking runs on the rows directly (an exact 0.7 cannot be
    // stored as a vector); the applied config comes from a real search.
    let ranked = rank_matches(
        WS,
        vec![row(1, Some(0.9)), row(2, Some(0.7)), row(3, Some(0.2))],
        0.7,
    );
    assert_eq!(
        ranked
            .iter()
            .map(|m| (m.id, m.rank, m.passed_threshold))
            .collect::<Vec<_>>(),
        vec![(1, 1, true), (2, 2, false), (3, 3, false)]
    );

    let db = ready(json!([])).await;
    let embedder = FakeEmbedder::default();
    let result =
        search::search_knowledge_chunks(&db, &embedder, "price", config(&["9201", "2"], 5))
            .await
            .unwrap();
    assert_eq!(embedder.queries(), vec!["price"]);
    let applied = &result.applied_config;
    assert_eq!(
        (
            applied.provider,
            applied.model_id,
            applied.similarity_threshold,
            applied.limit
        ),
        ("openai", "text-embedding-ada-002", 0.7, 5)
    );
    assert_eq!(result.matches.len(), 2);
}

/// Port of: packages/ai/__tests__/knowledge-base-search.test.ts:85 "searchKnowledgeChunks > drops non-numeric file ids before querying"
#[tokio::test]
async fn search_knowledge_chunks_drops_non_numeric_file_ids_before_querying() {
    let db = ready(json!([])).await;
    let result = search::search_knowledge_chunks(
        &db,
        &FakeEmbedder::default(),
        "price",
        config(&["abc"], 5),
    )
    .await
    .unwrap();

    assert!(result.matches.is_empty());
    assert_eq!(result.timings_ms.1, 0);
}

/// Port of: packages/ai/__tests__/knowledge-base-search.test.ts:97 "searchKnowledgeChunks > returns no matches and warns when rows are malformed"
#[tokio::test]
async fn search_knowledge_chunks_returns_no_matches_and_warns_when_rows_are_malformed() {
    let log = RecordingLog::install();
    let ranked = rank_matches(WS, vec![row(1, Some(0.9)), row(2, Some(f64::NAN))], 0.7);

    assert!(ranked.is_empty());
    let warning = log
        .find("[ai-package] Invalid similarity search results")
        .expect("warning");
    assert_eq!(warning.field("workspace_id"), Some("2001"));
}

/// Port of: packages/ai/__tests__/knowledge-base-search.test.ts:111 "performFileSearch > returns only above-threshold chunks in the legacy shape"
#[tokio::test]
async fn perform_file_search_returns_only_above_threshold_chunks_in_the_legacy_shape() {
    let db = fixture(json!([])).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.9), None, "content a", "success"),
            (2, Some(0.5), None, "content b", "success"),
        ],
    )
    .await;
    let results =
        search::perform_file_search(&db, &FakeEmbedder::default(), "price", config(&["9201"], 5))
            .await
            .unwrap();

    assert_eq!(results.len(), 1);
    let first = &results[0];
    assert_eq!(
        (
            first.id,
            first.content.as_str(),
            first.ai_file_id,
            first.chunk_index
        ),
        (1, "content a", FILE, None)
    );
    assert!((first.similarity - 0.9).abs() < 1e-5);
}

/// Port of: packages/ai/__tests__/knowledge-base-search.test.ts:127 "performFileSearch > uses the configured threshold and limit unchanged"
#[tokio::test]
async fn perform_file_search_uses_the_configured_threshold_and_limit_unchanged() {
    let db = fixture(json!([])).await;
    chunks(
        &db,
        FILE,
        &[
            (1, Some(0.5), Some(0), "a", "success"),
            (2, Some(0.3), Some(1), "b", "success"),
            (3, Some(0.2), Some(2), "c", "success"),
            (4, Some(0.1), Some(3), "d", "success"),
        ],
    )
    .await;
    let embedder = FakeEmbedder::default();
    let config = FileSearchConfig {
        similarity_threshold: Some(0.4),
        ..config(&["9201"], 3)
    };
    let results = search::perform_file_search(&db, &embedder, "price", config.clone())
        .await
        .unwrap();
    assert_eq!(results.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);

    // The limit reaches the vector search unchanged.
    let all = search::search_knowledge_chunks(&db, &embedder, "price", config)
        .await
        .unwrap();
    assert_eq!(all.matches.len(), 3);
}

/// Port of: packages/ai/__tests__/embedding-model.test.ts:39 "resolveEmbeddingModel > resolves Gemini when it is the only configured provider"
#[tokio::test]
async fn resolve_embedding_model_resolves_gemini_when_it_is_the_only_configured_provider() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (82, ?1, 'gemini')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationGemini" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
               VALUES (83, '{"authType":"secretText","secretText":"gemini-key"}', 'gemini-3.5-flash', 1000, ?1, 82)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let model = tx::read(&db, |conn| search::resolve_embedding_model(conn, WS))
        .await
        .unwrap();

    assert_eq!(
        model,
        ResolvedEmbeddingModel {
            provider: "gemini",
            model_id: "gemini-embedding-001",
            api_key: "gemini-key".into(),
        }
    );
    // The Gemini request asks for the stored vector size.
    let (url, _, body) = search::embed_request(&model, "q");
    assert!(url.ends_with("/models/gemini-embedding-001:embedContent"));
    assert_eq!(body["outputDimensionality"], json!(1536));
}

fn api_call_error(status: u16, body: Option<&str>) -> ProviderCallError {
    ProviderCallError {
        status: Some(status),
        body: body.map(str::to_string),
        message: "provider rejected the request".into(),
    }
}

/// Port of: packages/ai/__tests__/provider-errors.test.ts:15 "isInvalidApiKeyError > is true for a %i from the provider"
#[test]
fn is_invalid_api_key_error_is_true_for_a_i_from_the_provider() {
    for status in [401, 403] {
        assert!(is_invalid_api_key_error(Some(&api_call_error(
            status, None
        ))));
    }
}

/// Port of: packages/ai/__tests__/provider-errors.test.ts:19 "isInvalidApiKeyError > is true for Gemini's 400 that names an invalid key"
#[test]
fn is_invalid_api_key_error_is_true_for_geminis_400_that_names_an_invalid_key() {
    let body = json!({
        "error": { "code": 400, "status": "INVALID_ARGUMENT", "details": [{ "reason": "API_KEY_INVALID" }] },
    })
    .to_string();

    assert!(is_invalid_api_key_error(Some(&api_call_error(
        400,
        Some(&body)
    ))));
}

/// Port of: packages/ai/__tests__/provider-errors.test.ts:31 "isInvalidApiKeyError > is false for a 400 about anything else"
#[test]
fn is_invalid_api_key_error_is_false_for_a_400_about_anything_else() {
    assert!(!is_invalid_api_key_error(Some(&api_call_error(
        400,
        Some(r#"{"error":"bad input"}"#)
    ))));
}

/// Port of: packages/ai/__tests__/provider-errors.test.ts:37 "isInvalidApiKeyError > is false for a retryable %i"
#[test]
fn is_invalid_api_key_error_is_false_for_a_retryable_i() {
    for status in [429, 500, 503] {
        let error = api_call_error(status, None);
        assert!(!is_invalid_api_key_error(Some(&error)));
        assert!(error.is_retryable());
    }
}

/// Port of: packages/ai/__tests__/provider-errors.test.ts:41 "isInvalidApiKeyError > is false for errors that are not provider API calls"
#[test]
fn is_invalid_api_key_error_is_false_for_errors_that_are_not_provider_api_calls() {
    // Adapted: a non-API error is not a `ProviderCallError` (`None`).
    assert!(!is_invalid_api_key_error(None));
}
