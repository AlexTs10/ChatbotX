//! The dispatch schedule (spec 3.2): the `DispatchSchedule` trait that
//! replaces `SchedulerClient` (Redis ZSETs `seq:dispatch:{b}:schedule|retry`
//! and the Redlock `withLock`). The SQLite implementation uses the
//! `sequence_schedule` table; tests use recording fakes.

use std::fmt;
use std::sync::Arc;

use tokio::sync::Notify;

use crate::enqueue::BoxFuture;

/// Which set: the schedule set or the retry set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScheduleKind {
    Schedule,
    Retry,
}

/// One `(bucket, dispatch id, due time)` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub bucket: u32,
    pub dispatch_id: String,
    pub at_ms: i64,
}

/// Errors of the schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// The lock for `key` is held by another owner (`LockAcquisitionError`).
    LockAcquisition { key: String },
    /// Some entries of a batch failed (`AggregateError`); one message each.
    Aggregate(Vec<String>),
    /// The store failed; the text is the cause.
    Store(String),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::LockAcquisition { key } => {
                write!(f, "Failed to acquire lock {key}")
            }
            ScheduleError::Aggregate(errors) => {
                write!(f, "Schedule batch failed: {}", errors.join("; "))
            }
            ScheduleError::Store(m) => f.write_str(m),
        }
    }
}

/// When the next schedule or retry entry of some buckets is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextDue {
    /// This schedule cannot tell (poll at the tick interval).
    Unknown,
    /// No entry at all (wait for an add).
    Nothing,
    /// The earliest due time (unix ms).
    At(i64),
}

impl std::error::Error for ScheduleError {}

/// `seq:dispatch:{<bucket>}:lock:<dispatch id>` (the lock key the consumer
/// compares against a `LockAcquisitionError`).
pub fn dispatch_lock_key(bucket: u32, dispatch_id: &str) -> String {
    format!("seq:dispatch:{{{bucket}}}:lock:{dispatch_id}")
}

/// A held dispatch lock. Dropping it releases the lock.
pub struct DispatchLock {
    pub key: String,
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl DispatchLock {
    /// A lock that runs `release` when dropped.
    pub fn new(key: String, release: Box<dyn FnOnce() + Send>) -> DispatchLock {
        DispatchLock {
            key,
            release: Some(release),
        }
    }
}

impl Drop for DispatchLock {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// The `SchedulerClient` operations (same names, snake_case).
pub trait DispatchSchedule: Send + Sync {
    /// `addToSchedule(bucket, dispatchId, runAtMs)` (ZADD = upsert time).
    fn add_to_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>>;

    /// `removeFromSchedule(bucket, dispatchId)`.
    fn remove_from_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>>;

    /// `addToRetry(bucket, dispatchId, retryAtMs)`.
    fn add_to_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>>;

    /// `removeFromRetry(bucket, dispatchId)`.
    fn remove_from_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>>;

    /// `removeFromAll(bucket, dispatchId)`: both sets.
    fn remove_from_all<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>>;

    /// `getDue(key, now, limit)`: ids with `at_ms <= now_ms`, oldest first.
    fn get_due(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>>;

    /// `getScheduleCount` / `getRetryCount`.
    fn count(&self, kind: ScheduleKind, bucket: u32) -> BoxFuture<'_, Result<u64, ScheduleError>>;

    /// `batchAddToSchedule(entries)`: one write transaction, all-or-nothing
    /// (DECISIONS P2.6). An empty batch does nothing.
    fn batch_add_to_schedule(
        &self,
        entries: Vec<ScheduleEntry>,
    ) -> BoxFuture<'_, Result<(), ScheduleError>>;

    /// `getZSetMembers(key)`: every id of one set.
    fn members(
        &self,
        kind: ScheduleKind,
        bucket: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>>;

    /// `withLock(bucket, dispatchId, ttlSeconds, fn)`: takes the dispatch
    /// lock or fails fast with `ScheduleError::LockAcquisition`.
    fn try_lock<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        ttl_secs: u64,
    ) -> BoxFuture<'a, Result<DispatchLock, ScheduleError>>;

    /// When the next entry of `buckets` is due (both sets). The default
    /// cannot tell: the producer then polls at its tick interval.
    fn next_due<'a>(&'a self, buckets: &'a [u32]) -> BoxFuture<'a, Result<NextDue, ScheduleError>> {
        let _ = buckets;
        Box::pin(async { Ok(NextDue::Unknown) })
    }

    /// The buckets of `buckets` with an entry due at `now_ms` (`None` = this
    /// schedule cannot tell: every bucket is polled).
    fn due_buckets<'a>(
        &'a self,
        now_ms: i64,
        buckets: &'a [u32],
    ) -> BoxFuture<'a, Result<Option<Vec<u32>>, ScheduleError>> {
        let _ = (now_ms, buckets);
        Box::pin(async { Ok(None) })
    }

    /// Notified after every add (`None` = no signal).
    fn change_signal(&self) -> Option<Arc<Notify>> {
        None
    }
}
