//! `packages/business/src/ai-file/service.ts`: the AI knowledge files of a
//! workspace (`"AIFile"`, chunks in `"AIEmbedding"`).
//!
//! - No template edit locks / delete guards (P3.6-D1 rule): a re-chunk and a
//!   delete never consult the template lock.
//! - The worker side (`processAIFile`, chunking, embedding) is unit Q.W2; this
//!   module enqueues `processAIFile` on the heavy queue with the original
//!   payload and keeps the run helpers the worker calls.
//! - The original's no-op `DELETE "AIEmbedding" WHERE "id" = <file id>` is
//!   not ported: ids come from one generator, so it never matches, and the
//!   chunks cascade with the file.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use chatbotx_core::js::to_iso_string;
use chatbotx_core::utils::id::{create_id, is_numeric_id};
use chatbotx_db::Db;
use chatbotx_db::repo::ai_file_embedding::{
    self as repo, AiFileChunkCursor, AiFileChunkRow, ChunkingConfigUpdate,
};
use chatbotx_db::repo::models::AiFile;
use chatbotx_db::repo::partials::ai::{
    ResolvedAiFileChunkingConfig, parse_ai_file_chunking_config, resolve_ai_file_chunking_config,
};
use chatbotx_db::repo::utils::{
    PaginationInput, escape_like_pattern, get_pagination_with_defaults,
};
use chatbotx_db::repo::{find_many, find_one, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_files::{
    Acl, FilesError, HttpFetcher, InputFile, ObjectPutter, ReqwestFetcher, UploadDeps,
    UploadFromUrlOptions, Uploader, ValidateUrl, upload_file, upload_file_from_url,
};
use chatbotx_jobs::queues::ai_file::ai_file_reindex_job_id;
use chatbotx_jobs::queues::heavy::{HeavyJobAction, heavy_job_options};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use serde_json::{Map, Value, json};

use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::net::ssrf_guard::assert_public_url;
use crate::tx;

/// `AI_FILE_MAX_UPLOAD_BYTES` (100 MB).
pub const AI_FILE_MAX_UPLOAD_BYTES: u64 = 100 * 1000 * 1000;

/// `AI_FILE_PROCESSING_RUN_STALE_MS`: a queued / processing run without a
/// heartbeat this long is replaceable.
pub const AI_FILE_PROCESSING_RUN_STALE_MS: i64 = 60 * 60 * 1000;

/// `AI_FILE_CHUNKER_VERSIONS`.
pub const AI_FILE_CHUNKER_VERSIONS: &[(&str, i64)] =
    &[("prose", 1), ("tabular", 1), ("markdown", 1)];

/// `aiFileProcessingRunErrorCodes`.
pub const AI_FILE_PROCESSING_RUN_ERROR_CODES: &[&str] = &[
    "fileTooLarge",
    "tooManyChunks",
    "fileUnreadable",
    "noExtractableContent",
    "noEmbeddingProvider",
    "enqueueFailed",
    "unknown",
];

const RUN_STATUSES: &[&str] = &["queued", "processing", "success", "error"];
const CHUNK_STRATEGIES: &[&str] = &["prose", "tabular", "markdown"];

/// The `noEmbeddingProvider` message.
pub const NO_EMBEDDING_PROVIDER: &str = "No embedding provider configured. AI file embeddings require OpenAI or Gemini integration. DeepSeek, Claude and Muse do not support embedding models.";

/// The `conflict` message of a re-chunk that meets an active run.
pub const REINDEX_CONFLICT: &str =
    "File is being processed; parameters were not applied. Retry when processingRun finishes.";

/// The `invalidCursor` message.
pub const INVALID_CURSOR: &str = "The cursor is invalid. Start again without a cursor.";

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn system_error(message: &str, status: u16) -> BusinessError {
    BusinessError::exception(message, "systemError", status)
}

/// The services an AI file call uses.
pub struct AiFileDeps<'a> {
    pub db: &'a Db,
    /// The object store (uploads, presigned downloads, deletes).
    pub files: &'a Arc<Uploader>,
    /// The job store; `None` = no queue (an enqueue fails).
    pub jobs: Option<&'a Arc<dyn Enqueuer>>,
    pub audit: &'a AuditService,
    /// The download client of the URL mode; `None` = reqwest.
    pub fetcher: Option<Arc<dyn HttpFetcher>>,
}

// ---------------------------------------------------------------------------
// partials/ai.ts schemas
// ---------------------------------------------------------------------------

