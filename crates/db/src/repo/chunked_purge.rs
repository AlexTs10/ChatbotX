//! Port of `repositories/chunked-purge.ts`: deletes rows of one table in id chunks, each
//! chunk its own short write transaction, with a delay between chunks.

use super::sql::Sql;
use super::{Result, now_ms};
use crate::Db;

/// `ChunkedPurgeStopReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkedPurgeStopReason {
    Drained,
    Deadline,
    ChunkCap,
}

/// `ChunkedPurgeOptions` (`where` is a SQL fragment over the table's columns).
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkedPurgeOptions {
    pub table: String,
    pub filter: Sql,
    pub order_by: String,
    pub chunk_size: i64,
    pub inter_chunk_delay_ms: u64,
    pub max_chunks: i64,
    pub max_run_duration_ms: Option<i64>,
}

/// `chunkedPurge`: returns `(deleted, stop reason)`. The deadline is checked after a full
/// chunk (as the TypeScript).
pub async fn chunked_purge(
    db: &Db,
    options: &ChunkedPurgeOptions,
) -> Result<(i64, ChunkedPurgeStopReason)> {
    let deadline = options.max_run_duration_ms.map(|ms| now_ms() + ms);
    let mut deleted = 0_i64;
    let mut chunk = 0_i64;
    loop {
        if chunk >= options.max_chunks {
            return Ok((deleted, ChunkedPurgeStopReason::ChunkCap));
        }
        chunk += 1;
        let mut q = Sql::raw(format!(
            "DELETE FROM \"{t}\" WHERE \"id\" IN (SELECT \"id\" FROM \"{t}\" WHERE ",
            t = options.table
        ));
        q.append(options.filter.clone());
        q.push(&format!(" ORDER BY \"{}\" ASC LIMIT ", options.order_by));
        q.push_bind(options.chunk_size);
        q.push(")");
        let count = super::write(db, move |tx| q.execute(tx)).await?;
        let count = i64::try_from(count).unwrap_or(i64::MAX);
        deleted += count;
        if count < options.chunk_size {
            return Ok((deleted, ChunkedPurgeStopReason::Drained));
        }
        if deadline.is_some_and(|d| now_ms() >= d) {
            return Ok((deleted, ChunkedPurgeStopReason::Deadline));
        }
        if options.inter_chunk_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                options.inter_chunk_delay_ms,
            ))
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deletes_in_chunks_until_drained_or_capped() {
        let db = Db::open_in_memory().expect("open");
        db.write(|tx| {
            tx.execute_batch(
                "CREATE TABLE \"T\" (\"id\" INTEGER PRIMARY KEY, \"k\" INTEGER) STRICT;",
            )?;
            for i in 1..=7 {
                tx.execute("INSERT INTO \"T\" VALUES (?1, ?2)", [i, i % 2])?;
            }
            Ok(())
        })
        .await
        .expect("setup");
        let mut options = ChunkedPurgeOptions {
            table: "T".into(),
            filter: Sql::bind("\"k\" = ", 1_i64),
            order_by: "id".into(),
            chunk_size: 2,
            inter_chunk_delay_ms: 0,
            max_chunks: 1,
            max_run_duration_ms: None,
        };
        assert_eq!(
            chunked_purge(&db, &options).await.expect("capped"),
            (2, ChunkedPurgeStopReason::ChunkCap)
        );
        options.max_chunks = 10;
        assert_eq!(
            chunked_purge(&db, &options).await.expect("drained"),
            (2, ChunkedPurgeStopReason::Drained)
        );
    }
}
