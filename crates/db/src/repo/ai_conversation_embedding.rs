//! Port of `repositories/ai-conversation-embedding/repository.ts`.

use rusqlite::Connection;

use super::models::AiConversationEmbedding;
use super::{Result, now_ms};

/// One chunk of [`replace_for_source`] (`EmbeddingChunkInput`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingChunkInput {
    pub chunk_index: i64,
    pub content: String,
    pub conversation_id: i64,
    pub id: i64,
    pub source_id: i64,
    pub workspace_id: i64,
}

/// `VectorSearchRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchRow {
    pub chunk_index: Option<i64>,
    pub content: String,
    pub similarity: Option<f64>,
}

/// `findManyBySource`: the `success` chunks of the source in the workspace, by
/// `chunkIndex`, at most `limit`.
pub fn find_many_by_source(
    conn: &Connection,
    source_id: i64,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<AiConversationEmbedding>> {
    todo!("P3.2");
    super::find_many(
        conn,
        "WHERE \"sourceId\" = ?1 AND \"workspaceId\" = ?2 AND \"status\" = 'success' ORDER BY \"chunkIndex\" ASC LIMIT ?3",
        &[&source_id, &workspace_id, &limit],
    )
}

/// `vectorSearch`: the `topK` nearest `success` chunks (cosine) of the source in the
/// workspace; `query_vector` is the `[a,b,...]` text. Blank contents are skipped and the
/// content is trimmed.
pub fn vector_search(
    conn: &Connection,
    query_vector: &str,
    source_id: i64,
    top_k: i64,
    workspace_id: i64,
) -> Result<Vec<VectorSearchRow>> {
    todo!("P3.2");
    let mut stmt = conn.prepare_cached(
        "SELECT \"chunkIndex\", \"content\", 1 - vector_distance_cos(\"embedding\", vector32(?1)) AS similarity \
         FROM \"AIConversationEmbedding\" \
         WHERE \"sourceId\" = ?2 AND \"workspaceId\" = ?3 AND \"status\" = 'success' AND \"embedding\" IS NOT NULL \
         ORDER BY vector_distance_cos(\"embedding\", vector32(?1)) LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![query_vector, source_id, workspace_id, top_k], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<f64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(chunk_index, content, similarity)| {
            let content = content.trim().to_owned();
            if content.is_empty() {
                return None;
            }
            Some(VectorSearchRow {
                chunk_index,
                content,
                similarity: similarity.filter(|s| s.is_finite()),
            })
        })
        .collect())
}

/// `replaceForSource`: deletes every chunk of the source and inserts `chunks` as
/// `pending` (run inside one write transaction). Returns the new ids.
pub fn replace_for_source(
    conn: &Connection,
    source_id: i64,
    chunks: &[EmbeddingChunkInput],
) -> Result<Vec<i64>> {
    conn.execute(
        "DELETE FROM \"AIConversationEmbedding\" WHERE \"sourceId\" = ?1",
        [source_id],
    )?;
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"AIConversationEmbedding\" (\"id\", \"createdAt\", \"updatedAt\", \"sourceId\", \"workspaceId\", \"conversationId\", \"chunkIndex\", \"content\", \"status\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
    )?;
    let mut ids = Vec::with_capacity(chunks.len());
    for c in chunks {
        stmt.execute(rusqlite::params![
            c.id,
            now,
            c.source_id,
            c.workspace_id,
            c.conversation_id,
            c.chunk_index,
            c.content
        ])?;
        ids.push(c.id);
    }
    Ok(ids)
}
