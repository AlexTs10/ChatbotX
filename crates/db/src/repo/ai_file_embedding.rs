//! Port of `repositories/ai-file-embedding/repository.ts`.
//!
//! `processingRun` / `chunking` / `chunkingConfig` are JSON objects (`partials/ai.ts`
//! types); this layer takes and stores them as `serde_json::Value`. The run patches use
//! `json_set` (keeps explicit `null` values, as the Postgres `||` merge does).

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::Value as JsonValue;

use super::models::AiFile;
use super::sql::Sql;
use super::{Error, Result, id_list, now_ms};

/// `AiFileEmbeddingChunk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFileEmbeddingChunk {
    pub chunk_index: i64,
    pub content: String,
    pub id: i64,
}

/// `AiFileChunkCursor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiFileChunkCursor {
    pub chunk_index: Option<i64>,
    pub id: i64,
}

/// `AiFileChunkRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFileChunkRow {
    pub id: i64,
    pub content: String,
    pub status: String,
    pub created_at: i64,
    pub chunk_index: Option<i64>,
}

/// One row of [`vector_search`].
#[derive(Debug, Clone, PartialEq)]
pub struct AiFileVectorSearchRow {
    pub id: i64,
    pub content: String,
    pub ai_file_id: i64,
    pub chunk_index: Option<i64>,
    pub similarity: Option<f64>,
}

/// One row of [`find_chunks_containing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkMatch {
    pub id: i64,
    pub ai_file_id: i64,
    pub chunk_index: Option<i64>,
}

/// One row of [`count_by_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusCount {
    pub ai_file_id: i64,
    pub status: String,
    pub count: i64,
}

/// `findFileOrFail`: the file by id, else `AI file not found`.
pub fn find_file_or_fail(conn: &Connection, ai_file_id: i64) -> Result<AiFile> {
    super::find_one(conn, "\"id\" = ?1", &[&ai_file_id])?
        .ok_or_else(|| Error::msg("AI file not found"))
}

/// `markChunked`: writes only `chunking` of the file in its own workspace.
pub fn mark_chunked(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    chunking: &JsonValue,
) -> Result<()> {
    todo!("P3.2");
    conn.execute(
        "UPDATE \"AIFile\" SET \"chunking\" = ?1 WHERE \"id\" = ?2 AND \"workspaceId\" = ?3",
        rusqlite::params![chunking.to_string(), ai_file_id, workspace_id],
    )?;
    Ok(())
}

