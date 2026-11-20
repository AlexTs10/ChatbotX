//! Q.W2 slice 2: AI file ingestion. `processAIFile` (heavy queue:
//! extract, chunk, reconcile the chunks, queue their embeddings) and
//! `processPendingEmbedding` (aiAgent queue: embed one chunk).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use chatbotx_business::ai::knowledge::{
    self, ResolvedEmbeddingModel, is_embedding_provider_not_configured, is_invalid_api_key_error,
};
use chatbotx_business::ai_file as service;
use chatbotx_core::js::to_iso_string;
use chatbotx_db::repo::ai_file_embedding::{self as repo, AiFileEmbeddingChunk};
use chatbotx_db::repo::ai_file_embedding_id::create_ai_file_embedding_id;
use chatbotx_db::repo::models::AiFile;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::ai::{
    AiFileChunkingConfigStrategy, resolve_ai_file_chunking_config,
};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::queues::ai_file::ai_file_embedding_job_id;
use chatbotx_jobs::{JobCtx, JobError, JobOptions, NewJob, QueueName};
use serde_json::{Value, json};

use crate::ai::chunk::{self, Document};
use crate::ai::extract;
use crate::{Deps, db_fail, id_of, repo_err, str_of};

/// `AI_FILE_EMBEDDING_PRIORITY`: below live conversation jobs.
const EMBEDDING_PRIORITY: u32 = 5;

/// The heavy limits (`HEAVY_MAX_*`, worker env).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_text_chars: usize,
    pub max_chunks: usize,
}

impl Limits {
    pub fn from_env() -> Limits {
        let num = |name: &str, default: u64, max: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|v| (1..=max).contains(v))
                .unwrap_or(default)
        };
        Limits {
            max_file_bytes: num("HEAVY_MAX_FILE_BYTES", 50 << 20, 100 << 20),
            max_text_chars: num("HEAVY_MAX_EXTRACTED_TEXT_CHARS", 5_000_000, 10_000_000) as usize,
            max_chunks: num("HEAVY_MAX_CHUNKS_PER_FILE", 5000, 10_000) as usize,
        }
    }
}

