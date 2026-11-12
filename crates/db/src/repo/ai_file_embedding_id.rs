//! Port of `repositories/ai-file-embedding/embedding-id.ts`.

use sha2::{Digest, Sha256};

/// `createAIFileEmbeddingId`: the deterministic chunk id: the first 15 hex digits of
/// `sha256(JSON.stringify({aiFileId, chunkIndex, content}))` as a positive integer
/// (`1` for a zero digest prefix).
pub fn create_ai_file_embedding_id(ai_file_id: &str, chunk_index: i64, content: &str) -> i64 {
    todo!("P3.2");
    let json = format!(
        "{{\"aiFileId\":{},\"chunkIndex\":{},\"content\":{}}}",
        serde_json::Value::from(ai_file_id),
        chunk_index,
        serde_json::Value::from(content)
    );
    let digest = Sha256::digest(json.as_bytes());
    // 15 hex digits = 60 bits: the first 7 bytes and the high nibble of the 8th.
    let mut id: u64 = 0;
    for b in &digest[..7] {
        id = (id << 8) | u64::from(*b);
    }
    id = (id << 4) | u64::from(digest[7] >> 4);
    if id == 0 { 1 } else { i64::try_from(id).unwrap_or(1) }
}
