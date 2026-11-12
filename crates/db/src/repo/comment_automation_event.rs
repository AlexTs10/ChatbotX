//! Port of `repositories/comment-automation-event/repository.ts`.

use super::Result;
use super::chunked_purge::{ChunkedPurgeOptions, ChunkedPurgeStopReason, chunked_purge};
use super::sql::Sql;
use crate::Db;

/// `PurgeCommentAutomationEventsOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeOptions {
    pub retention_days: i64,
    pub chunk_size: i64,
    pub inter_chunk_delay_ms: u64,
    pub max_chunks: i64,
    pub max_run_duration_ms: Option<i64>,
}

/// `purgeFailedCommentAutomationEvents`: chunked delete of `failed` rows created more
/// than `retention_days` before `now`, oldest first.
pub async fn purge_failed_comment_automation_events(db: &Db, options: PurgeOptions, now: i64) -> Result<(i64, ChunkedPurgeStopReason)> {
    todo!("P3.2");
    let cutoff = now - options.retention_days * 86_400_000;
    chunked_purge(
        db,
        &ChunkedPurgeOptions {
            table: "CommentAutomationEvent".into(),
            filter: Sql::bind("\"status\" = 'failed' AND \"createdAt\" < ", cutoff),
            order_by: "createdAt".into(),
            chunk_size: options.chunk_size,
            inter_chunk_delay_ms: options.inter_chunk_delay_ms,
            max_chunks: options.max_chunks,
            max_run_duration_ms: options.max_run_duration_ms,
        },
    )
    .await
}