/// `z.iso.datetime()`: `YYYY-MM-DDTHH:MM[:SS[.fff…]]Z` (UTC only).
pub fn is_iso_datetime(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        b.get(range.clone())
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    let number = |range: std::ops::Range<usize>| {
        text.get(range)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(99)
    };
    if b.len() < 17
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
        || b[10] != b'T'
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
        || b.last() != Some(&b'Z')
    {
        return false;
    }
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > days || number(11..13) > 23 || number(14..16) > 59 {
        return false;
    }
    let rest = &text[16..text.len() - 1];
    if rest.is_empty() {
        return true;
    }
    let Some(seconds) = rest.strip_prefix(':') else {
        return false;
    };
    let (whole, fraction) = match seconds.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (seconds, None),
    };
    whole.len() == 2
        && whole.bytes().all(|c| c.is_ascii_digit())
        && whole.parse::<u32>().is_ok_and(|s| s <= 59)
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit()))
}

fn int_at(map: &Map<String, Value>, key: &str, min: i64) -> bool {
    map.get(key)
        .and_then(Value::as_f64)
        .is_some_and(|n| n.fract() == 0.0 && n >= min as f64)
}

fn one_of(value: Option<&Value>, allowed: &[&str]) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| allowed.contains(&s))
}

/// `aiFileChunking.safeParse` (a strict object): the value when valid.
pub fn parse_chunking(value: Option<&Value>) -> Option<Value> {
    let map = value?.as_object()?;
    const KEYS: &[&str] = &[
        "strategy",
        "chunkerVersion",
        "chunkSize",
        "overlapSize",
        "chunkCount",
        "processedAt",
        "extractedChars",
    ];
    let valid = map.keys().all(|k| KEYS.contains(&k.as_str()))
        && one_of(map.get("strategy"), CHUNK_STRATEGIES)
        && int_at(map, "chunkerVersion", 1)
        && int_at(map, "chunkSize", 1)
        && int_at(map, "overlapSize", 0)
        && int_at(map, "chunkCount", 0)
        && map
            .get("processedAt")
            .and_then(Value::as_str)
            .is_some_and(is_iso_datetime)
        && map
            .get("extractedChars")
            .is_none_or(|_| int_at(map, "extractedChars", 0));
    valid.then(|| Value::Object(map.clone()))
}

fn nullable_iso(map: &Map<String, Value>, key: &str) -> bool {
    match map.get(key) {
        Some(Value::Null) => true,
        Some(Value::String(text)) => is_iso_datetime(text),
        _ => false,
    }
}

/// `aiFileProcessingRun.safeParse` (a strict object): the value when valid.
pub fn parse_processing_run(value: Option<&Value>) -> Option<Value> {
    let map = value?.as_object()?;
    const KEYS: &[&str] = &[
        "id",
        "status",
        "error",
        "queuedAt",
        "startedAt",
        "heartbeatAt",
        "finishedAt",
    ];
    let valid = map.len() == KEYS.len()
        && map.keys().all(|k| KEYS.contains(&k.as_str()))
        && map
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
        && one_of(map.get("status"), RUN_STATUSES)
        && (map.get("error") == Some(&Value::Null)
            || one_of(map.get("error"), AI_FILE_PROCESSING_RUN_ERROR_CODES))
        && map
            .get("queuedAt")
            .and_then(Value::as_str)
            .is_some_and(is_iso_datetime)
        && nullable_iso(map, "startedAt")
        && nullable_iso(map, "heartbeatAt")
        && nullable_iso(map, "finishedAt");
    valid.then(|| Value::Object(map.clone()))
}

fn iso(ms: i64) -> String {
    to_iso_string(ms).unwrap_or_default()
}

/// The run plus `stale`: a queued / processing run whose `heartbeatAt` (else
/// `queuedAt`) is older than the stale window at `now`. Compared as ISO text,
/// as the original.
pub fn run_with_stale(run: &Value, now: i64) -> Value {
    let mut out = run.clone();
    let status = run["status"].as_str().unwrap_or_default();
    let lease = run["heartbeatAt"]
        .as_str()
        .or_else(|| run["queuedAt"].as_str())
        .unwrap_or_default();
    let stale = matches!(status, "queued" | "processing")
        && lease < iso(now - AI_FILE_PROCESSING_RUN_STALE_MS).as_str();
    if let Value::Object(map) = &mut out {
        map.insert("stale".into(), Value::Bool(stale));
    }
    out
}

/// `chunkingOutdated`: without (valid) metadata the file is outdated once
/// it has chunks; otherwise when its chunker version is older than
/// `versions`.
pub fn chunking_outdated(chunking: Option<&Value>, chunks: i64, versions: &[(&str, i64)]) -> bool {
    match chunking {
        None => chunks > 0,
        Some(chunking) => {
            let strategy = chunking["strategy"].as_str().unwrap_or_default();
            let current = versions
                .iter()
                .find(|(name, _)| *name == strategy)
                .map_or(0, |(_, version)| *version);
            chunking["chunkerVersion"].as_i64().unwrap_or(0) < current
        }
    }
}

// ---------------------------------------------------------------------------
// Read model
// ---------------------------------------------------------------------------

