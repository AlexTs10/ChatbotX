//! Port of `repositories/broadcast-purge.ts`: the chunked recipient purge and the helpers
//! of the `schedule:purge-broadcasts` handler. Each chunk is its own short write
//! transaction, capped by a statement timeout at the time left in the run.

use std::future::Future;

use rusqlite::Connection;

use super::{Result, is_statement_timeout_error, now_ms};
use crate::{Db, StatementTimeout};

/// `DEFAULT_PURGE_MAX_CHUNKS`.
pub const DEFAULT_PURGE_MAX_CHUNKS: i64 = 5000;

/// `PurgeStopReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeStopReason {
    Drained,
    Deadline,
    ChunkCap,
}

/// The result of a purge run (`{ deleted, stopReason }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub deleted: i64,
    pub stop_reason: PurgeStopReason,
}

/// `purgeBroadcastRecipients` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeInput {
    pub broadcast_id: i64,
    pub chunk_size: i64,
    pub inter_chunk_delay_ms: u64,
    pub max_run_duration_ms: i64,
    /// `None` = [`DEFAULT_PURGE_MAX_CHUNKS`].
    pub max_chunks: Option<i64>,
    pub only_while_draft: bool,
}

/// `listPurgeableBroadcasts`: soft-deleted, non-sending broadcasts, oldest deleted first.
pub fn list_purgeable_broadcasts(conn: &Connection, limit: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"Broadcast\" WHERE \"deletedAt\" IS NOT NULL AND \"status\" <> 'sending' ORDER BY \"deletedAt\" ASC, \"id\" ASC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// One chunk: deletes up to `chunk_size` recipients (by `contactId`) of the broadcast; with
/// `only_while_draft` only while the broadcast is still a draft (same statement).
pub fn delete_recipient_chunk(
    conn: &Connection,
    broadcast_id: i64,
    chunk_size: i64,
    only_while_draft: bool,
) -> Result<usize> {
    let guard = if only_while_draft {
        " AND EXISTS (SELECT 1 FROM \"Broadcast\" WHERE \"id\" = ?1 AND \"status\" = 'draft')"
    } else {
        ""
    };
    let sql = format!(
        "DELETE FROM \"ContactOnBroadcast\" WHERE (\"broadcastId\", \"contactId\") IN ( \
         SELECT \"broadcastId\", \"contactId\" FROM \"ContactOnBroadcast\" WHERE \"broadcastId\" = ?1{guard} \
         ORDER BY \"contactId\" ASC LIMIT ?2)"
    );
    Ok(conn.execute(&sql, rusqlite::params![broadcast_id, chunk_size])?)
}

/// The purge loop of `purgeBroadcastRecipients` with its clock and chunk step injected:
/// checks the deadline BEFORE each chunk, passes the time left to `chunk`, ends as
/// `Deadline` on a statement timeout (SQLSTATE 57014), `Drained` on a short chunk and
/// `ChunkCap` after `max_chunks`. Any other error propagates.
pub async fn run_purge_loop<C, F, Fut>(
    chunk_size: i64,
    inter_chunk_delay_ms: u64,
    max_run_duration_ms: i64,
    max_chunks: i64,
    mut clock: C,
    mut chunk: F,
) -> Result<PurgeOutcome>
where
    C: FnMut() -> i64,
    F: FnMut(i64) -> Fut,
    Fut: Future<Output = Result<usize>>,
{
    let deadline = clock() + max_run_duration_ms;
    let mut deleted: i64 = 0;
    for _ in 0..max_chunks {
        let remaining = deadline - clock();
        if remaining <= 0 {
            return Ok(PurgeOutcome {
                deleted,
                stop_reason: PurgeStopReason::Deadline,
            });
        }
        let count = match chunk(remaining).await {
            Ok(n) => i64::try_from(n).unwrap_or(i64::MAX),
            Err(e) if is_statement_timeout_error(&e) => {
                return Ok(PurgeOutcome {
                    deleted,
                    stop_reason: PurgeStopReason::Deadline,
                });
            }
            Err(e) => return Err(e),
        };
        deleted += count;
        if count < chunk_size {
            return Ok(PurgeOutcome {
                deleted,
                stop_reason: PurgeStopReason::Drained,
            });
        }
        if inter_chunk_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(inter_chunk_delay_ms)).await;
        }
    }
    Ok(PurgeOutcome {
        deleted,
        stop_reason: PurgeStopReason::ChunkCap,
    })
}

/// `purgeBroadcastRecipients`.
pub async fn purge_broadcast_recipients(db: &Db, input: &PurgeInput) -> Result<PurgeOutcome> {
    let (id, size, draft) = (input.broadcast_id, input.chunk_size, input.only_while_draft);
    run_purge_loop(
        input.chunk_size,
        input.inter_chunk_delay_ms,
        input.max_run_duration_ms,
        input.max_chunks.unwrap_or(DEFAULT_PURGE_MAX_CHUNKS),
        now_ms,
        |remaining| async move {
            super::write(db, move |tx| {
                let timeout = StatementTimeout::parse(&format!("{remaining}ms"))
                    .map_err(|e| super::Error::msg(e.to_string()))?;
                tx.set_statement_timeout(timeout)?;
                delete_recipient_chunk(tx, id, size, draft)
            })
            .await
        },
    )
    .await
}

/// `hasBroadcastRecipients`.
pub fn has_broadcast_recipients(conn: &Connection, broadcast_id: i64) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT 1 FROM \"ContactOnBroadcast\" WHERE \"broadcastId\" = ?1 LIMIT 1",
    )?;
    Ok(stmt.exists([broadcast_id])?)
}

/// `hardDeleteBroadcast`: only a soft-deleted, non-sending broadcast.
pub fn hard_delete_broadcast(conn: &Connection, broadcast_id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM \"Broadcast\" WHERE \"id\" = ?1 AND \"deletedAt\" IS NOT NULL AND \"status\" <> 'sending'",
        [broadcast_id],
    )?;
    Ok(n > 0)
}
