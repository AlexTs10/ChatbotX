//! The SQLite `DispatchSchedule` (spec 3.2): the `sequence_schedule` table
//! (`kind`, `dispatch_id`, `bucket`, `at_ms`; primary key `(kind,
//! dispatch_id)`) and the in-process dispatch lock.

use std::sync::Arc;

use chatbotx_db::Db;

use crate::enqueue::BoxFuture;
use crate::sequence::repo::TxObserver;
use crate::sequence::schedule::{
    DispatchLock, DispatchSchedule, ScheduleEntry, ScheduleError, ScheduleKind,
};

/// `SchedulerClient` on SQLite. Every write is one write transaction,
/// reported to `observer` (`on_begin` / `on_commit`).
pub struct SqliteDispatchSchedule {
    db: Arc<Db>,
    observer: Option<Arc<dyn TxObserver>>,
}

impl SqliteDispatchSchedule {
    /// Opens the schedule on `db` and creates `sequence_schedule` if missing.
    pub async fn open(
        db: Arc<Db>,
        observer: Option<Arc<dyn TxObserver>>,
    ) -> Result<SqliteDispatchSchedule, ScheduleError> {
        todo!()
    }
}

impl DispatchSchedule for SqliteDispatchSchedule {
    fn add_to_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        todo!()
    }

    fn remove_from_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        todo!()
    }

    fn add_to_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        todo!()
    }

    fn remove_from_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        todo!()
    }

    fn remove_from_all<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        todo!()
    }

    fn get_due(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        todo!()
    }

    fn count(&self, kind: ScheduleKind, bucket: u32) -> BoxFuture<'_, Result<u64, ScheduleError>> {
        todo!()
    }

    fn batch_add_to_schedule(
        &self,
        entries: Vec<ScheduleEntry>,
    ) -> BoxFuture<'_, Result<(), ScheduleError>> {
        todo!()
    }

    fn members(
        &self,
        kind: ScheduleKind,
        bucket: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        todo!()
    }

    fn try_lock<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        ttl_secs: u64,
    ) -> BoxFuture<'a, Result<DispatchLock, ScheduleError>> {
        todo!()
    }
}