/// The chunk counts of one file (the `aiEmbeddings` relation, aggregated).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingStats {
    pub total: i64,
    /// Chunks with a `chunkIndex`.
    pub numbered: i64,
    pub error: bool,
    /// Some chunk is `pending` or `processing`.
    pub in_progress: bool,
}

/// `processingStatus`: `error` beats `pending` / `processing` (both
/// `processing`) beats `success`; no chunks = `pending`.
pub fn processing_status(stats: &EmbeddingStats) -> &'static str {
    if stats.total == 0 {
        "pending"
    } else if stats.error {
        "error"
    } else if stats.in_progress {
        "processing"
    } else {
        "success"
    }
}

/// `AIFileWithProcessing`.
#[derive(Debug, Clone, PartialEq)]
pub struct AiFileWithProcessing {
    pub file: AiFile,
    /// The presigned download URL.
    pub url: String,
    pub chunks_count: i64,
    pub processing_status: &'static str,
    /// The valid `chunking` metadata, else `None`.
    pub chunking: Option<Value>,
    pub chunking_outdated: bool,
    /// The effective parameters.
    pub chunking_config: ResolvedAiFileChunkingConfig,
    /// No parameters stored (today's defaults apply).
    pub chunking_config_is_default: bool,
    /// Some chunks have no position yet.
    pub index_outdated: bool,
    /// The valid run plus `stale`, else `None`.
    pub processing_run: Option<Value>,
}

