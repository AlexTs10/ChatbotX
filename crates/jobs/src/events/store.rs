//! The stream store behind a bus: the `event_entry` and
//! `event_dead_letter` tables (spec 4.2), as a trait so tests can drive
//! the bus with a scripted fake (the frozen tests script a fake Redis).

use std::fmt;
use std::sync::Arc;

use chatbotx_db::rusqlite::{Connection, OptionalExtension, Rows, params};
use chatbotx_db::{Db, DbError};

use crate::enqueue::BoxFuture;

/// One entry as a read or reclaim returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEntry {
    /// `"<id>-0"` (keeps the Redis stream id shape).
    pub id: String,
    pub event_type: String,
    /// The payload JSON text.
    pub payload: String,
    /// Deliveries so far, this one included (Redis PEL delivery count).
    pub delivery_count: u32,
}

/// One dead-letter write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    /// The dead-letter stream key (`<stream>:dead` by default).
    pub stream: String,
    pub max_len: u64,
    pub event_type: String,
    pub payload: String,
    pub original_id: String,
    pub delivery_count: u32,
    pub error: String,
    pub ts_ms: i64,
}

/// Store errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventStoreError {
    /// The consumer group exists already (Redis `BUSYGROUP`).
    BusyGroup,
    /// Any other failure, with its message.
    Other(String),
}

impl fmt::Display for EventStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventStoreError::BusyGroup => {
                f.write_str("BUSYGROUP Consumer Group name already exists")
            }
            EventStoreError::Other(m) => f.write_str(m),
        }
    }
}

impl From<chatbotx_db::DbError> for EventStoreError {
    fn from(e: chatbotx_db::DbError) -> EventStoreError {
        EventStoreError::Other(e.to_string())
    }
}

impl std::error::Error for EventStoreError {}

/// The stream operations a bus uses.
pub trait EventStore: Send + Sync {
    /// `XGROUP CREATE <stream> <group> 0 MKSTREAM`.
    fn create_group<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>>;

    /// `XADD <stream> MAXLEN ~ <max_len> * type .. payload .. timestamp ..`; returns the id.
    fn append<'a>(
        &'a self,
        stream: &'a str,
        max_len: u64,
        event_type: &'a str,
        payload: &'a str,
        ts_ms: i64,
    ) -> BoxFuture<'a, Result<String, EventStoreError>>;

    /// `XREADGROUP GROUP <group> <consumer> COUNT <count> STREAMS <stream> >`:
    /// up to `count` new entries, marked delivered to `consumer`.
    fn read_new<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>>;

    /// `XAUTOCLAIM <stream> <group> <consumer> <min_idle_ms> 0 COUNT <count>`
    /// with the delivery count of each claimed entry.
    fn reclaim<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        min_idle_ms: u64,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>>;

    /// `XPENDING <stream> <group> - + 1 <consumer>`: unacked entries of `consumer`.
    fn pending_count<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>>;

    /// `XACK` (= delete the entries).
    fn ack<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>>;

    /// Writes one dead letter.
    fn dead_letter(&self, entry: DeadLetter) -> BoxFuture<'_, Result<String, EventStoreError>>;

    /// `XGROUP DELCONSUMER`.
    fn delete_consumer<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>>;

    /// Releases the consumer's read resources when its loop ends (the
    /// dedicated blocking connection's `quit`).
    fn close_reader<'a>(&'a self, consumer: &'a str) -> BoxFuture<'a, Result<(), EventStoreError>>;

    /// Fallback when `close_reader` fails (`disconnect`).
    fn force_close_reader(&self, consumer: &str);
}

/// The SQLite implementation (`event_entry`, `event_dead_letter`).
pub struct SqliteEventStore {
    db: Arc<Db>,
}

impl SqliteEventStore {
    /// Opens the store and creates its tables if missing.
    pub async fn open(db: Arc<Db>) -> Result<SqliteEventStore, EventStoreError> {
        db.migrate()
            .await
            .map_err(|e| EventStoreError::Other(e.to_string()))?;
        Ok(SqliteEventStore { db })
    }
}

