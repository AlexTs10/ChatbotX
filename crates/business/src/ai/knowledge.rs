//! `packages/ai/src/server/{embedding-model,knowledge-base,knowledge-search}.ts`
//! and `core/provider-errors.ts`: the query embedding, the vector search of
//! knowledge chunks and the admin test search (`searchAIFiles`).
//!
//! The embedding call (`ai`'s `embed` with `@ai-sdk/openai` /
//! `@ai-sdk/google`) is one HTTP request behind [`Embedder`], so tests can
//! answer it without the network.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use chatbotx_core::utils::id::is_numeric_id;
use chatbotx_db::Db;
use chatbotx_db::repo::ai_file_embedding::{self as repo, AiFileVectorSearchRow};
use chatbotx_db::repo::{self as db_repo};
use chatbotx_db::rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::{ai_agent, ai_file, tx};

/// `openaiEmbeddingModels.enum["text-embedding-ada-002"]`.
pub const OPENAI_EMBEDDING_MODEL: &str = "text-embedding-ada-002";
/// `geminiEmbeddingModels.enum["gemini-embedding-001"]`.
pub const GEMINI_EMBEDDING_MODEL: &str = "gemini-embedding-001";
/// The stored vector size (`vector(1536)`; Gemini `outputDimensionality`).
pub const EMBEDDING_DIMENSIONS: usize = 1536;
/// `DEFAULT_FILE_SEARCH_MAX_RESULTS`: chunks the agent's tool returns.
pub const DEFAULT_FILE_SEARCH_MAX_RESULTS: i64 = 5;
/// `AI_FILE_SEARCH_DEFAULT_LIMIT`.
pub const AI_FILE_SEARCH_DEFAULT_LIMIT: i64 = 10;
/// `AI_FILE_SEARCH_MAX_LIMIT`.
pub const AI_FILE_SEARCH_MAX_LIMIT: i64 = 20;
/// `ai`'s default `maxRetries`.
const EMBED_MAX_RETRIES: u32 = 2;
/// `ai`'s default first retry delay (doubled per retry).
const EMBED_RETRY_DELAY: Duration = Duration::from_millis(2000);

/// `embeddingSimilarityThresholds[provider]` (`openai` 0.7, `gemini` 0.55).
pub fn similarity_threshold(provider: &str) -> f64 {
    if provider == "gemini" { 0.55 } else { 0.7 }
}

/// `ResolvedEmbeddingModel` (the key instead of an SDK model).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEmbeddingModel {
    /// `openai` / `gemini`.
    pub provider: &'static str,
    pub model_id: &'static str,
    pub api_key: String,
}

/// `new EmbeddingProviderNotConfiguredError(message)` (a plain `Error`: 500
/// when it reaches a route).
fn not_configured(message: &str) -> BusinessError {
    BusinessError::Repo(db_repo::Error::Coded {
        kind: "EmbeddingProviderNotConfiguredError",
        code: "embeddingProviderNotConfigured".to_string(),
        message: message.to_string(),
    })
}

/// Whether `error` is `EmbeddingProviderNotConfiguredError`.
pub fn is_embedding_provider_not_configured(error: &BusinessError) -> bool {
    matches!(
        error,
        BusinessError::Repo(db_repo::Error::Coded {
            kind: "EmbeddingProviderNotConfiguredError",
            ..
        })
    )
}

/// `secretTextAuthSchema.safeParse(auth)`: the trimmed key, when valid.
fn secret_text(auth: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth).ok()?;
    if auth.get("authType")?.as_str()? != "secretText" {
        return None;
    }
    let key = auth.get("secretText")?.as_str()?.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// `resolveEmbeddingModel`: OpenAI first, then Gemini.