fn stats_of(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, EmbeddingStats>> {
    let mut out: HashMap<i64, EmbeddingStats> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut stmt = conn
        .prepare_cached(
            r#"SELECT "aiFileId", "status", count(*), count("chunkIndex") FROM "AIEmbedding"
               WHERE "aiFileId" IN (SELECT value FROM json_each(?1)) GROUP BY "aiFileId", "status""#,
        )
        .map_err(repo_err)?;
    let rows = stmt
        .query_map([id_list(ids)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(repo_err)?;
    for row in rows {
        let (file_id, status, count, numbered) = row.map_err(repo_err)?;
        let stats = out.entry(file_id).or_default();
        stats.total += count;
        stats.numbered += numbered;
        stats.error |= status == "error";
        stats.in_progress |= status == "pending" || status == "processing";
    }
    Ok(out)
}

/// The read model without the URL (`enrichFile` minus the presign).
fn enrich(file: AiFile, stats: EmbeddingStats, now: i64) -> AiFileWithProcessing {
    let chunking = parse_chunking(file.chunking.as_ref().map(|j| &j.0));
    let processing_run = parse_processing_run(file.processing_run.as_ref().map(|j| &j.0));
    if file.processing_run.is_some() && processing_run.is_none() {
        tracing::warn!(ai_file_id = file.id, "Invalid AI file processing run");
    }
    let config = file.chunking_config.as_ref().map(|j| &j.0);
    AiFileWithProcessing {
        url: String::new(),
        chunks_count: stats.total,
        processing_status: processing_status(&stats),
        chunking_outdated: chunking_outdated(
            chunking.as_ref(),
            stats.total,
            AI_FILE_CHUNKER_VERSIONS,
        ),
        chunking,
        chunking_config: resolve_ai_file_chunking_config(config),
        chunking_config_is_default: config.is_none_or(Value::is_null),
        index_outdated: stats.total > 0 && stats.numbered < stats.total,
        processing_run: processing_run.map(|run| run_with_stale(&run, now)),
        file,
    }
}

/// `uploader.getPresignedDownload` for each file (a failure is a 500, as
/// the original's unhandled rejection).
async fn with_urls(
    files: &Uploader,
    mut rows: Vec<AiFileWithProcessing>,
) -> Result<Vec<AiFileWithProcessing>> {
    for row in &mut rows {
        row.url = files
            .get_presigned_download(&row.file.path, None)
            .await
            .map_err(|e| system_error(&e.to_string(), 500))?;
    }
    Ok(rows)
}

fn read_enriched(
    conn: &Connection,
    filter: &str,
    args: &[&dyn chatbotx_db::rusqlite::ToSql],
) -> Result<Vec<AiFileWithProcessing>> {
    let files: Vec<AiFile> = find_many(conn, filter, args).map_err(repo_err)?;
    let ids: Vec<i64> = files.iter().map(|f| f.id).collect();
    let stats = stats_of(conn, &ids)?;
    let now = now_ms();
    Ok(files
        .into_iter()
        .map(|file| {
            let s = stats.get(&file.id).copied().unwrap_or_default();
            enrich(file, s, now)
        })
        .collect())
}

const NEWEST_FIRST: &str = r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#;

/// `findBy({ where: { id, workspaceId } })`.
pub fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<AiFile>> {
    find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )
    .map_err(repo_err)
}

/// `listWithEmbeddingStatus`: every file, newest first (the private UI).
pub async fn list_with_embedding_status(
    db: &Db,
    files: &Uploader,
    workspace_id: i64,
) -> Result<Vec<AiFileWithProcessing>> {
    let rows = tx::read(db, move |conn| {
        read_enriched(conn, NEWEST_FIRST, &[&workspace_id])
    })
    .await?;
    with_urls(files, rows).await
}

/// One page of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct AiFilePage {
    pub data: Vec<AiFileWithProcessing>,
    pub page_count: i64,
}

/// `listAIFiles`: newest first; without `page` / `perPage` every file and
/// `pageCount: 1`.
pub async fn list(
    db: &Db,
    files: &Uploader,
    workspace_id: i64,
    paging: PaginationInput,
) -> Result<AiFilePage> {
    let (rows, page_count) = tx::read(db, move |conn| {
        if paging.page.is_none() && paging.per_page.is_none() {
            return Ok((read_enriched(conn, NEWEST_FIRST, &[&workspace_id])?, 1));
        }
        let p = get_pagination_with_defaults(paging);
        let rows = read_enriched(
            conn,
            &format!("{NEWEST_FIRST} LIMIT ?2 OFFSET ?3"),
            &[&workspace_id, &p.limit, &p.offset],
        )?;
        let total: i64 = conn
            .query_row(
                r#"SELECT count(*) FROM "AIFile" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        Ok((rows, (total + p.limit - 1) / p.limit))
    })
    .await?;
    Ok(AiFilePage {
        data: with_urls(files, rows).await?,
        page_count,
    })
}

/// `findWithProcessing({ where: { id, workspaceId } })`.
pub async fn find_with_processing(
    db: &Db,
    files: &Uploader,
    workspace_id: i64,
    id: i64,
) -> Result<Option<AiFileWithProcessing>> {
    let rows = tx::read(db, move |conn| {
        read_enriched(
            conn,
            r#"WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &workspace_id],
        )
    })
    .await?;
    Ok(with_urls(files, rows).await?.into_iter().next())
}

/// `assertEmbeddingProviderConfigured`: an OpenAI or Gemini integration.
pub fn assert_embedding_provider_configured(conn: &Connection, workspace_id: i64) -> Result<()> {
    let configured: bool = conn
        .query_row(
            r#"SELECT EXISTS (SELECT 1 FROM "IntegrationOpenai" WHERE "workspaceId" = ?1)
                   OR EXISTS (SELECT 1 FROM "IntegrationGemini" WHERE "workspaceId" = ?1)"#,
            [workspace_id],
            |r| r.get(0),
        )
        .map_err(repo_err)?;
    if configured {
        Ok(())
    } else {
        Err(BusinessError::exception(
            NO_EMBEDDING_PROVIDER,
            "noEmbeddingProvider",
            400,
        ))
    }
}

/// `findManyByIds`: non-numeric ids are dropped (no query when none is left).
pub fn find_many_by_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[String],
) -> Result<Vec<AiFile>> {
    let ids: Vec<i64> = ids
        .iter()
        .filter(|id| is_numeric_id(id))
        .filter_map(|id| id.parse().ok())
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    find_many(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
        &[&workspace_id, &id_list(&ids)],
    )
    .map_err(repo_err)
}

/// `countEmbeddingsByStatus`: `(total, success)` per file.
pub fn count_embeddings_by_status(
    conn: &Connection,
    workspace_id: i64,
    ai_file_ids: &[i64],
) -> Result<HashMap<i64, (i64, i64)>> {
    let mut counts: HashMap<i64, (i64, i64)> = HashMap::new();
    for row in repo::count_by_status(conn, workspace_id, ai_file_ids).map_err(repo_err)? {
        let entry = counts.entry(row.ai_file_id).or_default();
        entry.0 += row.count;
        if row.status == "success" {
            entry.1 += row.count;
        }
    }
    Ok(counts)
}

// ---------------------------------------------------------------------------
// Chunks
// ---------------------------------------------------------------------------

const CURSOR_ENGINE: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// `encodeChunkCursor`: base64url of `{"i":<chunkIndex>,"id":"<id>"}`.
pub fn encode_chunk_cursor(chunk_index: Option<i64>, id: i64) -> String {
    CURSOR_ENGINE.encode(json!({ "i": chunk_index, "id": id.to_string() }).to_string())
}

/// `decodeChunkCursor`: `invalidCursor` (400) unless the cursor is
/// `{ i: int >= 0 | null, id: numeric string }`.
pub fn decode_chunk_cursor(cursor: &str) -> Result<AiFileChunkCursor> {
    let invalid = || BusinessError::exception(INVALID_CURSOR, "invalidCursor", 400);
    let bytes = CURSOR_ENGINE.decode(cursor).map_err(|_| invalid())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let chunk_index = match value.get("i") {
        Some(Value::Null) => None,
        Some(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f >= 0.0 && f <= i64::MAX as f64 => Some(f as i64),
            _ => return Err(invalid()),
        },
        None => return Err(invalid()),
    };
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| is_numeric_id(id))
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    Ok(AiFileChunkCursor { chunk_index, id })
}

