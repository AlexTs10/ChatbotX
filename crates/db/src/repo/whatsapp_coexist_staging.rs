//! Port of `repositories/whatsapp-coexist-staging/repository.ts` (P3.3).

use rusqlite::Connection;

use super::chunked_purge::{ChunkedPurgeOptions, ChunkedPurgeStopReason, chunked_purge};
use super::models::WhatsappCoexistStaging;
use super::sql::Sql;
use super::{Json, Result, id_list, now_ms};
use crate::Db;

/// Chunk bounds of the two purges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeBounds {
    pub chunk_size: i64,
    pub inter_chunk_delay_ms: u64,
    pub max_chunks: i64,
    pub max_run_duration_ms: Option<i64>,
}

/// `stagePayload` (a repeat of `(phoneNumberId, payloadHash)` is ignored).
pub fn stage_payload(
    conn: &Connection,
    id: i64,
    phone_number_id: &str,
    payload: &Json,
    payload_hash: &str,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"WhatsappCoexistStaging\" (\"id\", \"createdAt\", \"updatedAt\", \"phoneNumberId\", \"payload\", \
         \"payloadHash\") VALUES (?1, ?2, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        rusqlite::params![id, now, phone_number_id, payload, payload_hash],
    )?;
    Ok(())
}

/// `listPending`: neither processed nor parse-failed, by id.
pub fn list_pending(
    conn: &Connection,
    phone_number_id: &str,
    limit: i64,
) -> Result<Vec<WhatsappCoexistStaging>> {
    super::find_many(
        conn,
        "WHERE \"phoneNumberId\" = ?1 AND \"processedAt\" IS NULL AND \"parseFailedAt\" IS NULL ORDER BY \"id\" LIMIT ?2",
        &[&phone_number_id, &limit],
    )
}

fn stamp(conn: &Connection, column: &str, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_ms();
    conn.execute(
        &format!(
            "UPDATE \"WhatsappCoexistStaging\" SET \"{column}\" = ?2, \"updatedAt\" = ?2 \
             WHERE \"id\" IN (SELECT value FROM json_each(?1))"
        ),
        rusqlite::params![id_list(ids), now],
    )?;
    Ok(())
}

/// `markProcessed`.
pub fn mark_processed(conn: &Connection, ids: &[i64]) -> Result<()> {
    stamp(conn, "processedAt", ids)
}

/// `markParseFailed`.
pub fn mark_parse_failed(conn: &Connection, ids: &[i64]) -> Result<()> {
    stamp(conn, "parseFailedAt", ids)
}

/// `purgeProcessed`: rows processed more than `retention_hours` before `now`.
pub async fn purge_processed(
    db: &Db,
    retention_hours: i64,
    now: i64,
    bounds: PurgeBounds,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    purge(db, "processedAt", now - retention_hours * 3_600_000, bounds).await
}

/// `purgeParseFailed`: rows that failed to parse more than `retention_days` before `now`.
pub async fn purge_parse_failed(
    db: &Db,
    retention_days: i64,
    now: i64,
    bounds: PurgeBounds,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    purge(
        db,
        "parseFailedAt",
        now - retention_days * 86_400_000,
        bounds,
    )
    .await
}

async fn purge(
    db: &Db,
    column: &str,
    cutoff: i64,
    bounds: PurgeBounds,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    chunked_purge(
        db,
        &ChunkedPurgeOptions {
            table: "WhatsappCoexistStaging".to_owned(),
            filter: Sql::bind(
                format!("\"{column}\" IS NOT NULL AND \"{column}\" < "),
                cutoff,
            ),
            order_by: column.to_owned(),
            chunk_size: bounds.chunk_size,
            inter_chunk_delay_ms: bounds.inter_chunk_delay_ms,
            max_chunks: bounds.max_chunks,
            max_run_duration_ms: bounds.max_run_duration_ms,
        },
    )
    .await
}