/// How a run fails: a run error code (`AIFileProcessingError`, never
/// retried) or a transient error (retried).
#[derive(Debug)]
pub enum FileError {
    Code(&'static str),
    Retry(String),
}

impl From<FileError> for JobError {
    fn from(error: FileError) -> JobError {
        match error {
            FileError::Code(code) => JobError::unrecoverable(code),
            FileError::Retry(message) => JobError::retryable(message),
        }
    }
}

fn retry(error: impl std::fmt::Display) -> FileError {
    FileError::Retry(error.to_string())
}

/// `distributedLock.runExclusive("ai-file:process:<id>")`.
// ponytail: an in-process lock; one process runs the heavy queue. One
// entry per file id stays in the map (a few bytes each).
fn file_lock(ai_file_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(Mutex::default);
    let mut locks = LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(locks.entry(ai_file_id).or_default())
}

/// The heavy queue processor: `processAIFile` and the flow-step jobs
/// (`crate::heavy`).
pub async fn handle_heavy(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("");
    if crate::heavy::action_of(kind).is_some() {
        return crate::heavy::handle_step_job(&deps, &job).await;
    }
    if kind == "extractTextFromFile" {
        return crate::heavy_reader::extract_text_from_file(&deps, &job.data["data"]).await;
    }
    if kind == "optimizeStoredImage" {
        // One info line per job: the outcome and the bytes before/after.
        let started = std::time::Instant::now();
        let data = &job.data["data"];
        let result = crate::stored_image::optimize(&deps, data).await?;
        tracing::info!(
            job_id = %job.job_id,
            kind = data["kind"].as_str(),
            workspace_id = data["workspaceId"].as_str(),
            outcome = result["outcome"].as_str(),
            bytes_before = result["bytesBefore"].as_u64(),
            bytes_after = result["bytesAfter"].as_u64(),
            duration_ms = started.elapsed().as_millis() as u64,
            "Stored image optimization finished"
        );
        return Ok(Value::Null);
    }
    if kind != "processAIFile" {
        // ponytail: `analyzeImage` needs a vision reply model (no image part
        // in `crate::ai` yet); speech to text is a flow-step job.
        return Err(JobError::unrecoverable(format!(
            "Heavy job action is not available: {kind}"
        )));
    }
    let data = &job.data["data"];
    let Some(ai_file_id) = data.get("aiFileId").and_then(id_of) else {
        return Err(JobError::unrecoverable("Invalid processAIFile data"));
    };
    let run_id = str_of(data, "runId").map(str::to_owned);
    let result = process_ai_file(&deps, &Limits::from_env(), ai_file_id, run_id.clone()).await;
    let Err(error) = result else {
        return Ok(Value::Null);
    };
    tracing::error!(err = ?error, job_id = %job.job_id, "Heavy job failed");
    // The `failed` hook: a code fails at once, a transient error on its
    // last attempt.
    let code = match &error {
        FileError::Code(code) => Some(*code),
        FileError::Retry(_) if job.is_final_attempt() => Some("unknown"),
        FileError::Retry(_) => None,
    };
    if let (Some(code), Some(run_id)) = (code, run_id) {
        let ws = deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "workspaceId" FROM "AIFile" WHERE "id" = ?1"#,
                        [ai_file_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?)
            })
            .await
            .ok()
            .flatten();
        match ws {
            Some(ws) => {
                if let Err(err) =
                    service::mark_processing_run_failed(&deps.db, ai_file_id, ws, run_id, code)
                        .await
                {
                    tracing::error!(err = %err, ai_file_id, "Failed to mark AI file processing run failed");
                }
            }
            None => tracing::error!(
                ai_file_id,
                "Cannot mark AI file processing run failed without a workspace"
            ),
        }
    }
    Err(error.into())
}