/// One page of [`list_chunks`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPage {
    pub data: Vec<AiFileChunkRow>,
    pub next_cursor: Option<String>,
}

/// The input of [`list_chunks`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListChunksInput {
    pub workspace_id: i64,
    pub ai_file_id: i64,
    pub limit: i64,
    /// The opaque `nextCursor` of the previous page.
    pub cursor: Option<String>,
    /// Start the first page at this position (ignored with `cursor`).
    pub from_chunk_index: Option<i64>,
}

/// `listChunks`: the cursor is decoded before any query; a file of another
/// workspace is `AI file not found`.
pub async fn list_chunks(db: &Db, input: ListChunksInput) -> Result<ChunkPage> {
    let cursor = input
        .cursor
        .as_deref()
        .map(decode_chunk_cursor)
        .transpose()?;
    tx::read(db, move |conn| {
        if find_in(conn, input.workspace_id, input.ai_file_id)?.is_none() {
            return Err(BusinessError::not_found("AI file not found"));
        }
        let mut rows = repo::list_chunks(
            conn,
            input.workspace_id,
            input.ai_file_id,
            input.limit,
            cursor,
            input.from_chunk_index,
        )
        .map_err(repo_err)?;
        let has_next = rows.len() as i64 > input.limit;
        rows.truncate(usize::try_from(input.limit).unwrap_or(0));
        let next_cursor = match rows.last() {
            Some(last) if has_next => Some(encode_chunk_cursor(last.chunk_index, last.id)),
            _ => None,
        };
        Ok(ChunkPage {
            data: rows,
            next_cursor,
        })
    })
    .await
}

/// One match of [`find_chunks_containing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkContaining {
    pub chunk_id: i64,
    pub file_id: i64,
    pub file_name: String,
    pub chunk_index: Option<i64>,
}

