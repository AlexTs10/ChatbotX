//! The SQLite `DispatchSchedule` (spec 3.2): the `sequence_schedule` table
//! (`kind`, `dispatch_id`, `bucket`, `at_ms`; primary key `(kind,
//! dispatch_id)`) and the in-process dispatch lock.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use tokio::sync::Notify;

use crate::enqueue::BoxFuture;
use crate::sequence::buckets::BUCKET_COUNT;
use crate::sequence::repo::TxObserver;
use crate::sequence::schedule::{
    DispatchLock, DispatchSchedule, NextDue, ScheduleEntry, ScheduleError, ScheduleKind,
    dispatch_lock_key,
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
        db.migrate()
            .await
            .map_err(|e| ScheduleError::Store(e.to_string()))?;
        Ok(SqliteDispatchSchedule { db, observer })
    }

    /// One write transaction, reported to the observer.
    async fn write<R, F>(&self, f: F) -> Result<R, ScheduleError>
    where
        F: FnOnce(&Connection) -> Result<R, ScheduleError> + Clone + Send + 'static,
        R: Send + 'static,
    {
        if let Some(o) = &self.observer {
            o.on_begin();
        }
        let slot: Arc<Mutex<Option<ScheduleError>>> = Arc::new(Mutex::new(None));
        let inner = Arc::clone(&slot);
        let result = crate::db_retry::write(&self.db, move |tx| match f(tx) {
            Ok(v) => Ok(v),
            Err(e) => {
                *inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                Err(DbError::other("rolled back"))
            }
        })
        .await;
        match result {
            Ok(v) => {
                if let Some(o) = &self.observer {
                    o.on_commit();
                }
                Ok(v)
            }
            Err(db_err) => Err(slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| ScheduleError::Store(db_err.to_string()))),
        }
    }

    async fn read<R, F>(&self, f: F) -> Result<R, ScheduleError>
    where
        F: FnOnce(&Connection) -> Result<R, ScheduleError> + Clone + Send + 'static,
        R: Send + 'static,
    {
        crate::db_retry::read(&self.db, move |conn| Ok(f(conn)))
            .await
            .map_err(store)?
    }

    async fn upsert(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        dispatch_id: &str,
        at_ms: i64,
    ) -> Result<(), ScheduleError> {
        let id = dispatch_number(dispatch_id)?;
        self.write(move |conn| {
            conn.execute(UPSERT, params![kind_str(kind), id, bucket, at_ms])
                .map_err(store)?;
            Ok(())
        })
        .await?;
        signal().notify_one();
        Ok(())
    }

    async fn delete(
        &self,
        kinds: &'static [ScheduleKind],
        bucket: u32,
        dispatch_id: &str,
    ) -> Result<(), ScheduleError> {
        let id = dispatch_number(dispatch_id)?;
        self.write(move |conn| {
            for kind in kinds {
                conn.execute(
                    "DELETE FROM sequence_schedule WHERE kind = ?1 AND dispatch_id = ?2 AND bucket = ?3",
                    params![kind_str(*kind), id, bucket],
                )
                .map_err(store)?;
            }
            Ok(())
        })
        .await
    }
}

const UPSERT: &str = "INSERT INTO sequence_schedule (kind, dispatch_id, bucket, at_ms) VALUES (?1, ?2, ?3, ?4) \
     ON CONFLICT (kind, dispatch_id) DO UPDATE SET bucket = excluded.bucket, at_ms = excluded.at_ms";

fn store(e: impl fmt::Display) -> ScheduleError {
    ScheduleError::Store(e.to_string())
}

fn kind_str(kind: ScheduleKind) -> &'static str {
    match kind {
        ScheduleKind::Schedule => "schedule",
        ScheduleKind::Retry => "retry",
    }
}

fn dispatch_number(id: &str) -> Result<i64, ScheduleError> {
    id.trim()
        .parse()
        .map_err(|_| ScheduleError::Store(format!("invalid dispatch id {id:?}")))
}

/// Process-wide: every add to any schedule of this process wakes the
/// producer (spec 3.2, no polling while idle).
fn signal() -> Arc<Notify> {
    static SIGNAL: OnceLock<Arc<Notify>> = OnceLock::new();
    Arc::clone(SIGNAL.get_or_init(|| Arc::new(Notify::new())))
}