pub fn resolve_embedding_model(
    conn: &Connection,
    workspace_id: i64,
) -> Result<ResolvedEmbeddingModel> {
    let auth = |table: &str| -> Result<Option<String>> {
        conn.query_row(
            &format!(r#"SELECT "auth" FROM "{table}" WHERE "workspaceId" = ?1 LIMIT 1"#),
            [workspace_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    };
    for (table, provider, model_id, label) in [
        (
            "IntegrationOpenai",
            "openai",
            OPENAI_EMBEDDING_MODEL,
            "OpenAI",
        ),
        (
            "IntegrationGemini",
            "gemini",
            GEMINI_EMBEDDING_MODEL,
            "Gemini",
        ),
    ] {
        if let Some(auth) = auth(table)? {
            let api_key = secret_text(&auth).ok_or_else(|| {
                not_configured(&format!("Invalid {label} integration auth configuration"))
            })?;
            return Ok(ResolvedEmbeddingModel {
                provider,
                model_id,
                api_key,
            });
        }
    }
    Err(not_configured(
        "No embedding provider configured. AI file embeddings require OpenAI or Gemini integration. DeepSeek, Claude and Muse do not support embedding models.",
    ))
}

/// `APICallError` of a provider call (`status` `None` = no HTTP answer).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCallError {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub message: String,
}

impl ProviderCallError {
    /// `APICallError.isRetryable`: no answer, 408, 409, 429 or 5xx.
    pub fn is_retryable(&self) -> bool {
        self.status
            .is_none_or(|s| matches!(s, 408 | 409 | 429) || s >= 500)
    }
}

/// `isInvalidApiKeyError`: 401 / 403, or Gemini's 400 naming
/// `API_KEY_INVALID`. `None` (not a provider call error) is never one.
pub fn is_invalid_api_key_error(error: Option<&ProviderCallError>) -> bool {
    let Some(error) = error else {
        return false;
    };
    match error.status {
        Some(401 | 403) => true,
        Some(400) => error
            .body
            .as_deref()
            .is_some_and(|b| b.contains("API_KEY_INVALID")),
        _ => false,
    }
}

/// A boxed future of [`Embedder::embed`].
pub type EmbedFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<Vec<f32>, ProviderCallError>> + Send + 'a>>;

/// One `embed({ model, value })` request.
pub trait Embedder: Send + Sync {
    /// The embedding of `value` (one try; [`embed`] retries).
    fn embed<'a>(&'a self, model: &'a ResolvedEmbeddingModel, value: &'a str) -> EmbedFuture<'a>;
}

/// The provider HTTP APIs (`@ai-sdk/openai` / `@ai-sdk/google`).
#[derive(Debug, Clone, Default)]
pub struct HttpEmbedder {
    http: reqwest::Client,
}

impl HttpEmbedder {
    pub fn new() -> HttpEmbedder {
        HttpEmbedder::default()
    }
}

/// The request of `model` for `value`: URL, headers, JSON body.
pub fn embed_request(
    model: &ResolvedEmbeddingModel,
    value: &str,
) -> (String, Vec<(&'static str, String)>, Value) {
    if model.provider == "gemini" {
        (
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent",
                model.model_id
            ),
            vec![("x-goog-api-key", model.api_key.clone())],
            json!({
                "model": format!("models/{}", model.model_id),
                "content": { "parts": [{ "text": value }] },
                "outputDimensionality": EMBEDDING_DIMENSIONS,
            }),
        )
    } else {
        (
            "https://api.openai.com/v1/embeddings".to_string(),
            vec![("Authorization", format!("Bearer {}", model.api_key))],
            json!({ "model": model.model_id, "input": [value], "encoding_format": "float" }),
        )
    }
}

/// The vector of a provider answer (`data[0].embedding` / `embedding.values`).
pub fn parse_embedding(provider: &str, body: &Value) -> Option<Vec<f32>> {
    let values = if provider == "gemini" {
        body.pointer("/embedding/values")
    } else {
        body.pointer("/data/0/embedding")
    }?;
    values
        .as_array()?
        .iter()
        .map(|v| v.as_f64().map(|f| f as f32))
        .collect()
}

impl Embedder for HttpEmbedder {
    fn embed<'a>(&'a self, model: &'a ResolvedEmbeddingModel, value: &'a str) -> EmbedFuture<'a> {
        Box::pin(async move {
            let (url, headers, body) = embed_request(model, value);
            let mut request = self
                .http
                .post(&url)
                .header("content-type", "application/json")
                .body(body.to_string());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            let failed = |status, body, message: String| ProviderCallError {
                status,
                body,
                message,
            };
            let response = request
                .send()
                .await
                .map_err(|e| failed(None, None, format!("Cannot connect to API: {e}")))?;
            let status = response.status().as_u16();
            let text = response
                .text()
                .await
                .map_err(|e| failed(Some(status), None, e.to_string()))?;
            if !(200..300).contains(&status) {
                return Err(failed(
                    Some(status),
                    Some(text),
                    format!("Provider request failed with status {status}"),
                ));
            }
            serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|body| parse_embedding(model.provider, &body))
                .ok_or_else(|| {
                    failed(
                        Some(status),
                        Some(text),
                        "Invalid JSON response".to_string(),
                    )
                })
        })
    }
}