/// `findChunksContaining`: chunks of the workspace's files among `file_ids`
/// whose text contains `text` (case-insensitive, LIKE wildcards escaped).
/// No query for blank text or when no file belongs to the workspace.
pub fn find_chunks_containing(
    conn: &Connection,
    workspace_id: i64,
    file_ids: &[String],
    text: &str,
    limit: Option<i64>,
) -> Result<Vec<ChunkContaining>> {
    let files = find_many_by_ids(conn, workspace_id, file_ids)?;
    let names: HashMap<i64, String> = files.into_iter().map(|f| (f.id, f.name)).collect();
    let text = text.trim();
    if names.is_empty() || text.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<i64> = names.keys().copied().collect();
    ids.sort_unstable();
    let rows = repo::find_chunks_containing(
        conn,
        workspace_id,
        &ids,
        &format!("%{}%", escape_like_pattern(text)),
        limit.unwrap_or(20),
    )
    .map_err(repo_err)?;
    Ok(rows
        .into_iter()
        .map(|row| ChunkContaining {
            chunk_id: row.id,
            file_name: names.get(&row.ai_file_id).cloned().unwrap_or_default(),
            file_id: row.ai_file_id,
            chunk_index: row.chunk_index,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Processing runs (the worker calls these)
// ---------------------------------------------------------------------------

/// `createProcessingRun`: a new `queued` run.
pub fn new_processing_run(now: i64) -> Value {
    json!({
        "id": create_id(),
        "status": "queued",
        "error": null,
        "queuedAt": iso(now),
        "startedAt": null,
        "heartbeatAt": null,
        "finishedAt": null,
    })
}

/// `markChunked`: the metadata must be valid `aiFileChunking` (else nothing
/// is written).
pub async fn mark_chunked(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    chunking: Value,
) -> Result<()> {
    let chunking = parse_chunking(Some(&chunking))
        .ok_or_else(|| BusinessError::validation("chunking", "Invalid AI file chunking"))?;
    tx::write(db, move |conn| {
        repo::mark_chunked(conn, ai_file_id, workspace_id, &chunking).map_err(repo_err)
    })
    .await
}

/// `markProcessingRunStarted`.
pub async fn mark_processing_run_started(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: String,
) -> Result<bool> {
    let now = iso(now_ms());
    tx::write(db, move |conn| {
        repo::mark_processing_run_started(conn, ai_file_id, workspace_id, &run_id, &now)
            .map_err(repo_err)
    })
    .await
}

/// `refreshProcessingRunLease`.
pub async fn refresh_processing_run_lease(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: String,
) -> Result<bool> {
    let now = iso(now_ms());
    tx::write(db, move |conn| {
        repo::refresh_processing_run_lease(conn, ai_file_id, workspace_id, &run_id, &now)
            .map_err(repo_err)
    })
    .await
}

/// `markProcessingRunSucceeded`.
pub async fn mark_processing_run_succeeded(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: String,
) -> Result<bool> {
    let now = iso(now_ms());
    tx::write(db, move |conn| {
        repo::finish_processing_run(
            conn,
            ai_file_id,
            workspace_id,
            &run_id,
            "success",
            None,
            &now,
        )
        .map_err(repo_err)
    })
    .await
}

/// `markEmbeddingFailed`: one chunk used its last attempt.
pub async fn mark_embedding_failed(db: &Db, embedding_id: i64, workspace_id: i64) -> Result<bool> {
    tx::write(db, move |conn| {
        repo::mark_embedding_failed(conn, embedding_id, workspace_id).map_err(repo_err)
    })
    .await
}

/// `markProcessingRunFailed`: `code` must be a run error code (else nothing
/// is written).
pub async fn mark_processing_run_failed(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: String,
    code: &str,
) -> Result<bool> {
    if !AI_FILE_PROCESSING_RUN_ERROR_CODES.contains(&code) {
        return Err(BusinessError::validation(
            "code",
            format!("Invalid AI file processing run error code: {code}"),
        ));
    }
    let code = code.to_owned();
    let now = iso(now_ms());
    tx::write(db, move |conn| {
        repo::finish_processing_run(
            conn,
            ai_file_id,
            workspace_id,
            &run_id,
            "error",
            Some(&code),
            &now,
        )
        .map_err(repo_err)
    })
    .await
}

// ---------------------------------------------------------------------------
// Create / reindex / delete
// ---------------------------------------------------------------------------

/// `CreateAIFileInput`.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateAiFileInput {
    /// An object the caller uploaded under the workspace's AI file prefix.
    Path {
        name: String,
        path: String,
        mime_type: String,
        size: i64,
    },
    /// A multipart upload.
    File {
        name: Option<String>,
        file: InputFile,
    },
    /// A URL the server downloads.
    Url { name: Option<String>, url: String },
}

struct Resolved {
    name: String,
    path: String,
    mime_type: String,
    size: i64,
}

fn url_guard() -> ValidateUrl {
    Arc::new(|candidate: String| {
        Box::pin(async move {
            // The guard's message echoes the URL; it never reaches the caller.
            assert_public_url(&candidate, "AI file URL")
                .await
                .map_err(|_| FilesError::UploadValidation("The provided URL is not allowed".into()))
        })
    })
}

async fn resolve_upload(
    deps: &AiFileDeps<'_>,
    workspace_id: i64,
    input: CreateAiFileInput,
) -> Result<Resolved, FilesError> {
    let prefix = format!("workspaces/{workspace_id}/ai-files/");
    let path = format!("{prefix}{}", create_id());
    let size = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    match input {
        CreateAiFileInput::Path {
            name,
            path,
            mime_type,
            size,
        } => {
            // A member must not point a file at another workspace's object.
            if !path.starts_with(&prefix) || path.contains("..") {
                return Err(FilesError::UploadValidation(
                    "The uploaded file path is not allowed".into(),
                ));
            }
            Ok(Resolved {
                name,
                path,
                mime_type,
                size,
            })
        }
        CreateAiFileInput::File { name, file } => {
            let uploaded = upload_file(deps.files.as_ref(), &file, &path, Acl::Private).await?;
            Ok(Resolved {
                name: name.unwrap_or(uploaded.name),
                path,
                mime_type: uploaded.mime_type,
                size: size(uploaded.size),
            })
        }
        CreateAiFileInput::Url { name, url } => {
            let fetcher = match &deps.fetcher {
                Some(fetcher) => Arc::clone(fetcher),
                None => Arc::new(ReqwestFetcher::new()?),
            };
            let putter: Arc<dyn ObjectPutter> = Arc::clone(deps.files) as Arc<dyn ObjectPutter>;
            let upload = UploadDeps { fetcher, putter };
            let uploaded = upload_file_from_url(
                &upload,
                &url,
                &path,
                UploadFromUrlOptions {
                    acl: Acl::Private,
                    max_bytes: Some(AI_FILE_MAX_UPLOAD_BYTES),
                    validate_url: Some(url_guard()),
                    cancel: None,
                },
            )
            .await?;
            Ok(Resolved {
                name: name.unwrap_or(uploaded.name),
                path,
                mime_type: uploaded.mime_type,
                size: size(uploaded.size),
            })
        }
    }
}

/// `heavyQueue.add(processAIFile, { type, data: { aiFileId, runId } }, ...)`.
async fn enqueue_processing(
    jobs: Option<&Arc<dyn Enqueuer>>,
    ai_file_id: i64,
    run_id: &str,
    job_id: String,
) -> std::result::Result<(), String> {
    let Some(jobs) = jobs else {
        return Err("the job queue is not configured".into());
    };
    let name = HeavyJobAction::ProcessAiFile.as_str();
    jobs.add(
        QueueName::Heavy,
        name,
        json!({ "type": name, "data": { "aiFileId": ai_file_id.to_string(), "runId": run_id } }),
        JobOptions {
            job_id: Some(job_id),
            ..heavy_job_options(HeavyJobAction::ProcessAiFile)
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

async fn record_enqueue_failure(
    db: &Db,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: &str,
    log: &str,
) -> bool {
    match mark_processing_run_failed(
        db,
        ai_file_id,
        workspace_id,
        run_id.to_owned(),
        "enqueueFailed",
    )
    .await
    {
        Ok(marked) => marked,
        Err(error) => {
            tracing::error!(
                err = %error.message(),
                ai_file_id,
                workspace_id,
                run_id,
                "{log}"
            );
            false
        }
    }
}

/// `create`: the provider check, the upload, the row with a `queued` run,
/// the `processAIFile` job (`heavy-ai-file-<id>`) and the audit. A failed
/// enqueue still returns the stored file, with the run marked
/// `enqueueFailed` (the caller reindexes).
pub async fn create(
    deps: &AiFileDeps<'_>,
    workspace_id: i64,
    input: CreateAiFileInput,
) -> Result<AiFileWithProcessing> {
    tx::read(deps.db, move |conn| {
        assert_embedding_provider_configured(conn, workspace_id)
    })
    .await?;
    let resolved = match resolve_upload(deps, workspace_id, input).await {
        Ok(resolved) => resolved,
        Err(FilesError::UploadValidation(message)) => {
            return Err(BusinessError::exception(message, "businessError", 400));
        }
        Err(error) => {
            tracing::error!(err = %error, "Failed to store AI file");
            return Err(system_error(
                "Failed to store AI file. Please try again later.",
                502,
            ));
        }
    };
    let run = new_processing_run(now_ms());
    let run_id = run["id"].as_str().unwrap_or_default().to_owned();
    let created = tx::write(deps.db, move |conn| {
        let id = new_id();
        conn.execute(
            r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "processingRun", "workspaceId")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                id,
                resolved.name,
                resolved.path,
                resolved.size,
                resolved.mime_type,
                run.to_string(),
                workspace_id
            ],
        )
        .map_err(repo_err)?;
        find_in(conn, workspace_id, id)?.ok_or_else(|| BusinessError::not_found("AI file not found"))
    })
    .await?;
    let mut returned_run = created
        .processing_run
        .as_ref()
        .map_or(Value::Null, |j| j.0.clone());
    if let Err(error) = enqueue_processing(
        deps.jobs,
        created.id,
        &run_id,
        format!("heavy-ai-file-{}", created.id),
    )
    .await
    {
        // The row exists, so this is still a 201: the caller recovers through reindex.
        let finished_at = iso(now_ms());
        if record_enqueue_failure(
            deps.db,
            created.id,
            workspace_id,
            &run_id,
            "Failed to record AI file enqueue failure",
        )
        .await
        {
            returned_run["status"] = json!("error");
            returned_run["error"] = json!("enqueueFailed");
            returned_run["finishedAt"] = json!(finished_at);
        }
        tracing::error!(
            err = %error,
            ai_file_id = created.id,
            workspace_id,
            run_id,
            "Failed to enqueue AI file processing"
        );
    }
    deps.audit
        .audit(
            "create",
            &format!("created a new Knowledge (#{})", created.id),
        )
        .await;
    let url = deps
        .files
        .get_presigned_download(&created.path, None)
        .await
        .map_err(|e| system_error(&e.to_string(), 500))?;
    let mut run = returned_run;
    if let Value::Object(map) = &mut run {
        map.insert("stale".into(), Value::Bool(false));
    }
    Ok(AiFileWithProcessing {
        url,
        chunks_count: 0,
        processing_status: "pending",
        chunking: None,
        chunking_outdated: false,
        chunking_config: resolve_ai_file_chunking_config(None),
        chunking_config_is_default: true,
        index_outdated: false,
        processing_run: Some(run),
        file: created,
    })
}

/// The `chunkingConfig` of [`reindex`]: `Keep` = omitted, `Reset` = `null`,
/// `Set` = new parameters (validated, stored normalized).
#[derive(Debug, Clone, PartialEq)]
pub enum ReindexConfig {
    Keep,
    Reset,
    Set(Value),
}

/// The result of [`reindex`].
#[derive(Debug, Clone, PartialEq)]
pub struct Reindexed {
    pub file: AiFileWithProcessing,
    /// An active run owned the file; nothing new was queued.
    pub coalesced: bool,
}

/// `reindex`: validates the parameters, claims a new run (unless one is
/// active and not stale), enqueues `processAIFile`
/// (`heavy-ai-file-<id>-reindex-<runId>`) and audits. With parameters, an
/// active run is a 409 `conflict` instead of a coalesce.
pub async fn reindex(
    deps: &AiFileDeps<'_>,
    workspace_id: i64,
    id: i64,
    config: ReindexConfig,
) -> Result<Reindexed> {
    let update = match &config {
        ReindexConfig::Keep => ChunkingConfigUpdate::Keep,
        ReindexConfig::Reset => ChunkingConfigUpdate::Reset,
        ReindexConfig::Set(value) => {
            let parsed = parse_ai_file_chunking_config(value).map_err(|e| {
                BusinessError::validation(
                    "chunkingConfig",
                    e.issues
                        .first()
                        .map_or("Invalid chunking settings".to_owned(), |i| {
                            i.message.clone()
                        }),
                )
            })?;
            let normalized = resolve_ai_file_chunking_config(Some(&json!(parsed)));
            ChunkingConfigUpdate::Set(json!(normalized))
        }
    };
    let current = find_with_processing(deps.db, deps.files, workspace_id, id)
        .await?
        .ok_or_else(|| BusinessError::not_found("AI file not found"))?;
    let run = new_processing_run(now_ms());
    let run_id = run["id"].as_str().unwrap_or_default().to_owned();
    let stale_before = iso(now_ms() - AI_FILE_PROCESSING_RUN_STALE_MS);
    let claimed = tx::write(deps.db, move |conn| {
        assert_embedding_provider_configured(conn, workspace_id)?;
        repo::claim_processing_run(conn, id, workspace_id, &run, &stale_before, &update)
            .map_err(repo_err)
    })
    .await?;
    if !claimed {
        // Parameters the caller chose must not vanish behind a coalesce.
        if config != ReindexConfig::Keep {
            return Err(BusinessError::exception(REINDEX_CONFLICT, "conflict", 409));
        }
        let file = find_with_processing(deps.db, deps.files, workspace_id, id).await?;
        return Ok(Reindexed {
            file: file.unwrap_or(current),
            coalesced: true,
        });
    }
    if let Err(error) = enqueue_processing(
        deps.jobs,
        id,
        &run_id,
        ai_file_reindex_job_id(&id.to_string(), &run_id),
    )
    .await
    {
        record_enqueue_failure(
            deps.db,
            id,
            workspace_id,
            &run_id,
            "Failed to record AI file reindex enqueue failure",
        )
        .await;
        tracing::error!(
            err = %error,
            ai_file_id = id,
            workspace_id,
            run_id,
            "Failed to enqueue AI file reindex"
        );
        return Err(system_error(
            "Failed to queue AI file processing. Please try again later.",
            502,
        ));
    }
    deps.audit
        .audit("reindex", &format!("reindexed a Knowledge (#{id})"))
        .await;
    tracing::info!(workspace_id, ai_file_id = id, run_id, "Reindexed AI file");
    let file = find_with_processing(deps.db, deps.files, workspace_id, id)
        .await?
        .unwrap_or(current);
    Ok(Reindexed {
        file,
        coalesced: false,
    })
}

/// `delete`: `AIFile with id <id> not found` (404) for a file of another
/// workspace; then the stored object and the row (its chunks cascade) and
/// the audit. A storage / database failure after that is logged, not
/// returned (the original's best-effort delete).
pub async fn delete(deps: &AiFileDeps<'_>, workspace_id: i64, id: i64) -> Result<()> {
    let file = tx::read(deps.db, move |conn| find_in(conn, workspace_id, id))
        .await?
        .ok_or_else(|| BusinessError::not_found(format!("AIFile with id {id} not found")))?;
    let result = async {
        deps.files
            .delete_object(&file.path)
            .await
            .map_err(|e| e.to_string())?;
        tx::write(deps.db, move |conn| delete_in(conn, workspace_id, &[id]))
            .await
            .map_err(|e| e.message())
    }
    .await;
    match result {
        Ok(_) => {
            deps.audit
                .audit("delete", &format!("deleted a Knowledge (#{id})"))
                .await;
        }
        Err(error) => tracing::warn!(err = %error, "Failed to delete AI file (#{id})"),
    }
    Ok(())
}

/// `deleteInTx`: deletes the workspace's rows among `ids` (chunks cascade;
/// the stored objects are not touched). Returns the deleted ids.
pub fn delete_in(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    crate::template::guard::assert_deletable(conn, workspace_id, "aiFile", ids)?;
    let mut stmt = conn
        .prepare_cached(
            r#"DELETE FROM "AIFile" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) RETURNING "id""#,
        )
        .map_err(repo_err)?;
    let rows = stmt
        .query_map(params![workspace_id, id_list(ids)], |r| r.get(0))
        .map_err(repo_err)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<i64>>>()
        .map_err(repo_err)?;
    Ok(rows)
}