/// `processAIFile`.
pub async fn process_ai_file(
    deps: &Deps,
    limits: &Limits,
    ai_file_id: i64,
    run_id: Option<String>,
) -> Result<(), FileError> {
    let lock = file_lock(ai_file_id);
    let _held = lock.lock().await;
    let file: AiFile = deps
        .db
        .read(move |conn| repo::find_file_or_fail(conn, ai_file_id).map_err(repo_err))
        .await
        .map_err(retry)?;
    let ws = file.workspace_id;
    if let Some(run) = &run_id
        && !service::mark_processing_run_started(&deps.db, ai_file_id, ws, run.clone())
            .await
            .map_err(retry)?
    {
        tracing::info!(ai_file_id, run_id = %run, "AI file processing run superseded, skipping");
        return Ok(());
    }
    let lease = |stage: &'static str| {
        let run_id = run_id.clone();
        async move {
            let Some(run) = run_id else {
                return Ok(true);
            };
            let ok = service::refresh_processing_run_lease(&deps.db, ai_file_id, ws, run.clone())
                .await
                .map_err(retry)?;
            if !ok {
                tracing::info!(ai_file_id, run_id = %run, stage, "AI file processing run superseded, skipping");
            }
            Ok::<bool, FileError>(ok)
        }
    };

    let config = resolve_ai_file_chunking_config(file.chunking_config.as_ref().map(|j| &j.0));
    let model = deps
        .db
        .read(move |conn| Ok(knowledge::resolve_embedding_model(conn, ws)))
        .await
        .map_err(retry)?;
    if let Err(err) = model {
        return Err(if is_embedding_provider_not_configured(&err) {
            FileError::Code("noEmbeddingProvider")
        } else {
            retry(err)
        });
    }
    if file.size < 0 || file.size as u64 > limits.max_file_bytes {
        return Err(FileError::Code("fileTooLarge"));
    }
    let Some(files) = &deps.files else {
        return Err(FileError::Retry("File storage is not configured".into()));
    };
    let bytes = files
        .get_object(&file.path)
        .await
        .map_err(|e| FileError::Retry(format!("Failed to read the file from storage: {e}")))?;
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(FileError::Code("fileTooLarge"));
    }
    let mime = extract::mime_of(&file.mime_type, &file.path);
    let document = extract::knowledge_document(
        &bytes,
        &mime,
        &extract::Options {
            source_name: file.name.clone(),
            max_text_chars: limits.max_text_chars,
            force_prose: config.strategy == AiFileChunkingConfigStrategy::Prose,
            tabular_header_row: config.tabular_header_row.max(1) as usize,
        },
    )
    .map_err(|err| {
        tracing::warn!(ai_file_id, err = %err, mime_type = %mime, "AI file extraction failed");
        FileError::Code("fileUnreadable")
    })?;
    if let Document::Tabular {
        sheets,
        truncated: true,
    } = &document
    {
        tracing::warn!(
            ai_file_id,
            workspace_id = ws,
            sheet_count = sheets.len(),
            row_count = sheets.iter().map(|s| s.rows.len()).sum::<usize>(),
            "AI file tabular extraction reached its text limit"
        );
    }
    let result = chunk::chunk_document(&document, config.chunk_size, config.overlap_size)
        .map_err(FileError::Retry)?;
    if result.chunks.len() > limits.max_chunks {
        return Err(FileError::Code("tooManyChunks"));
    }
    // Reconciling an empty set would delete every existing chunk.
    if result.chunks.is_empty() {
        return Err(FileError::Code("noExtractableContent"));
    }
    if !lease("beforeReconcile").await? {
        return Ok(());
    }
    let id_text = ai_file_id.to_string();
    let chunks: Vec<AiFileEmbeddingChunk> = result
        .chunks
        .iter()
        .enumerate()
        .map(|(i, content)| AiFileEmbeddingChunk {
            chunk_index: i as i64,
            content: content.clone(),
            id: create_ai_file_embedding_id(&id_text, i as i64, content),
        })
        .collect();
    let pending = deps
        .db
        .write(move |tx| {
            repo::reconcile_pending_chunks(tx, ai_file_id, ws, &chunks).map_err(repo_err)
        })
        .await
        .map_err(retry)?;
    let version = chatbotx_business::ai_file::AI_FILE_CHUNKER_VERSIONS
        .iter()
        .find(|(s, _)| *s == result.strategy)
        .map_or(1, |(_, v)| *v);
    service::mark_chunked(
        &deps.db,
        ai_file_id,
        ws,
        json!({
            "strategy": result.strategy,
            "chunkerVersion": version,
            "chunkSize": result.chunk_size,
            "overlapSize": result.overlap_size,
            "chunkCount": result.chunks.len(),
            "processedAt": to_iso_string(now_ms()).unwrap_or_default(),
            "extractedChars": document.extracted_chars(),
        }),
    )
    .await
    .map_err(retry)?;
    if !pending.is_empty() {
        if !lease("beforeEnqueue").await? {
            return Ok(());
        }
        let jobs: Vec<NewJob> = pending
            .iter()
            .map(|id| NewJob {
                name: "processPendingEmbedding".into(),
                data: json!({
                    "type": "processPendingEmbedding",
                    "data": { "aiEmbeddingId": id.to_string() },
                }),
                opts: JobOptions {
                    job_id: Some(ai_file_embedding_job_id(
                        &id_text,
                        &id.to_string(),
                        run_id.as_deref(),
                    )),
                    priority: Some(EMBEDDING_PRIORITY),
                    ..JobOptions::default()
                },
            })
            .collect();
        deps.jobs
            .add_bulk(QueueName::AiAgent, jobs)
            .await
            .map_err(retry)?;
    }
    if let Some(run) = run_id {
        service::mark_processing_run_succeeded(&deps.db, ai_file_id, ws, run)
            .await
            .map_err(retry)?;
    }
    Ok(())
}