/// `"<id>-0"` (the Redis stream id shape).
fn entry_id(id: i64) -> String {
    format!("{id}-0")
}

fn parse_entry_id(id: &str) -> Option<i64> {
    id.split('-').next()?.parse().ok()
}

const NOW_MS: &str = "CAST(unixepoch('subsec') * 1000 AS INTEGER)";

/// Drops the oldest entries of `stream` in `table` beyond `max_len`
/// (the `MAXLEN ~` intent: bound the disk while a consumer is down).
fn cap(conn: &Connection, table: &str, stream: &str, max_len: u64) -> Result<(), DbError> {
    if max_len == 0 {
        return Ok(());
    }
    let offset = i64::try_from(max_len).unwrap_or(i64::MAX);
    let cut: Option<i64> = conn
        .query_row(
            &format!("SELECT id FROM {table} WHERE stream = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2"),
            params![stream, offset],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(cut) = cut {
        let dropped = conn.execute(
            &format!("DELETE FROM {table} WHERE stream = ?1 AND id <= ?2"),
            params![stream, cut],
        )?;
        tracing::warn!(
            stream,
            dropped,
            max_len,
            "event stream over its cap; oldest entries dropped"
        );
    }
    Ok(())
}

fn claimed(rows: &mut Rows<'_>) -> Result<Vec<StreamEntry>, DbError> {
    let mut out: Vec<(i64, StreamEntry)> = Vec::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        out.push((
            id,
            StreamEntry {
                id: entry_id(id),
                event_type: row.get(1)?,
                payload: row.get(2)?,
                delivery_count: row.get(3)?,
            },
        ));
    }
    out.sort_by_key(|(id, _)| *id);
    Ok(out.into_iter().map(|(_, e)| e).collect())
}

impl EventStore for SqliteEventStore {
    fn create_group<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>> {
        let (stream, group) = (stream.to_owned(), group.to_owned());
        Box::pin(async move {
            let created = crate::db_retry::write(&self.db, move |tx| {
                    Ok(tx.execute(
                        "INSERT INTO event_group (stream, group_name) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                        params![stream, group],
                    )? > 0)
                })
                .await?;
            if created {
                Ok(())
            } else {
                Err(EventStoreError::BusyGroup)
            }
        })
    }