/// The in-process dispatch locks (`withLock` without Redis): key → (holder
/// token, expiry). A lock past its ttl can be taken again.
fn locks() -> &'static Mutex<HashMap<String, (u64, Instant)>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, (u64, Instant)>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `AND bucket IN (..)` unless every bucket is assigned.
fn bucket_filter(buckets: &[u32]) -> String {
    if buckets.len() >= BUCKET_COUNT as usize {
        return String::new();
    }
    let list = buckets
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(" AND bucket IN ({list})")
}

impl DispatchSchedule for SqliteDispatchSchedule {
    fn add_to_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(self.upsert(ScheduleKind::Schedule, bucket, dispatch_id, at_ms))
    }

    fn remove_from_schedule<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(self.delete(&[ScheduleKind::Schedule], bucket, dispatch_id))
    }

    fn add_to_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        at_ms: i64,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(self.upsert(ScheduleKind::Retry, bucket, dispatch_id, at_ms))
    }

    fn remove_from_retry<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(self.delete(&[ScheduleKind::Retry], bucket, dispatch_id))
    }

    fn remove_from_all<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ScheduleError>> {
        Box::pin(self.delete(
            &[ScheduleKind::Schedule, ScheduleKind::Retry],
            bucket,
            dispatch_id,
        ))
    }

    fn get_due(
        &self,
        kind: ScheduleKind,
        bucket: u32,
        now_ms: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        Box::pin(self.read(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT dispatch_id FROM sequence_schedule WHERE kind = ?1 AND bucket = ?2 \
                     AND at_ms <= ?3 ORDER BY at_ms, dispatch_id LIMIT ?4",
                )
                .map_err(store)?;
            let ids = stmt
                .query_map(params![kind_str(kind), bucket, now_ms, limit], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(store)?
                .map(|r| r.map(|id| id.to_string()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(store)?;
            Ok(ids)
        }))
    }

    fn count(&self, kind: ScheduleKind, bucket: u32) -> BoxFuture<'_, Result<u64, ScheduleError>> {
        Box::pin(self.read(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM sequence_schedule WHERE kind = ?1 AND bucket = ?2",
                params![kind_str(kind), bucket],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as u64)
            .map_err(store)
        }))
    }

    fn batch_add_to_schedule(
        &self,
        entries: Vec<ScheduleEntry>,
    ) -> BoxFuture<'_, Result<(), ScheduleError>> {
        Box::pin(async move {
            if entries.is_empty() {
                return Ok(());
            }
            let mut parsed = Vec::with_capacity(entries.len());
            for e in &entries {
                parsed.push((dispatch_number(&e.dispatch_id)?, e.bucket, e.at_ms));
            }
            self.write(move |conn| {
                let mut errors = Vec::new();
                for (id, bucket, at_ms) in parsed {
                    if let Err(e) = conn.execute(UPSERT, params!["schedule", id, bucket, at_ms]) {
                        errors.push(format!("dispatch {id}: {e}"));
                    }
                }
                // All-or-nothing (DECISIONS P2.6): any failure rolls back.
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(ScheduleError::Aggregate(errors))
                }
            })
            .await?;
            signal().notify_one();
            Ok(())
        })
    }

    fn members(
        &self,
        kind: ScheduleKind,
        bucket: u32,
    ) -> BoxFuture<'_, Result<Vec<String>, ScheduleError>> {
        Box::pin(self.read(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT dispatch_id FROM sequence_schedule WHERE kind = ?1 AND bucket = ?2 \
                     ORDER BY at_ms, dispatch_id",
                )
                .map_err(store)?;
            let ids = stmt
                .query_map(params![kind_str(kind), bucket], |r| r.get::<_, i64>(0))
                .map_err(store)?
                .map(|r| r.map(|id| id.to_string()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(store)?;
            Ok(ids)
        }))
    }

    fn try_lock<'a>(
        &'a self,
        bucket: u32,
        dispatch_id: &'a str,
        ttl_secs: u64,
    ) -> BoxFuture<'a, Result<DispatchLock, ScheduleError>> {
        let key = dispatch_lock_key(bucket, dispatch_id);
        Box::pin(async move {
            let now = Instant::now();
            let token = crate::rand::next_u64();
            {
                let mut held = locks().lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((_, expiry)) = held.get(&key)
                    && *expiry > now
                {
                    return Err(ScheduleError::LockAcquisition { key });
                }
                held.insert(
                    key.clone(),
                    (token, now + Duration::from_secs(ttl_secs.max(1))),
                );
            }
            let released = key.clone();
            Ok(DispatchLock::new(
                key,
                Box::new(move || {
                    let mut held = locks().lock().unwrap_or_else(PoisonError::into_inner);
                    if held.get(&released).is_some_and(|(t, _)| *t == token) {
                        held.remove(&released);
                    }
                }),
            ))
        })
    }

    fn next_due<'a>(&'a self, buckets: &'a [u32]) -> BoxFuture<'a, Result<NextDue, ScheduleError>> {
        let filter = bucket_filter(buckets);
        Box::pin(self.read(move |conn| {
            let next: Option<i64> = conn
                .query_row(
                    &format!(
                        "SELECT min(at_ms) FROM sequence_schedule WHERE kind IN ('schedule', 'retry'){filter}"
                    ),
                    [],
                    |r| r.get(0),
                )
                .map_err(store)?;
            Ok(next.map_or(NextDue::Nothing, NextDue::At))
        }))
    }

    fn due_buckets<'a>(
        &'a self,
        now_ms: i64,
        buckets: &'a [u32],
    ) -> BoxFuture<'a, Result<Option<Vec<u32>>, ScheduleError>> {
        let filter = bucket_filter(buckets);
        Box::pin(self.read(move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT DISTINCT bucket FROM sequence_schedule \
                     WHERE kind IN ('schedule', 'retry') AND at_ms <= ?1{filter} ORDER BY bucket"
                ))
                .map_err(store)?;
            let list = stmt
                .query_map(params![now_ms], |r| r.get::<_, u32>(0))
                .map_err(store)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store)?;
            Ok(Some(list))
        }))
    }

    fn change_signal(&self) -> Option<Arc<Notify>> {
        Some(signal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn schedule() -> SqliteDispatchSchedule {
        let db = Arc::new(Db::open_in_memory().expect("db"));
        SqliteDispatchSchedule::open(db, None)
            .await
            .expect("schedule")
    }

    #[tokio::test]
    async fn the_sets_behave_like_sorted_sets() {
        let s = schedule().await;
        s.add_to_schedule(1, "10", 300).await.unwrap();
        s.add_to_schedule(1, "11", 100).await.unwrap();
        s.add_to_schedule(2, "12", 50).await.unwrap();
        s.add_to_schedule(1, "10", 200).await.unwrap();
        s.add_to_retry(1, "13", 10).await.unwrap();

        assert_eq!(
            s.get_due(ScheduleKind::Schedule, 1, 250, 10).await.unwrap(),
            ["11", "10"]
        );
        assert_eq!(
            s.get_due(ScheduleKind::Schedule, 1, 250, 1).await.unwrap(),
            ["11"]
        );
        assert_eq!(s.count(ScheduleKind::Schedule, 1).await.unwrap(), 2);
        assert_eq!(s.members(ScheduleKind::Retry, 1).await.unwrap(), ["13"]);
        assert_eq!(s.next_due(&[1]).await.unwrap(), NextDue::At(10));
        assert_eq!(s.next_due(&[2]).await.unwrap(), NextDue::At(50));
        assert_eq!(s.due_buckets(60, &[1, 2]).await.unwrap(), Some(vec![1, 2]));
        assert_eq!(s.due_buckets(20, &[1, 2]).await.unwrap(), Some(vec![1]));

        s.remove_from_all(1, "13").await.unwrap();
        s.remove_from_schedule(1, "11").await.unwrap();
        assert_eq!(
            s.get_due(ScheduleKind::Schedule, 1, 1000, 10)
                .await
                .unwrap(),
            ["10"]
        );
        s.remove_from_schedule(1, "10").await.unwrap();
        s.remove_from_schedule(2, "12").await.unwrap();
        assert_eq!(s.next_due(&[1, 2]).await.unwrap(), NextDue::Nothing);
        assert!(s.add_to_schedule(1, "not-a-number", 1).await.is_err());
    }

    #[tokio::test]
    async fn the_dispatch_lock_is_exclusive_until_released() {
        let s = schedule().await;
        let held = s.try_lock(4, "lock-unit-1", 30).await.expect("first");
        assert_eq!(held.key, "seq:dispatch:{4}:lock:lock-unit-1");
        match s.try_lock(4, "lock-unit-1", 30).await {
            Err(ScheduleError::LockAcquisition { key }) => assert_eq!(key, held.key),
            Err(e) => panic!("unexpected error {e:?}"),
            Ok(_) => panic!("lock taken twice"),
        }
        drop(held);
        assert!(s.try_lock(4, "lock-unit-1", 30).await.is_ok());
    }
}