/// The aiAgent queue processor: only `processPendingEmbedding` is in Rust.
pub async fn handle_ai_agent(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("");
    if kind != "processPendingEmbedding" {
        // ponytail: automated response, summaries, conversation sources,
        // comment / story replies are later slices (Q.W2 spec).
        return Err(JobError::unrecoverable(format!(
            "AI job action is not available: {kind}"
        )));
    }
    let Some(id) = job.data["data"].get("aiEmbeddingId").and_then(id_of) else {
        return Err(JobError::unrecoverable(
            "Invalid processPendingEmbedding data",
        ));
    };
    let result = process_pending_embedding(&deps, id).await;
    // `markEmbeddingChunkFailed`: the last attempt marks the chunk `error`.
    if result.is_err()
        && (job.is_final_attempt() || matches!(result, Err(JobError::Unrecoverable(_))))
    {
        let ws = deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "workspaceId" FROM "AIEmbedding" WHERE "id" = ?1"#,
                        [id],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?)
            })
            .await
            .ok()
            .flatten();
        match ws {
            Some(ws) => {
                if let Err(err) = service::mark_embedding_failed(&deps.db, id, ws).await {
                    tracing::error!(err = %err, embedding_id = id, "Failed to mark the embedding chunk failed");
                }
            }
            None => tracing::error!(
                embedding_id = id,
                "Cannot mark the embedding chunk failed without a workspace"
            ),
        }
    }
    result.map(|()| Value::Null)
}

/// `processPendingEmbedding`.
pub async fn process_pending_embedding(deps: &Deps, id: i64) -> Result<(), JobError> {
    let row = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "status", "content", "workspaceId" FROM "AIEmbedding" WHERE "id" = ?1"#,
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some((status, content, ws)) = row else {
        return Err(JobError::retryable("AI embedding not found"));
    };
    // A reindex re-queues pending chunks; the duplicate has nothing to do.
    if status == "success" {
        return Ok(());
    }
    if status != "pending" && status != "processing" {
        return Err(JobError::retryable(
            "AI embedding is processing or already processed",
        ));
    }
    let model: ResolvedEmbeddingModel = match deps
        .db
        .read(move |conn| Ok(knowledge::resolve_embedding_model(conn, ws)))
        .await
        .map_err(db_fail)?
    {
        Ok(model) => model,
        Err(err) => {
            tracing::error!(err = %err, "processPendingEmbedding item failed for embeddingId: {id}");
            return Err(JobError::retryable(err.to_string()));
        }
    };
    let vector = match knowledge::embed(&*deps.ai.embedder, &model, &content).await {
        Ok(vector) => vector,
        Err(err) => {
            if is_invalid_api_key_error(Some(&err)) {
                // Surfaced on the workspace's Error Logs, then failed without retry.
                let input = crate::events::ProviderError {
                    provider: crate::events::ai_error_log_provider(model.provider).to_owned(),
                    workspace_id: ws.to_string(),
                    error: json!({ "message": err.message, "statusCode": err.status }),
                    ..Default::default()
                };
                crate::events::log_provider_errors(deps.events.as_ref(), &[input]).await;
                return Err(JobError::unrecoverable(format!(
                    "Embedding provider rejected the API key: {}",
                    err.message
                )));
            }
            tracing::error!(err = %err.message, "processPendingEmbedding item failed for embeddingId: {id}");
            return Err(JobError::retryable(err.message));
        }
    };
    let vector = Value::from(vector.iter().map(|v| f64::from(*v)).collect::<Vec<_>>()).to_string();
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "AIEmbedding" SET "embedding" = vector32(?1), "updatedAt" = ?2, "status" = 'success'
                   WHERE "id" = ?3"#,
                params![vector, now_ms(), id],
            )?;
            Ok(())
        })
        .await
        .map_err(db_fail)
}