/// `embed(...)` with `ai`'s retries (2, after 2 s then 4 s, retryable errors
/// only).
// ponytail: no Retry-After parsing; the SDK prefers it when under 60 s.
pub async fn embed(
    embedder: &dyn Embedder,
    model: &ResolvedEmbeddingModel,
    value: &str,
) -> std::result::Result<Vec<f32>, ProviderCallError> {
    let mut delay = EMBED_RETRY_DELAY;
    let mut attempt = 0;
    loop {
        match embedder.embed(model, value).await {
            Err(error) if error.is_retryable() && attempt < EMBED_MAX_RETRIES => {
                attempt += 1;
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            other => return other,
        }
    }
}

/// A provider failure that reaches a caller (a plain error: 500).
fn provider_error(error: ProviderCallError) -> BusinessError {
    BusinessError::Repo(db_repo::Error::Coded {
        kind: "APICallError",
        code: error.status.map_or_else(String::new, |s| s.to_string()),
        message: error.message,
    })
}

/// `FileSearchConfig`.
#[derive(Debug, Clone)]
pub struct FileSearchConfig {
    pub workspace_id: i64,
    pub selected_file_ids: Vec<String>,
    pub similarity_threshold: Option<f64>,
    pub max_results: i64,
}

/// One match of [`search_knowledge_chunks`].
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeMatch {
    pub id: i64,
    pub content: String,
    pub ai_file_id: i64,
    pub similarity: f64,
    pub chunk_index: Option<i64>,
    pub rank: i64,
    pub passed_threshold: bool,
}

/// `appliedConfig` of [`search_knowledge_chunks`].
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedConfig {
    pub provider: &'static str,
    pub model_id: &'static str,
    pub similarity_threshold: f64,
    pub limit: i64,
}

/// The result of [`search_knowledge_chunks`].
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeSearch {
    pub matches: Vec<KnowledgeMatch>,
    pub applied_config: AppliedConfig,
    /// `(embed, search)` milliseconds.
    pub timings_ms: (u64, u64),
}

/// Ranks the rows (1-based) and marks `similarity > threshold`. A row
/// without a finite similarity fails the whole parse: no matches and a
/// warning, as the zod parse of the original.
pub fn rank_matches(
    workspace_id: i64,
    rows: Vec<AiFileVectorSearchRow>,
    threshold: f64,
) -> Vec<KnowledgeMatch> {
    if rows
        .iter()
        .any(|r| !r.similarity.is_some_and(f64::is_finite))
    {
        tracing::warn!(
            workspace_id,
            "[ai-package] Invalid similarity search results"
        );
        return Vec::new();
    }
    rows.into_iter()
        .zip(1..)
        .map(|(row, rank)| {
            let similarity = row.similarity.unwrap_or_default();
            KnowledgeMatch {
                id: row.id,
                content: row.content,
                ai_file_id: row.ai_file_id,
                similarity,
                chunk_index: row.chunk_index,
                rank,
                passed_threshold: similarity > threshold,
            }
        })
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// `searchKnowledgeChunks`: embeds the query and returns the nearest ready
/// chunks of the selected files, below-threshold ones included.
pub async fn search_knowledge_chunks(
    db: &Db,
    embedder: &dyn Embedder,
    query: &str,
    config: FileSearchConfig,
) -> Result<KnowledgeSearch> {
    let started = Instant::now();
    let ws = config.workspace_id;
    let model = tx::read(db, move |conn| resolve_embedding_model(conn, ws)).await?;
    let vector = embed(embedder, &model, query)
        .await
        .map_err(provider_error)?;
    let embed_ms = elapsed_ms(started);
    let threshold = config
        .similarity_threshold
        .unwrap_or_else(|| similarity_threshold(model.provider));
    let applied_config = AppliedConfig {
        provider: model.provider,
        model_id: model.model_id,
        similarity_threshold: threshold,
        limit: config.max_results,
    };
    let ids: Vec<i64> = config
        .selected_file_ids
        .iter()
        .filter(|id| is_numeric_id(id))
        .filter_map(|id| id.parse().ok())
        .collect();
    if ids.is_empty() {
        return Ok(KnowledgeSearch {
            matches: Vec::new(),
            applied_config,
            timings_ms: (embed_ms, 0),
        });
    }
    let started = Instant::now();
    let limit = config.max_results;
    let rows = tx::read(db, move |conn| {
        repo::vector_search(conn, ws, &ids, &vector, limit).map_err(BusinessError::Repo)
    })
    .await?;
    Ok(KnowledgeSearch {
        matches: rank_matches(ws, rows, threshold),
        applied_config,
        timings_ms: (embed_ms, elapsed_ms(started)),
    })
}

/// `performFileSearch`: the above-threshold matches (the agent's tool).
pub async fn perform_file_search(
    db: &Db,
    embedder: &dyn Embedder,
    query: &str,
    config: FileSearchConfig,
) -> Result<Vec<KnowledgeMatch>> {
    let search = search_knowledge_chunks(db, embedder, query, config).await?;
    Ok(search
        .matches
        .into_iter()
        .filter(|m| m.passed_threshold)
        .collect())
}

/// `searchAIFiles` input.
#[derive(Debug, Clone, Default)]
pub struct SearchAiFilesInput {
    pub workspace_id: i64,
    pub query: String,
    pub agent_id: Option<i64>,
    pub file_ids: Option<Vec<String>>,
    pub limit: Option<i64>,
    pub similarity_threshold: Option<f64>,
    /// Text the admin expects the answer to contain.
    pub expected_text: Option<String>,
}

/// `parseToolIds(tools, prefix)`.
pub fn parse_tool_ids(tools: &Value, prefix: &str) -> Vec<String> {
    let marker = format!("{prefix}:");
    tools
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|tool| tool.starts_with(prefix))
        .map(|tool| tool.replacen(&marker, "", 1))
        .filter(|id| !id.is_empty())
        .collect()
}

fn unique(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// `resolveSearchFiles`: agent mode drops `file:` tools of deleted files;
/// files mode needs every id in the workspace.
fn resolve_search_files(
    conn: &Connection,
    input: &SearchAiFilesInput,
    agent_mode: bool,
) -> Result<Vec<chatbotx_db::repo::models::AiFile>> {
    let ws = input.workspace_id;
    if agent_mode {
        let agent = ai_agent::find_in(conn, ws, input.agent_id.unwrap_or_default())?
            .ok_or_else(|| BusinessError::not_found("AI agent not found"))?;
        let ids = unique(parse_tool_ids(&agent.tools.0, "file"));
        return ai_file::find_many_by_ids(conn, ws, &ids);
    }
    let ids = unique(input.file_ids.clone().unwrap_or_default());
    let files = ai_file::find_many_by_ids(conn, ws, &ids)?;
    if files.len() != ids.len() {
        return Err(BusinessError::not_found("AI file not found"));
    }
    Ok(files)
}

/// `searchAIFiles`: the test search of the knowledge-base pages and
/// `aiFiles.search`. Returns the `AIFileSearchResult` JSON.
pub async fn search_ai_files(
    db: &Db,
    embedder: &dyn Embedder,
    mut input: SearchAiFilesInput,
) -> Result<Value> {
    input.expected_text = input.expected_text.filter(|t| !t.is_empty());
    let has_files = input.file_ids.as_ref().is_some_and(|ids| !ids.is_empty());
    if input.agent_id.is_some() == has_files {
        return Err(BusinessError::validation(
            "agentId",
            "Provide exactly one of agentId or fileIds",
        ));
    }
    let agent_mode = input.agent_id.is_some();
    let mode = if agent_mode { "agent" } else { "files" };
    let ws = input.workspace_id;
    let resolve = input.clone();
    let files = tx::read(db, move |conn| {
        let files = resolve_search_files(conn, &resolve, agent_mode)?;
        if !files.is_empty() {
            ai_file::assert_embedding_provider_configured(conn, ws)?;
        }
        Ok(files)
    })
    .await?;
    if files.is_empty() {
        return Ok(json!({
            "mode": mode,
            "results": [],
            "appliedConfig": null,
            "files": [],
            "diagnosis": "agentHasNoFiles",
            "expected": null,
            "timingsMs": { "embed": 0, "search": 0 },
        }));
    }
    let file_ids: Vec<i64> = files.iter().map(|f| f.id).collect();
    // Agent mode never returns fewer rows than the agent itself would see.
    let floor = if agent_mode || input.expected_text.is_some() {
        DEFAULT_FILE_SEARCH_MAX_RESULTS
    } else {
        1
    };
    let limit = AI_FILE_SEARCH_MAX_LIMIT
        .min(floor.max(input.limit.unwrap_or(AI_FILE_SEARCH_DEFAULT_LIMIT)));
    let searched = search_knowledge_chunks(
        db,
        embedder,
        &input.query,
        FileSearchConfig {
            workspace_id: ws,
            selected_file_ids: file_ids.iter().map(i64::to_string).collect(),
            max_results: limit,
            similarity_threshold: input.similarity_threshold,
        },
    )
    .await?;
    let ids = file_ids.clone();
    let counts = tx::read(db, move |conn| {
        ai_file::count_embeddings_by_status(conn, ws, &ids)
    })
    .await?;
    let files_with_status: Vec<(i64, i64)> = files
        .iter()
        .map(|f| counts.get(&f.id).copied().unwrap_or((0, 0)))
        .collect();
    let names: HashMap<i64, &str> = files.iter().map(|f| (f.id, f.name.as_str())).collect();
    // The agent always runs on the provider default and its top 5, so a
    // trial threshold never changes `usedByAgent`.
    let default_threshold = similarity_threshold(searched.applied_config.provider);
    let results: Vec<SearchResult> = searched
        .matches
        .iter()
        .map(|m| SearchResult {
            agent_visible: m.similarity > default_threshold
                && m.rank <= DEFAULT_FILE_SEARCH_MAX_RESULTS,
            file_name: names.get(&m.ai_file_id).copied().unwrap_or("").to_string(),
            m: m.clone(),
        })
        .collect();
    let expected = match input.expected_text.clone() {
        Some(text) => Some(evaluate_expected(db, ws, &file_ids, text, &results).await?),
        None => None,
    };
    let diagnosis = if files_with_status.iter().all(|(_, ready)| *ready == 0) {
        "noReadyChunks"
    } else if results.is_empty() {
        "noMatches"
    } else if results.iter().all(|r| !r.m.passed_threshold) {
        "allBelowThreshold"
    } else {
        "ok"
    };
    let (embed_ms, search_ms) = searched.timings_ms;
    tracing::info!(
        workspace_id = ws,
        mode,
        result_count = results.len(),
        diagnosis,
        embed_ms,
        search_ms,
        "AI file search completed"
    );
    let applied = &searched.applied_config;
    Ok(json!({
        "mode": mode,
        "results": results.iter().map(|r| r.to_json(agent_mode)).collect::<Vec<_>>(),
        "appliedConfig": {
            "provider": applied.provider,
            "modelId": applied.model_id,
            "similarityThreshold": applied.similarity_threshold,
            "limit": applied.limit,
            "agentDefaultThreshold": default_threshold,
            "agentMaxResults": DEFAULT_FILE_SEARCH_MAX_RESULTS,
        },
        "files": files.iter().zip(&files_with_status).map(|(f, (total, ready))| {
            let status = if *total == 0 {
                "pending"
            } else if ready == total {
                "success"
            } else {
                "processing"
            };
            json!({
                "id": f.id.to_string(),
                "name": f.name,
                "chunksCount": total,
                "processingStatus": status,
                "readyChunks": ready,
            })
        }).collect::<Vec<_>>(),
        "diagnosis": diagnosis,
        "expected": expected,
        "timingsMs": { "embed": embed_ms, "search": search_ms },
    }))
}

/// One result row of [`search_ai_files`].
struct SearchResult {
    m: KnowledgeMatch,
    file_name: String,
    agent_visible: bool,
}

impl SearchResult {
    fn to_json(&self, agent_mode: bool) -> Value {
        let m = &self.m;
        json!({
            "chunkId": m.id.to_string(),
            "fileId": m.ai_file_id.to_string(),
            "fileName": self.file_name,
            "chunkIndex": m.chunk_index,
            "content": m.content,
            "similarity": m.similarity,
            "rank": m.rank,
            "passedThreshold": m.passed_threshold,
            "usedByAgent": if agent_mode { json!(self.agent_visible) } else { Value::Null },
            "agentVisible": self.agent_visible,
        })
    }
}

/// `evaluateExpected`: whether any chunk of the files holds `text` (as the UI
/// highlighter: case-insensitive) and whether the agent would see it.
async fn evaluate_expected(
    db: &Db,
    workspace_id: i64,
    file_ids: &[i64],
    text: String,
    results: &[SearchResult],
) -> Result<Value> {
    let ids: Vec<String> = file_ids.iter().map(i64::to_string).collect();
    let needle = text.trim().to_lowercase();
    let found = tx::read(db, move |conn| {
        ai_file::find_chunks_containing(conn, workspace_id, &ids, &text, None)
    })
    .await?;
    let by_chunk: HashMap<i64, &SearchResult> = results.iter().map(|r| (r.m.id, r)).collect();
    let entry = |chunk: i64, file: i64, name: &str, index: Option<i64>, rank: Option<i64>| {
        let json = json!({
            "chunkId": chunk.to_string(),
            "fileId": file.to_string(),
            "fileName": name,
            "chunkIndex": index,
            "rank": rank,
        });
        (chunk, json)
    };
    let mut matches: Vec<(i64, Value)> = found
        .iter()
        .map(|f| {
            let rank = by_chunk.get(&f.chunk_id).map(|r| r.m.rank);
            entry(f.chunk_id, f.file_id, &f.file_name, f.chunk_index, rank)
        })
        .collect();
    // The lookup is capped and ordered by position, not relevance: add every
    // returned chunk that holds the text so the verdict never depends on the cap.
    let known: HashSet<i64> = matches.iter().map(|m| m.0).collect();
    for r in results {
        if !known.contains(&r.m.id) && r.m.content.to_lowercase().contains(&needle) {
            let m = &r.m;
            matches.push(entry(
                m.id,
                m.ai_file_id,
                &r.file_name,
                m.chunk_index,
                Some(m.rank),
            ));
        }
    }
    let verdict = if matches.is_empty() {
        "notInAnyChunk"
    } else if matches
        .iter()
        .any(|m| by_chunk.get(&m.0).is_some_and(|r| r.agent_visible))
    {
        "inTopResults"
    } else {
        "foundButLowRank"
    };
    Ok(json!({
        "matches": matches.into_iter().map(|(_, json)| json).collect::<Vec<_>>(),
        "verdict": verdict,
    }))
}
