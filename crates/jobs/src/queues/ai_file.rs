//! AI file job ids (`queues/heavy`: `buildAIFileReindexJobId`,
//! `buildAIFileEmbeddingJobId`).

/// `heavy-ai-file-<aiFileId>-reindex-<runId>`.
pub fn ai_file_reindex_job_id(ai_file_id: &str, run_id: &str) -> String {
    format!("heavy-ai-file-{ai_file_id}-reindex-{run_id}")
}

/// `ai-file-embedding-<aiFileId>-<embeddingId>[-r<runId>]`.
pub fn ai_file_embedding_job_id(
    ai_file_id: &str,
    embedding_id: &str,
    run_id: Option<&str>,
) -> String {
    match run_id {
        Some(run) => format!("ai-file-embedding-{ai_file_id}-{embedding_id}-r{run}"),
        None => format!("ai-file-embedding-{ai_file_id}-{embedding_id}"),
    }
}