    fn append<'a>(
        &'a self,
        stream: &'a str,
        max_len: u64,
        event_type: &'a str,
        payload: &'a str,
        ts_ms: i64,
    ) -> BoxFuture<'a, Result<String, EventStoreError>> {
        let (stream, event_type, payload) =
            (stream.to_owned(), event_type.to_owned(), payload.to_owned());
        Box::pin(async move {
            let id = crate::db_retry::write(&self.db, move |tx| {
                    let id: i64 = tx.query_row(
                        "INSERT INTO event_entry (stream, type, payload, ts_ms) VALUES (?1, ?2, ?3, ?4) RETURNING id",
                        params![stream, event_type, payload, ts_ms],
                        |r| r.get(0),
                    )?;
                    cap(tx, "event_entry", &stream, max_len)?;
                    Ok(id)
                })
                .await?;
            Ok(entry_id(id))
        })
    }

    fn read_new<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>> {
        let _ = group;
        let (stream, consumer) = (stream.to_owned(), consumer.to_owned());
        Box::pin(async move {
            Ok(crate::db_retry::write(&self.db, move |tx| {
                    let sql = format!(
                        "UPDATE event_entry SET delivered_at_ms = {NOW_MS}, delivery_count = delivery_count + 1, \
                         consumer = ?1 WHERE id IN (SELECT id FROM event_entry WHERE stream = ?2 \
                         AND delivered_at_ms IS NULL ORDER BY id LIMIT ?3) \
                         RETURNING id, type, payload, delivery_count"
                    );
                    let mut stmt = tx.prepare_cached(&sql)?;
                    let mut rows = stmt.query(params![consumer, stream, count])?;
                    claimed(&mut rows)
                })
                .await?)
        })
    }

    fn reclaim<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        min_idle_ms: u64,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>> {
        let _ = group;
        let (stream, consumer) = (stream.to_owned(), consumer.to_owned());
        let min_idle = i64::try_from(min_idle_ms).unwrap_or(i64::MAX);
        Box::pin(async move {
            Ok(crate::db_retry::write(&self.db, move |tx| {
                    let sql = format!(
                        "UPDATE event_entry SET delivered_at_ms = {NOW_MS}, delivery_count = delivery_count + 1, \
                         consumer = ?1 WHERE id IN (SELECT id FROM event_entry WHERE stream = ?2 \
                         AND delivered_at_ms IS NOT NULL AND delivered_at_ms <= {NOW_MS} - ?3 \
                         ORDER BY delivered_at_ms LIMIT ?4) \
                         RETURNING id, type, payload, delivery_count"
                    );
                    let mut stmt = tx.prepare_cached(&sql)?;
                    let mut rows = stmt.query(params![consumer, stream, min_idle, count])?;
                    claimed(&mut rows)
                })
                .await?)
        })
    }

    fn pending_count<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>> {
        let _ = group;
        let (stream, consumer) = (stream.to_owned(), consumer.to_owned());
        Box::pin(async move {
            Ok(crate::db_retry::read(&self.db, move |conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM event_entry WHERE stream = ?1 \
                         AND delivered_at_ms IS NOT NULL AND consumer = ?2",
                    params![stream, consumer],
                    |r| r.get::<_, i64>(0),
                )? as u64)
            })
            .await?)
        })
    }

    fn ack<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>> {
        let _ = group;
        let stream = stream.to_owned();
        Box::pin(async move {
            let ids: Vec<i64> = ids.iter().filter_map(|id| parse_entry_id(id)).collect();
            if ids.is_empty() {
                return Ok(0);
            }
            Ok(crate::db_retry::write(&self.db, move |tx| {
                let mut stmt =
                    tx.prepare_cached("DELETE FROM event_entry WHERE stream = ?1 AND id = ?2")?;
                let mut n = 0u64;
                for id in ids {
                    n += stmt.execute(params![stream, id])? as u64;
                }
                Ok(n)
            })
            .await?)
        })
    }

    fn dead_letter(&self, entry: DeadLetter) -> BoxFuture<'_, Result<String, EventStoreError>> {
        Box::pin(async move {
            let id = crate::db_retry::write(&self.db, move |tx| {
                    let id: i64 = tx.query_row(
                        "INSERT INTO event_dead_letter (stream, type, payload, original_id, delivery_count, error, ts_ms) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
                        params![
                            entry.stream,
                            entry.event_type,
                            entry.payload,
                            entry.original_id,
                            entry.delivery_count,
                            entry.error,
                            entry.ts_ms
                        ],
                        |r| r.get(0),
                    )?;
                    cap(tx, "event_dead_letter", &entry.stream, entry.max_len)?;
                    Ok(id)
                })
                .await?;
            Ok(entry_id(id))
        })
    }

    fn delete_consumer<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>> {
        // Consumers are implicit (the `consumer` column of pending entries).
        let _ = (stream, group, consumer);
        Box::pin(async { Ok(()) })
    }

    fn close_reader<'a>(&'a self, consumer: &'a str) -> BoxFuture<'a, Result<(), EventStoreError>> {
        // No dedicated connection: the reader pool is shared.
        let _ = consumer;
        Box::pin(async { Ok(()) })
    }

    fn force_close_reader(&self, consumer: &str) {
        let _ = consumer;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;
    use crate::clock::SystemClock;
    use crate::events::bus::{EventBus, EventListener, ListenerOutcome, Listeners};
    use crate::events::config::EventBusConfig;

    async fn store() -> Arc<SqliteEventStore> {
        let db = crate::temp_db();
        Arc::new(SqliteEventStore::open(db).await.expect("store"))
    }

    #[tokio::test]
    async fn entries_flow_through_read_ack_reclaim_and_dead_letter() {
        let store = store().await;
        assert_eq!(store.create_group("s", "g").await, Ok(()));
        assert_eq!(
            store.create_group("s", "g").await,
            Err(EventStoreError::BusyGroup)
        );

        let first = store.append("s", 0, "t", "{\"n\":1}", 1).await.unwrap();
        store.append("s", 0, "t", "{\"n\":2}", 2).await.unwrap();
        store.append("other", 0, "t", "{}", 3).await.unwrap();
        assert!(first.ends_with("-0"));

        let read = store.read_new("s", "g", "c1", 10).await.unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(
            (read[0].id.clone(), read[0].delivery_count),
            (first.clone(), 1)
        );
        assert!(store.read_new("s", "g", "c1", 10).await.unwrap().is_empty());
        assert_eq!(store.pending_count("s", "g", "c1").await.unwrap(), 2);

        assert_eq!(store.ack("s", "g", vec![first.clone()]).await.unwrap(), 1);
        assert_eq!(store.pending_count("s", "g", "c1").await.unwrap(), 1);

        // Idle long enough (0 ms): reclaimed with a higher delivery count.
        let reclaimed = store.reclaim("s", "g", "c2", 0, 10).await.unwrap();
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].delivery_count, 2);
        assert!(
            store
                .reclaim("s", "g", "c2", 3_600_000, 10)
                .await
                .unwrap()
                .is_empty()
        );

        let dead = DeadLetter {
            stream: "s:dead".to_owned(),
            max_len: 1,
            event_type: "t".to_owned(),
            payload: "{\"n\":2}".to_owned(),
            original_id: reclaimed[0].id.clone(),
            delivery_count: 6,
            error: "boom".to_owned(),
            ts_ms: 5,
        };
        store.dead_letter(dead.clone()).await.unwrap();
        store.dead_letter(dead).await.unwrap();
        let dead_rows: i64 = store
            .db
            .read(|c| Ok(c.query_row("SELECT count(*) FROM event_dead_letter", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(dead_rows, 1, "dead letters capped at max_len");
    }

    #[tokio::test]
    async fn unacked_entries_are_capped_at_max_len() {
        let store = store().await;
        for n in 0..5 {
            store
                .append("s", 3, "t", &format!("{{\"n\":{n}}}"), n)
                .await
                .unwrap();
        }
        let read = store.read_new("s", "g", "c", 10).await.unwrap();
        let payloads: Vec<&str> = read.iter().map(|e| e.payload.as_str()).collect();
        assert_eq!(payloads, ["{\"n\":2}", "{\"n\":3}", "{\"n\":4}"]);
    }

    #[tokio::test]
    async fn a_consuming_bus_wakes_on_emit_without_polling() {
        let store = store().await;
        let config = EventBusConfig {
            claim_interval_ms: None,
            ..EventBusConfig::new("events:unit", "unit-group", BTreeMap::new())
        };
        let bus = Arc::new(EventBus::new(store.clone(), config, Arc::new(SystemClock)));
        bus.initialize().await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<Value>>();
        let mut listeners: Listeners = BTreeMap::new();
        listeners.insert(
            "unit:event".to_owned(),
            vec![EventListener {
                name: "collect".to_owned(),
                handler: Arc::new(move |payloads, _signal| {
                    let _ = tx.send(payloads);
                    Box::pin(async { Ok(ListenerOutcome::default()) })
                }),
            }],
        );
        let consuming = {
            let bus = Arc::clone(&bus);
            tokio::spawn(async move { bus.start_consuming("c", listeners).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let id = bus.emit("unit:event", json!({ "v": 1 })).await.unwrap();
        let batch = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("listener called")
            .expect("batch");
        assert_eq!(batch, vec![json!({ "v": 1, "__eventBusMessageId": id })]);
        bus.stop();
        tokio::time::timeout(Duration::from_secs(5), consuming)
            .await
            .expect("loop ended")
            .unwrap();
        assert_eq!(
            store
                .pending_count("events:unit", "unit-group", "c")
                .await
                .unwrap(),
            0
        );
    }
}
