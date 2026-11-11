//! The stream store behind a bus: the `event_entry` and
//! `event_dead_letter` tables (spec 4.2), as a trait so tests can drive
//! the bus with a scripted fake (the frozen tests script a fake Redis).

use std::fmt;
use std::sync::Arc;

use chatbotx_db::Db;

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
        todo!()
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
        todo!()
    }
}

impl EventStore for SqliteEventStore {
    fn create_group<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>> {
        todo!()
    }

    fn append<'a>(
        &'a self,
        stream: &'a str,
        max_len: u64,
        event_type: &'a str,
        payload: &'a str,
        ts_ms: i64,
    ) -> BoxFuture<'a, Result<String, EventStoreError>> {
        todo!()
    }

    fn read_new<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>> {
        todo!()
    }

    fn reclaim<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
        min_idle_ms: u64,
        count: u32,
    ) -> BoxFuture<'a, Result<Vec<StreamEntry>, EventStoreError>> {
        todo!()
    }

    fn pending_count<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>> {
        todo!()
    }

    fn ack<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Result<u64, EventStoreError>> {
        todo!()
    }

    fn dead_letter(&self, entry: DeadLetter) -> BoxFuture<'_, Result<String, EventStoreError>> {
        todo!()
    }

    fn delete_consumer<'a>(
        &'a self,
        stream: &'a str,
        group: &'a str,
        consumer: &'a str,
    ) -> BoxFuture<'a, Result<(), EventStoreError>> {
        todo!()
    }

    fn close_reader<'a>(&'a self, consumer: &'a str) -> BoxFuture<'a, Result<(), EventStoreError>> {
        todo!()
    }

    fn force_close_reader(&self, consumer: &str) {
        todo!()
    }
}