/// `vectorSearch`: the `limit` nearest `success` chunks (cosine) of `file_ids` in the
/// workspace. No query for an empty `file_ids`.
pub fn vector_search(
    conn: &Connection,
    workspace_id: i64,
    file_ids: &[i64],
    query_vector: &[f32],
    limit: i64,
) -> Result<Vec<AiFileVectorSearchRow>> {
    todo!("P3.2");
    if file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let vector = format!(
        "[{}]",
        query_vector
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"content\", \"aiFileId\", \"chunkIndex\", 1 - vector_distance_cos(\"embedding\", vector32(?1)) \
         FROM \"AIEmbedding\" INDEXED BY \"AIEmbedding_aiFileId_chunkIndex_idx\" \
         WHERE \"workspaceId\" = ?2 AND \"aiFileId\" IN (SELECT value FROM json_each(?3)) \
           AND \"status\" = 'success' AND \"embedding\" IS NOT NULL \
         ORDER BY vector_distance_cos(\"embedding\", vector32(?1)) LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![vector, workspace_id, id_list(file_ids), limit],
            |row| {
                Ok(AiFileVectorSearchRow {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    ai_file_id: row.get(2)?,
                    chunk_index: row.get(3)?,
                    similarity: row.get(4)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listChunks`: one page (`limit + 1` rows, so the caller can see another page) of the
/// file's chunks in document order (`chunkIndex ASC NULLS LAST, id`), keyset after
/// `cursor`, or from `from_chunk_index` (ignored with a cursor; unnumbered rows stay
/// reachable).
pub fn list_chunks(
    conn: &Connection,
    workspace_id: i64,
    ai_file_id: i64,
    limit: i64,
    cursor: Option<AiFileChunkCursor>,
    from_chunk_index: Option<i64>,
) -> Result<Vec<AiFileChunkRow>> {
    todo!("P3.2");
    let extra = match (cursor, from_chunk_index) {
        (Some(AiFileChunkCursor {
            chunk_index: None,
            id,
        }), _) => Some(Sql::bind("\"chunkIndex\" IS NULL AND \"id\" > ", id)),
        (Some(AiFileChunkCursor {
            chunk_index: Some(ci),
            id,
        }), _) => Some(Sql::new(
            "\"chunkIndex\" > ? OR (\"chunkIndex\" = ? AND \"id\" > ?) OR \"chunkIndex\" IS NULL",
            vec![Value::Integer(ci), Value::Integer(ci), Value::Integer(id)],
        )),
        (None, Some(from)) => Some(Sql::bind("\"chunkIndex\" IS NULL OR \"chunkIndex\" >= ", from)),
        (None, None) => None,
    };
    let mut q = Sql::raw(
        "SELECT \"id\", \"content\", \"status\", \"createdAt\", \"chunkIndex\" FROM \"AIEmbedding\" WHERE ",
    );
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        Some(Sql::bind("\"aiFileId\" = ", ai_file_id)),
        extra,
    ]));
    q.push(" ORDER BY \"chunkIndex\" IS NULL, \"chunkIndex\" ASC, \"id\" ASC LIMIT ");
    q.push_bind(limit + 1);
    q.query_map(conn, |row| {
        Ok(AiFileChunkRow {
            id: row.get(0)?,
            content: row.get(1)?,
            status: row.get(2)?,
            created_at: row.get(3)?,
            chunk_index: row.get(4)?,
        })
    })
}

/// `findChunksContaining`: chunks of `file_ids` in the workspace whose content matches
/// the (already escaped, `\`) case-insensitive LIKE `pattern`, in document order. No
/// query for an empty `file_ids`.
pub fn find_chunks_containing(
    conn: &Connection,
    workspace_id: i64,
    file_ids: &[i64],
    pattern: &str,
    limit: i64,
) -> Result<Vec<ChunkMatch>> {
    todo!("P3.2");
    if file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"aiFileId\", \"chunkIndex\" FROM \"AIEmbedding\" \
         WHERE \"workspaceId\" = ?1 AND \"aiFileId\" IN (SELECT value FROM json_each(?2)) AND ilike(\"content\", ?3) \
         ORDER BY \"chunkIndex\" IS NULL, \"chunkIndex\" ASC, \"id\" ASC LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, id_list(file_ids), pattern, limit],
            |row| {
                Ok(ChunkMatch {
                    id: row.get(0)?,
                    ai_file_id: row.get(1)?,
                    chunk_index: row.get(2)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `countByStatus`: chunk counts per `(aiFileId, status)`. Empty for no files.
pub fn count_by_status(
    conn: &Connection,
    workspace_id: i64,
    ai_file_ids: &[i64],
) -> Result<Vec<StatusCount>> {
    if ai_file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"aiFileId\", \"status\", count(*) FROM \"AIEmbedding\" \
         WHERE \"workspaceId\" = ?1 AND \"aiFileId\" IN (SELECT value FROM json_each(?2)) GROUP BY \"aiFileId\", \"status\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(ai_file_ids)], |row| {
            Ok(StatusCount {
                ai_file_id: row.get(0)?,
                status: row.get(1)?,
                count: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The `chunkingConfig` argument of [`claim_processing_run`]: `Keep` = TS `undefined`
/// (column untouched), `Reset` = `null`, `Set` = a new value.
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkingConfigUpdate {
    Keep,
    Reset,
    Set(JsonValue),
}

/// `claimProcessingRun`: writes `run` (and the chunking config update) when the file has
/// no active run or its lease (`heartbeatAt`, else `queuedAt`, ISO text) is older than
/// `stale_before_iso`. Workspace-scoped. `true` when claimed.
pub fn claim_processing_run(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    run: &JsonValue,
    stale_before_iso: &str,
    chunking_config: &ChunkingConfigUpdate,
) -> Result<bool> {
    todo!("P3.2");
    let mut q = Sql::bind("UPDATE \"AIFile\" SET \"processingRun\" = ", run.to_string());
    match chunking_config {
        ChunkingConfigUpdate::Keep => {}
        ChunkingConfigUpdate::Reset => {
            q.push(", \"chunkingConfig\" = NULL");
        }
        ChunkingConfigUpdate::Set(config) => {
            q.push(", \"chunkingConfig\" = ");
            q.push_bind(config.to_string());
        }
    }
    q.push(", \"updatedAt\" = ");
    q.push_bind(now_ms());
    q.push(" WHERE \"id\" = ");
    q.push_bind(ai_file_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(
        " AND (COALESCE(\"processingRun\"->>'status', '') NOT IN ('queued', 'processing') \
         OR COALESCE(\"processingRun\"->>'heartbeatAt', \"processingRun\"->>'queuedAt', '') < ",
    );
    q.push_bind(stale_before_iso.to_owned());
    q.push(")");
    Ok(q.execute(conn)? > 0)
}

/// `updateProcessingRun`: sets the `patch` keys of the run when the run id matches and its
/// status is one of `active_statuses`.
fn update_processing_run(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: &str,
    active_statuses: &[&str],
    patch: Sql,
) -> Result<bool> {
    let mut q = Sql::raw("UPDATE \"AIFile\" SET \"processingRun\" = json_set(\"processingRun\", ");
    q.append(patch);
    q.push(") WHERE \"id\" = ");
    q.push_bind(ai_file_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" AND \"processingRun\"->>'id' = ");
    q.push_bind(run_id.to_owned());
    q.push(" AND \"processingRun\"->>'status' IN (");
    for (i, s) in active_statuses.iter().enumerate() {
        if i > 0 {
            q.push(", ");
        }
        q.push_bind((*s).to_owned());
    }
    q.push(")");
    Ok(q.execute(conn)? > 0)
}

fn text_or_null(v: Option<&str>) -> Value {
    v.map_or(Value::Null, |s| Value::Text(s.to_owned()))
}

/// `markProcessingRunStarted`: `status = processing`, the first `startedAt` is kept,
/// `heartbeatAt = now` (for a queued or processing run of `run_id`).
pub fn mark_processing_run_started(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: &str,
    now: &str,
) -> Result<bool> {
    todo!("P3.2");
    update_processing_run(
        conn,
        ai_file_id,
        workspace_id,
        run_id,
        &["queued", "processing"],
        Sql::new(
            "'$.status', 'processing', '$.startedAt', COALESCE(\"processingRun\"->>'startedAt', ?), '$.heartbeatAt', ?",
            vec![Value::Text(now.to_owned()), Value::Text(now.to_owned())],
        ),
    )
}

/// `refreshProcessingRunLease`: `heartbeatAt` of a processing run of `run_id`.
pub fn refresh_processing_run_lease(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: &str,
    heartbeat_at: &str,
) -> Result<bool> {
    todo!("P3.2");
    update_processing_run(
        conn,
        ai_file_id,
        workspace_id,
        run_id,
        &["processing"],
        Sql::bind("'$.heartbeatAt', ", heartbeat_at.to_owned()),
    )
}

/// `finishProcessingRun`: `status` (`success`/`error`), `error`, `finishedAt` of a queued
/// or processing run of `run_id`.
#[allow(clippy::too_many_arguments)]
pub fn finish_processing_run(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    run_id: &str,
    status: &str,
    error: Option<&str>,
    finished_at: &str,
) -> Result<bool> {
    todo!("P3.2");
    update_processing_run(
        conn,
        ai_file_id,
        workspace_id,
        run_id,
        &["queued", "processing"],
        Sql::new(
            "'$.status', ?, '$.error', ?, '$.finishedAt', ?",
            vec![
                Value::Text(status.to_owned()),
                text_or_null(error),
                Value::Text(finished_at.to_owned()),
            ],
        ),
    )
}

/// `markEmbeddingFailed`: a pending chunk of the workspace becomes `error`.
pub fn mark_embedding_failed(conn: &Connection, embedding_id: i64, workspace_id: i64) -> Result<bool> {
    todo!("P3.2");
    let n = conn.execute(
        "UPDATE \"AIEmbedding\" SET \"status\" = 'error', \"updatedAt\" = ?3 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'pending'",
        rusqlite::params![embedding_id, workspace_id, now_ms()],
    )?;
    Ok(n > 0)
}

/// `reconcilePendingChunks` (one write transaction): deletes the file's chunks that are
/// not in `chunks`, inserts the new ones as `pending` (a kept row only gains a missing
/// `chunkIndex`), resets kept `error` chunks to `pending`, and returns the pending ids.
/// An empty `chunks` is an error and touches nothing.
pub fn reconcile_pending_chunks(
    conn: &Connection,
    ai_file_id: i64,
    workspace_id: i64,
    chunks: &[AiFileEmbeddingChunk],
) -> Result<Vec<i64>> {
    todo!("P3.2");
    if chunks.is_empty() {
        return Err(Error::msg("reconcilePendingChunks requires at least one chunk"));
    }
    let ids: Vec<i64> = chunks.iter().map(|c| c.id).collect();
    let ids_json = id_list(&ids);
    conn.execute(
        "DELETE FROM \"AIEmbedding\" WHERE \"aiFileId\" = ?1 AND \"workspaceId\" = ?2 \
         AND \"id\" NOT IN (SELECT value FROM json_each(?3))",
        rusqlite::params![ai_file_id, workspace_id, ids_json],
    )?;
    let now = now_ms();
    {
        let mut insert = conn.prepare_cached(
            "INSERT INTO \"AIEmbedding\" (\"id\", \"createdAt\", \"updatedAt\", \"aiFileId\", \"chunkIndex\", \"content\", \"status\", \"workspaceId\") \
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'pending', ?6) \
             ON CONFLICT (\"id\") DO UPDATE SET \"chunkIndex\" = excluded.\"chunkIndex\" WHERE \"AIEmbedding\".\"chunkIndex\" IS NULL",
        )?;
        for c in chunks {
            insert.execute(rusqlite::params![
                c.id,
                now,
                ai_file_id,
                c.chunk_index,
                c.content,
                workspace_id
            ])?;
        }
    }
    conn.execute(
        "UPDATE \"AIEmbedding\" SET \"status\" = 'pending' WHERE \"aiFileId\" = ?1 AND \"workspaceId\" = ?2 \
         AND \"id\" IN (SELECT value FROM json_each(?3)) AND \"status\" = 'error'",
        rusqlite::params![ai_file_id, workspace_id, ids_json],
    )?;
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"AIEmbedding\" WHERE \"aiFileId\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'pending'",
    )?;
    let pending = stmt
        .query_map(rusqlite::params![ai_file_id, workspace_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(pending)
}