#[cfg(test)]
mod tests {
    //! `process-ai-file.test.ts` and `process-pending-embeddings.test.ts`
    //! on a migrated DB and a local file store (the originals mock both).
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chatbotx_business::ai::knowledge::{EmbedFuture, Embedder, ProviderCallError};
    use chatbotx_files::{Backend, FilesConfig, Uploader};

    use super::*;
    use crate::ai::Ai;
    use crate::steps::test_support::Fixture;

    const FILE: i64 = 300;
    const LIMITS: Limits = Limits {
        max_file_bytes: 50 << 20,
        max_text_chars: 5_000_000,
        max_chunks: 5000,
    };

    /// Answers `[0.5; 1536]`, or the scripted error.
    struct FakeEmbedder(Option<ProviderCallError>, AtomicUsize);

    impl Embedder for FakeEmbedder {
        fn embed<'a>(&'a self, _: &'a ResolvedEmbeddingModel, _: &'a str) -> EmbedFuture<'a> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let out = self.0.clone().map_or(Ok(vec![0.5; 1536]), Err);
            Box::pin(async move { out })
        }
    }

    struct Files {
        fx: Fixture,
        root: std::path::PathBuf,
        embedder: Arc<FakeEmbedder>,
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A workspace with OpenAI connected and the stored file `content`.
    async fn stored(
        mime: &str,
        content: &[u8],
        config: Value,
        run: bool,
    ) -> (Files, Option<String>) {
        stored_with(mime, content, config, run, None).await
    }

    async fn stored_with(
        mime: &str,
        content: &[u8],
        config: Value,
        run: bool,
        embed_error: Option<ProviderCallError>,
    ) -> (Files, Option<String>) {
        let mut fx = Fixture::new().await;
        let root =
            std::env::temp_dir().join(format!("cx-ai-files-{}", chatbotx_db::repo::new_id()));
        let path = "workspaces/2/ai-files/1.txt";
        std::fs::create_dir_all(root.join("workspaces/2/ai-files")).unwrap();
        std::fs::write(root.join(path), content).unwrap();
        let config_files = FilesConfig {
            backend: Backend::Local { root: root.clone() },
            signing_secret: None,
            url_base: "http://files.test".into(),
        };
        fx.deps.files = Some(Arc::new(Uploader::new(&config_files).unwrap()));
        let embedder = Arc::new(FakeEmbedder(embed_error, AtomicUsize::new(0)));
        fx.deps.ai = Arc::new(Ai {
            http: Arc::new(crate::ai::llm::ReqwestHttp::default()),
            embedder: embedder.clone(),
        });
        let processing = run.then(|| service::new_processing_run(now_ms()));
        let run_id = processing
            .as_ref()
            .map(|p| p["id"].as_str().unwrap().to_owned());
        let (mime, size) = (mime.to_owned(), content.len() as i64);
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (79, 2, 'openai')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationOpenai" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
                   VALUES (80, '{"authType":"secretText","secretText":"sk-test"}', 'gpt-4o', 1000, 2, 79)"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "chunkingConfig",
                     "processingRun", "workspaceId")
                   VALUES (?1, 'notes.txt', ?2, ?3, ?4, ?5, ?6, 2)"#,
                params![
                    FILE,
                    path,
                    size,
                    mime,
                    config.to_string(),
                    processing.map(|p| p.to_string())
                ],
            )?;
            Ok(())
        })
        .await;
        (Files { fx, root, embedder }, run_id)
    }

    async fn chunks(fx: &Fixture) -> Vec<(i64, i64, String, String)> {
        fx.deps
            .db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    r#"SELECT "id", "chunkIndex", "content", "status" FROM "AIEmbedding" ORDER BY "chunkIndex""#,
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    async fn file_json(fx: &Fixture, column: &'static str) -> Value {
        fx.deps
            .db
            .read(move |conn| {
                let text: Option<String> = conn.query_row(
                    &format!(r#"SELECT "{column}" FROM "AIFile" WHERE "id" = ?1"#),
                    [FILE],
                    |r| r.get(0),
                )?;
                Ok(text.map_or(Value::Null, |t| serde_json::from_str(&t).unwrap()))
            })
            .await
            .unwrap()
    }

    fn code(result: Result<(), FileError>) -> &'static str {
        match result {
            Err(FileError::Code(code)) => code,
            other => panic!("expected a run error code, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn locks_reconciles_chunks_and_enqueues_deterministic_pending_jobs() {
        let text = vec!["alpha"; 60].join(" ");
        let (f, _) = stored(
            "text/plain",
            text.as_bytes(),
            json!({ "chunkSize": 200, "overlapSize": 0 }),
            false,
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        let rows = chunks(&f.fx).await;
        assert!(rows.len() > 1);
        assert!(rows.iter().all(|r| r.2.len() <= 200 && r.3 == "pending"));
        assert_eq!(
            rows.iter().map(|r| r.1).collect::<Vec<_>>(),
            (0..rows.len() as i64).collect::<Vec<_>>()
        );
        // The ids are the legacy deterministic ids.
        assert_eq!(rows[0].0, create_ai_file_embedding_id("300", 0, &rows[0].2));
        let jobs = f.fx.jobs.named("processPendingEmbedding");
        assert_eq!(jobs.len(), rows.len());
        assert_eq!(jobs[0].opts.priority, Some(5));
        assert_eq!(
            jobs[0].opts.job_id.as_deref(),
            Some(format!("ai-file-embedding-300-{}", rows[0].0).as_str())
        );
        assert_eq!(
            jobs[0].data["data"]["aiEmbeddingId"],
            json!(rows[0].0.to_string())
        );
    }

    #[tokio::test]
    async fn does_not_re_enqueue_chunks_that_reconciled_as_already_successful() {
        let (f, _) = stored("text/plain", b"alpha beta gamma delta", json!(null), false).await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        f.fx.exec(r#"UPDATE "AIEmbedding" SET "status" = 'success'"#)
            .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        assert_eq!(chunks(&f.fx).await[0].3, "success");
        assert_eq!(f.fx.jobs.named("processPendingEmbedding").len(), 1);
        let chunking = file_json(&f.fx, "chunking").await;
        assert_eq!(chunking["strategy"], "prose");
        assert_eq!(chunking["chunkCount"], 1);
        assert_eq!(chunking["chunkerVersion"], 1);
    }

    #[tokio::test]
    async fn fences_reindex_side_effects_with_the_processing_run_lease_and_records_success() {
        let (f, run) = stored("text/plain", b"alpha beta gamma delta", json!(null), true).await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, run.clone())
            .await
            .unwrap();
        let processing = file_json(&f.fx, "processingRun").await;
        assert_eq!(processing["id"], json!(run.unwrap()));
        assert_eq!(processing["status"], "success");
        let rows = chunks(&f.fx).await;
        let jobs = f.fx.jobs.named("processPendingEmbedding");
        assert_eq!(
            jobs[0].opts.job_id.as_deref(),
            Some(
                format!(
                    "ai-file-embedding-300-{}-r{}",
                    rows[0].0,
                    processing["id"].as_str().unwrap()
                )
                .as_str()
            )
        );
    }

    #[tokio::test]
    async fn skips_processing_when_a_run_has_been_superseded() {
        let (f, _) = stored("text/plain", b"alpha", json!(null), true).await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, Some("other-run".into()))
            .await
            .unwrap();
        assert!(chunks(&f.fx).await.is_empty());
        assert!(file_json(&f.fx, "chunking").await.is_null());
    }

    #[tokio::test]
    async fn throws_file_too_large_for_a_stored_size_over_the_limit() {
        let (f, run) = stored("text/plain", b"alpha", json!(null), true).await;
        f.fx.exec(r#"UPDATE "AIFile" SET "size" = 62914560"#).await;
        assert_eq!(
            code(process_ai_file(&f.fx.deps, &LIMITS, FILE, run).await),
            "fileTooLarge"
        );
        assert!(chunks(&f.fx).await.is_empty());
    }

    #[tokio::test]
    async fn throws_too_many_chunks() {
        let text = (0..1000)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let (f, run) = stored(
            "text/plain",
            text.as_bytes(),
            json!({ "chunkSize": 200, "overlapSize": 0 }),
            true,
        )
        .await;
        let limits = Limits {
            max_chunks: 10,
            ..LIMITS
        };
        assert_eq!(
            code(process_ai_file(&f.fx.deps, &limits, FILE, run).await),
            "tooManyChunks"
        );
        assert!(chunks(&f.fx).await.is_empty());
    }

    #[tokio::test]
    async fn throws_file_unreadable_for_a_file_the_extractor_rejects() {
        let (f, run) = stored("application/pdf", b"%PDF-1.4 bad zip", json!(null), true).await;
        assert_eq!(
            code(process_ai_file(&f.fx.deps, &LIMITS, FILE, run).await),
            "fileUnreadable"
        );
    }

    #[tokio::test]
    async fn throws_no_embedding_provider_as_a_permanent_error() {
        let (f, run) = stored("text/plain", b"alpha", json!(null), true).await;
        f.fx.exec(r#"DELETE FROM "IntegrationOpenai""#).await;
        let result = process_ai_file(&f.fx.deps, &LIMITS, FILE, run).await;
        assert_eq!(code(result), "noEmbeddingProvider");
        assert!(matches!(
            JobError::from(FileError::Code("noEmbeddingProvider")),
            JobError::Unrecoverable(_)
        ));
    }

    #[tokio::test]
    async fn rethrows_a_storage_read_error_so_the_job_is_retried() {
        let (f, run) = stored("text/plain", b"alpha", json!(null), true).await;
        std::fs::remove_file(f.root.join("workspaces/2/ai-files/1.txt")).unwrap();
        let result = process_ai_file(&f.fx.deps, &LIMITS, FILE, run).await;
        assert!(matches!(result, Err(FileError::Retry(_))), "{result:?}");
        assert!(chunks(&f.fx).await.is_empty());
    }

    #[tokio::test]
    async fn fails_an_empty_extraction_instead_of_wiping_the_existing_chunks() {
        let (f, run) = stored("text/plain", b"alpha", json!(null), false).await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        std::fs::write(f.root.join("workspaces/2/ai-files/1.txt"), b"   ").unwrap();
        assert_eq!(
            code(process_ai_file(&f.fx.deps, &LIMITS, FILE, run).await),
            "noExtractableContent"
        );
        assert_eq!(chunks(&f.fx).await.len(), 1);
    }

    #[tokio::test]
    async fn chunks_tabular_documents_by_row_and_records_the_tabular_strategy() {
        let (f, _) = stored(
            "text/csv",
            b"Product,Price\nBasic,199000\nPro,499000",
            json!(null),
            false,
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        let rows = chunks(&f.fx).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].2,
            "Sheet: notes.txt\n\nProduct: Basic\nPrice: 199000\n\nProduct: Pro\nPrice: 499000"
        );
        let chunking = file_json(&f.fx, "chunking").await;
        assert_eq!(chunking["strategy"], "tabular");
        assert_eq!(chunking["overlapSize"], 0);
    }

    #[tokio::test]
    async fn chunks_markdown_documents_by_heading_and_records_the_markdown_strategy() {
        let (f, _) = stored(
            "text/markdown",
            b"# Policy\n## Returns\nKeep receipts.",
            json!(null),
            false,
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        assert_eq!(chunks(&f.fx).await[0].2, "Policy > Returns\nKeep receipts.");
        assert_eq!(file_json(&f.fx, "chunking").await["strategy"], "markdown");
    }

    #[tokio::test]
    async fn a_prose_config_reads_markdown_as_plain_text() {
        let (f, _) = stored(
            "text/markdown",
            b"# Policy\n\n- Keep the **receipt**",
            json!({ "strategy": "prose" }),
            false,
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        assert_eq!(chunks(&f.fx).await[0].2, "Policy\n\nKeep the receipt");
        assert_eq!(file_json(&f.fx, "chunking").await["strategy"], "prose");
    }

    // processPendingEmbedding

    async fn one_pending() -> Files {
        let (f, _) = stored("text/plain", b"knowledge chunk", json!(null), false).await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        f
    }

    #[tokio::test]
    async fn embeds_a_pending_chunk_and_marks_it_success() {
        let f = one_pending().await;
        let id = chunks(&f.fx).await[0].0;
        process_pending_embedding(&f.fx.deps, id).await.unwrap();
        assert_eq!(chunks(&f.fx).await[0].3, "success");
        let stored: i64 =
            f.fx.deps
                .db
                .read(move |c| {
                    Ok(c.query_row(
                        r#"SELECT length("embedding") FROM "AIEmbedding" WHERE "id" = ?1"#,
                        [id],
                        |r| r.get(0),
                    )?)
                })
                .await
                .unwrap();
        assert_eq!(stored, 6144);
    }

    #[tokio::test]
    async fn keeps_the_embedding_pending_and_rejects_when_its_provider_is_unavailable() {
        let f = one_pending().await;
        let id = chunks(&f.fx).await[0].0;
        f.fx.exec(r#"DELETE FROM "IntegrationOpenai""#).await;
        let result = process_pending_embedding(&f.fx.deps, id).await;
        assert!(matches!(result, Err(JobError::Retryable(_))));
        assert_eq!(chunks(&f.fx).await[0].3, "pending");
    }

    #[tokio::test]
    async fn does_nothing_for_a_duplicate_job_whose_chunk_is_already_embedded() {
        let f = one_pending().await;
        let id = chunks(&f.fx).await[0].0;
        f.fx.exec(r#"UPDATE "AIEmbedding" SET "status" = 'success'"#)
            .await;
        process_pending_embedding(&f.fx.deps, id).await.unwrap();
        assert_eq!(f.embedder.1.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn fails_without_retrying_when_the_key_is_rejected() {
        let (f, _) = stored_with(
            "text/plain",
            b"knowledge chunk",
            json!(null),
            false,
            Some(ProviderCallError {
                status: Some(401),
                body: None,
                message: "Incorrect API key provided".into(),
            }),
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        let id = chunks(&f.fx).await[0].0;
        let Err(JobError::Unrecoverable(err)) = process_pending_embedding(&f.fx.deps, id).await
        else {
            panic!("unrecoverable")
        };
        assert_eq!(
            err.to_string(),
            "Embedding provider rejected the API key: Incorrect API key provided"
        );
        assert_eq!(chunks(&f.fx).await[0].3, "pending");
    }

    #[tokio::test]
    async fn rethrows_a_retryable_provider_error() {
        let (f, _) = stored_with(
            "text/plain",
            b"knowledge chunk",
            json!(null),
            false,
            Some(ProviderCallError {
                status: Some(429),
                body: None,
                message: "rate limited".into(),
            }),
        )
        .await;
        process_ai_file(&f.fx.deps, &LIMITS, FILE, None)
            .await
            .unwrap();
        let id = chunks(&f.fx).await[0].0;
        assert!(matches!(
            process_pending_embedding(&f.fx.deps, id).await,
            Err(JobError::Retryable(_))
        ));
    }
}
