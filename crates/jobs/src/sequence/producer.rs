//! The sequence producer (`apps/worker/src/sequence-scheduler/worker-producer.ts`,
//! spec 3.1-3.2): claims due dispatches from the schedule and retry sets
//! of each assigned bucket and publishes them to the `sequenceScheduler`
//! queue with job id `sequence-<dispatchId>`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::{JobOptions, NewJob};
use crate::queue::QueueName;
use crate::sequence::buckets::{SCHEDULER_BUCKET_RANGE_ENV, assigned_buckets};
use crate::sequence::consumer::SequenceJobData;
use crate::sequence::schedule::{
    DispatchSchedule, NextDue, ScheduleEntry, ScheduleError, ScheduleKind,
};
use crate::sequence::tick::{
    EnvValue, MIN_SCHEDULER_TICK_INTERVAL_MS, SCHEDULER_TICK_INTERVAL_ENV, resolve_tick_interval_ms,
};
use crate::shutdown::CancelToken;

/// Default claim limit per set and tick (`getDue(..., 100)`).
pub const DEFAULT_CLAIM_LIMIT: u32 = 100;
/// Default dispatch lock TTL (30 s).
pub const DEFAULT_LOCK_TTL_MS: u64 = 30_000;

/// Constructor options (`Partial<SchedulerWorkerConfig>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerWorkerOptions {
    /// `None` = `assigned_buckets(SCHEDULER_BUCKET_RANGE)`.
    pub buckets: Option<Vec<u32>>,
    pub claim_limit: Option<u32>,
    pub lock_ttl_ms: Option<u64>,
    /// `None` or `Some(0)` = resolve from env (`resolve_tick_interval_ms`).
    pub tick_interval_ms: Option<u64>,
}

/// The resolved config (`SchedulerWorker.config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerWorkerConfig {
    pub buckets: Vec<u32>,
    pub claim_limit: u32,
    pub lock_ttl_ms: u64,
    pub tick_interval_ms: u64,
}

/// `{ id, workspaceId }` of a dispatch that is still pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDispatchRef {
    pub id: String,
    pub workspace_id: String,
}

/// `sequenceDispatchRepository.listPendingWorkspaceIds(ids)`.
pub trait PendingDispatchLookup: Send + Sync {
    fn list_pending_workspace_ids<'a>(
        &'a self,
        dispatch_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>>;
}

/// What `start` acquires once: the schedule handle
/// (`sequenceConnections.useExisting` + `SchedulerClient`) and the typed
/// `sequenceScheduler` queue (`getSequenceSchedulerQueue`, `None` =
/// unavailable).
pub trait ProducerBackend: Send + Sync {
    fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>>;
    fn sequence_queue(&self) -> BoxFuture<'_, Result<Option<Arc<dyn Enqueuer>>, JobsError>>;
}

/// Collaborators of the producer.
#[derive(Clone)]
pub struct SchedulerWorkerDeps {
    pub backend: Arc<dyn ProducerBackend>,
    pub pending: Arc<dyn PendingDispatchLookup>,
    pub clock: Arc<dyn Clock>,
    /// `isShutdownRequested()`.
    pub shutdown_requested: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// One claimed dispatch and the set it was claimed from (for re-insert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedDispatch {
    pub dispatch_id: String,
    pub source: ScheduleKind,
}

/// Per-bucket set sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BucketStats {
    pub schedule: u64,
    pub retry: u64,
}

/// `getHealth()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerHealth {
    pub running: bool,
    pub buckets: Vec<u32>,
    pub stats: BTreeMap<u32, BucketStats>,
}

/// `SchedulerWorker`.
pub struct SchedulerWorker {
    config: SchedulerWorkerConfig,
    deps: SchedulerWorkerDeps,
    schedule: Mutex<Option<Arc<dyn DispatchSchedule>>>,
    queue: Mutex<Option<Arc<dyn Enqueuer>>>,
    running: AtomicBool,
    timers: Mutex<BTreeSet<u32>>,
    stop: Mutex<CancelToken>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl SchedulerWorker {
    /// `new SchedulerWorker(options)`; `env` is the process env map (read
    /// for `SCHEDULER_TICK_INTERVAL_MS` and `SCHEDULER_BUCKET_RANGE`).
    pub fn new(
        options: SchedulerWorkerOptions,
        env: &BTreeMap<String, String>,
        deps: SchedulerWorkerDeps,
    ) -> Arc<SchedulerWorker> {
        let buckets = options.buckets.clone().unwrap_or_else(|| {
            assigned_buckets(env.get(SCHEDULER_BUCKET_RANGE_ENV).map(String::as_str))
        });
        let tick = resolve_tick_interval_ms(
            options.tick_interval_ms,
            env.get(SCHEDULER_TICK_INTERVAL_ENV)
                .map(|v| EnvValue::Text(v.as_str())),
        );
        Arc::new(SchedulerWorker {
            config: SchedulerWorkerConfig {
                buckets,
                claim_limit: options
                    .claim_limit
                    .filter(|v| *v != 0)
                    .unwrap_or(DEFAULT_CLAIM_LIMIT),
                lock_ttl_ms: options
                    .lock_ttl_ms
                    .filter(|v| *v != 0)
                    .unwrap_or(DEFAULT_LOCK_TTL_MS),
                tick_interval_ms: tick,
            },
            deps,
            schedule: Mutex::new(None),
            queue: Mutex::new(None),
            running: AtomicBool::new(false),
            timers: Mutex::new(BTreeSet::new()),
            stop: Mutex::new(CancelToken::new()),
            task: Mutex::new(None),
        })
    }

    /// Test hook (the TypeScript `makeReadyWorker`): a worker marked running
    /// with `schedule` and `queue` already attached and no bucket task.
    pub fn ready(
        options: SchedulerWorkerOptions,
        env: &BTreeMap<String, String>,
        deps: SchedulerWorkerDeps,
        schedule: Arc<dyn DispatchSchedule>,
        queue: Arc<dyn Enqueuer>,
    ) -> Arc<SchedulerWorker> {
        let worker = SchedulerWorker::new(options, env, deps);
        *lock(&worker.schedule) = Some(schedule);
        *lock(&worker.queue) = Some(queue);
        worker.running.store(true, Ordering::SeqCst);
        worker
    }

    pub fn config(&self) -> &SchedulerWorkerConfig {
        &self.config
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Buckets that have a tick task (the `timers` map keys), sorted.
    pub fn timer_buckets(&self) -> Vec<u32> {
        lock(&self.timers).iter().copied().collect()
    }

    fn schedule(&self) -> Option<Arc<dyn DispatchSchedule>> {
        lock(&self.schedule).clone()
    }

    fn queue(&self) -> Option<Arc<dyn Enqueuer>> {
        lock(&self.queue).clone()
    }

    /// Acquires the schedule and the queue once (a second `start` does
    /// nothing), then starts one tick task per bucket, unless shutdown was
    /// requested meanwhile. A missing queue is
    /// `JobsError::Unavailable("Sequence scheduler queue is unavailable")`.
    ///
    /// One task polls every assigned bucket: it sleeps until the next due
    /// entry (or an add) when the schedule can tell, else for the tick
    /// interval (spec 3.2: no 256 timers, no hot polling).
    pub async fn start(self: &Arc<Self>) -> Result<(), JobsError> {
        if self.is_running() {
            return Ok(());
        }
        if self.schedule().is_none() {
            let schedule = self.deps.backend.connect_schedule().await?;
            *lock(&self.schedule) = Some(schedule);
        }
        if self.queue().is_none() {
            let Some(queue) = self.deps.backend.sequence_queue().await? else {
                return Err(JobsError::Unavailable(
                    "Sequence scheduler queue is unavailable".to_owned(),
                ));
            };
            *lock(&self.queue) = Some(queue);
        }
        // Shutdown may have run stop() while the schedule was connecting.
        if (self.deps.shutdown_requested)() {
            tracing::warn!("Shutdown requested during startup; not starting scheduler worker");
            return Ok(());
        }
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        tracing::info!(
            tick_interval_ms = self.config.tick_interval_ms,
            bucket_count = self.config.buckets.len(),
            "Starting sequence producer bucket polling"
        );
        let token = CancelToken::new();
        *lock(&self.stop) = token.clone();
        lock(&self.timers).extend(self.config.buckets.iter().copied());
        let worker = Arc::clone(self);
        *lock(&self.task) = Some(tokio::spawn(async move { worker.poll(token).await }));
        Ok(())
    }

    async fn poll(self: Arc<Self>, stop: CancelToken) {
        let tick = Duration::from_millis(self.config.tick_interval_ms.max(1));
        while !stop.is_cancelled() {
            let Some(schedule) = self.schedule() else {
                break;
            };
            let signal = schedule.change_signal();
            let notified = signal.as_ref().map(|s| s.notified());
            tokio::pin!(notified);
            if let Some(n) = notified.as_mut().as_pin_mut() {
                n.enable();
            }
            let now = self.deps.clock.now_ms();
            let due = match schedule.due_buckets(now, &self.config.buckets).await {
                Ok(Some(list)) => list,
                Ok(None) => self.config.buckets.clone(),
                Err(err) => {
                    tracing::error!(err = %err, "Error reading due buckets");
                    self.config.buckets.clone()
                }
            };
            for bucket in due {
                if stop.is_cancelled() {
                    return;
                }
                self.process_bucket(bucket).await;
            }
            let wait = match schedule.next_due(&self.config.buckets).await {
                Ok(NextDue::Nothing) => None,
                Ok(NextDue::At(at)) => {
                    let now = self.deps.clock.now_ms();
                    let until = u64::try_from(at.saturating_sub(now)).unwrap_or(0);
                    // An entry due now but not claimed (locked): retry soon.
                    let floor = MIN_SCHEDULER_TICK_INTERVAL_MS.min(self.config.tick_interval_ms);
                    Some(Duration::from_millis(until.max(floor)).min(tick))
                }
                Ok(NextDue::Unknown) | Err(_) => Some(tick),
            };
            let wait = match (&wait, &signal) {
                // Nothing scheduled and no signal: keep the tick contract.
                (None, None) => Some(tick),
                _ => wait,
            };
            tokio::select! {
                () = sleep_opt(wait) => {}
                () = async {
                    match notified.as_mut().as_pin_mut() {
                        Some(n) => n.await,
                        None => std::future::pending().await,
                    }
                } => {}
                () = stop.cancelled() => break,
            }
        }
    }

    /// Stops the tick tasks (idempotent) and waits for a running tick to
    /// finish publishing what it claimed.
    pub async fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        lock(&self.timers).clear();
        lock(&self.stop).cancel();
        let task = lock(&self.task).take();
        if let Some(task) = task {
            let _ = task.await;
        }
        *lock(&self.queue) = None;
    }

    /// One tick of one bucket: `get_due` on both sets; nothing due → return
    /// before locking; else claim each id under its dispatch lock
    /// (`lock_ttl_ms / 1000` s, remove from its set), then publish.
    pub async fn process_bucket(&self, bucket: u32) {
        let Some(schedule) = self.schedule() else {
            return;
        };
        let now = self.deps.clock.now_ms();
        let limit = self.config.claim_limit;
        let due = async {
            let scheduled = schedule
                .get_due(ScheduleKind::Schedule, bucket, now, limit)
                .await?;
            let retries = schedule
                .get_due(ScheduleKind::Retry, bucket, now, limit)
                .await?;
            Ok::<_, ScheduleError>((scheduled, retries))
        };
        let (scheduled, retries) = match due.await {
            Ok(due) => due,
            Err(err) => {
                tracing::error!(err = %err, bucket, "Error processing bucket");
                return;
            }
        };
        if scheduled.is_empty() && retries.is_empty() {
            return;
        }
        let mut claimed = self
            .claim(schedule.as_ref(), bucket, scheduled, ScheduleKind::Schedule)
            .await;
        claimed.extend(
            self.claim(schedule.as_ref(), bucket, retries, ScheduleKind::Retry)
                .await,
        );
        if claimed.is_empty() {
            return;
        }
        self.publish_dispatches(bucket, &claimed).await;
    }

    async fn claim(
        &self,
        schedule: &dyn DispatchSchedule,
        bucket: u32,
        ids: Vec<String>,
        source: ScheduleKind,
    ) -> Vec<ClaimedDispatch> {
        let ttl_secs = self.config.lock_ttl_ms / 1000;
        let mut claimed = Vec::with_capacity(ids.len());
        for dispatch_id in ids {
            let result = async {
                let _lock = schedule.try_lock(bucket, &dispatch_id, ttl_secs).await?;
                match source {
                    ScheduleKind::Schedule => {
                        schedule.remove_from_schedule(bucket, &dispatch_id).await
                    }
                    ScheduleKind::Retry => schedule.remove_from_retry(bucket, &dispatch_id).await,
                }
            }
            .await;
            match result {
                Ok(()) => claimed.push(ClaimedDispatch {
                    dispatch_id,
                    source,
                }),
                Err(err) => {
                    tracing::debug!(err = %err, dispatch_id = %dispatch_id, bucket, "Dispatch claim skipped");
                }
            }
        }
        claimed
    }

    /// Publishes the claimed dispatches that still have a pending row in one
    /// `add_bulk`; on failure logs `Failed to publish ...` `{bucket, count}`
    /// and re-inserts them (schedule entries with `batch_add_to_schedule`,
    /// retry entries with `add_to_retry(.., now)`); a failed re-insert logs
    /// `Failed to re-insert ...` `{err, bucket, dispatch_ids}`.
    pub async fn publish_dispatches(&self, bucket: u32, claimed: &[ClaimedDispatch]) {
        if let Err(err) = self.publish(bucket, claimed).await {
            tracing::error!(
                err = %err,
                bucket,
                count = claimed.len(),
                "Failed to publish claimed dispatches; re-inserting for retry on next tick"
            );
            self.reinsert(bucket, claimed).await;
        }
    }

    async fn publish(&self, bucket: u32, claimed: &[ClaimedDispatch]) -> Result<(), JobsError> {
        let ids: Vec<String> = claimed.iter().map(|c| c.dispatch_id.clone()).collect();
        let pending = self.deps.pending.list_pending_workspace_ids(&ids).await?;
        let claimed_at = self.deps.clock.now_ms();
        let jobs: Vec<NewJob> = claimed
            .iter()
            .filter_map(|c| {
                let workspace_id = pending
                    .iter()
                    .find(|p| p.id == c.dispatch_id)?
                    .workspace_id
                    .clone();
                let data = SequenceJobData {
                    bucket,
                    claimed_at: Some(claimed_at),
                    dispatch_id: c.dispatch_id.clone(),
                    workspace_id,
                };
                Some(NewJob {
                    name: c.dispatch_id.clone(),
                    data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
                    opts: JobOptions {
                        job_id: Some(format!("sequence-{}", c.dispatch_id)),
                        ..JobOptions::default()
                    },
                })
            })
            .collect();
        if jobs.is_empty() {
            return Ok(());
        }
        let Some(queue) = self.queue() else {
            return Err(JobsError::Unavailable(
                "Sequence scheduler queue is unavailable".to_owned(),
            ));
        };
        queue.add_bulk(QueueName::SequenceScheduler, jobs).await?;
        Ok(())
    }

    async fn reinsert(&self, bucket: u32, claimed: &[ClaimedDispatch]) {
        let Some(schedule) = self.schedule() else {
            return;
        };
        let now = self.deps.clock.now_ms();
        let entries: Vec<ScheduleEntry> = claimed
            .iter()
            .filter(|c| c.source == ScheduleKind::Schedule)
            .map(|c| ScheduleEntry {
                bucket,
                dispatch_id: c.dispatch_id.clone(),
                at_ms: now,
            })
            .collect();
        let schedule_ids: Vec<String> = entries.iter().map(|e| e.dispatch_id.clone()).collect();
        let mut failed: Vec<(ScheduleError, Vec<String>)> = Vec::new();
        if let Err(err) = schedule.batch_add_to_schedule(entries).await {
            failed.push((err, schedule_ids));
        }
        for retry in claimed.iter().filter(|c| c.source == ScheduleKind::Retry) {
            if let Err(err) = schedule.add_to_retry(bucket, &retry.dispatch_id, now).await {
                failed.push((err, vec![retry.dispatch_id.clone()]));
            }
        }
        if let Some((first, _)) = failed.first() {
            let dispatch_ids: Vec<String> =
                failed.iter().flat_map(|(_, ids)| ids.clone()).collect();
            tracing::error!(
                err = %first,
                bucket,
                dispatch_ids = ?dispatch_ids,
                "Failed to re-insert claimed dispatches after publish failure; these dispatches are lost from scheduling until the hourly reconcile timer recovers them"
            );
        }
    }

    pub async fn health(&self) -> Result<SchedulerHealth, JobsError> {
        let schedule = self.schedule().ok_or_else(|| {
            JobsError::Unavailable("Scheduler not initialized. Call start() first.".to_owned())
        })?;
        let mut stats = BTreeMap::new();
        for &bucket in &self.config.buckets {
            let store = |e: ScheduleError| JobsError::Store(e.to_string());
            let schedule_count = schedule
                .count(ScheduleKind::Schedule, bucket)
                .await
                .map_err(store)?;
            let retry = schedule
                .count(ScheduleKind::Retry, bucket)
                .await
                .map_err(store)?;
            stats.insert(
                bucket,
                BucketStats {
                    schedule: schedule_count,
                    retry,
                },
            );
        }
        Ok(SchedulerHealth {
            running: self.is_running(),
            buckets: self.config.buckets.clone(),
            stats,
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn sleep_opt(wait: Option<Duration>) {
    match wait {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

/// `sequenceDispatchRepository.listPendingWorkspaceIds` on the P2.4
/// `SequenceDispatch` table.
pub struct SqlPendingDispatchLookup {
    pub db: Arc<chatbotx_db::Db>,
}

impl PendingDispatchLookup for SqlPendingDispatchLookup {
    fn list_pending_workspace_ids<'a>(
        &'a self,
        dispatch_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>> {
        Box::pin(async move {
            let ids: Vec<i64> = dispatch_ids
                .iter()
                .filter_map(|id| id.parse().ok())
                .collect();
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            Ok(self
                .db
                .read(move |conn| {
                    let sql = format!(
                        r#"SELECT "id", "workspaceId" FROM "SequenceDispatch"
                           WHERE "id" IN ({list}) AND "status" = 'pending'"#
                    );
                    let rows = conn
                        .prepare(&sql)?
                        .query_map([], |r| {
                            Ok(PendingDispatchRef {
                                id: r.get::<_, i64>(0)?.to_string(),
                                workspace_id: r.get::<_, i64>(1)?.to_string(),
                            })
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::sequence::sqlite_schedule::SqliteDispatchSchedule;
    use crate::store::{JobInspector, JobState, JobStore};

    struct AllPending;

    impl PendingDispatchLookup for AllPending {
        fn list_pending_workspace_ids<'a>(
            &'a self,
            dispatch_ids: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>> {
            let rows = dispatch_ids
                .iter()
                .map(|id| PendingDispatchRef {
                    id: id.clone(),
                    workspace_id: "1".to_owned(),
                })
                .collect();
            Box::pin(async move { Ok(rows) })
        }
    }

    struct Backend {
        schedule: Arc<SqliteDispatchSchedule>,
        queue: Arc<JobStore>,
    }

    impl ProducerBackend for Backend {
        fn connect_schedule(&self) -> BoxFuture<'_, Result<Arc<dyn DispatchSchedule>, JobsError>> {
            let schedule: Arc<dyn DispatchSchedule> = self.schedule.clone();
            Box::pin(async move { Ok(schedule) })
        }

        fn sequence_queue(&self) -> BoxFuture<'_, Result<Option<Arc<dyn Enqueuer>>, JobsError>> {
            let queue: Arc<dyn Enqueuer> = self.queue.clone();
            Box::pin(async move { Ok(Some(queue)) })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_due_dispatch_is_published_at_once_without_waiting_for_a_tick() {
        let db = crate::temp_db();
        let schedule = Arc::new(
            SqliteDispatchSchedule::open(Arc::clone(&db), None)
                .await
                .unwrap(),
        );
        let queue = Arc::new(JobStore::open(db, Arc::new(SystemClock)).await.unwrap());
        let worker = SchedulerWorker::new(
            SchedulerWorkerOptions {
                buckets: Some(vec![3]),
                tick_interval_ms: Some(10_000),
                ..SchedulerWorkerOptions::default()
            },
            &BTreeMap::new(),
            SchedulerWorkerDeps {
                backend: Arc::new(Backend {
                    schedule: Arc::clone(&schedule),
                    queue: Arc::clone(&queue),
                }),
                pending: Arc::new(AllPending),
                clock: Arc::new(SystemClock),
                shutdown_requested: Arc::new(|| false),
            },
        );
        worker.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let now = SystemClock.now_ms();
        schedule.add_to_schedule(3, "4242", now - 1).await.unwrap();
        let started = tokio::time::Instant::now();
        loop {
            let state = queue
                .get_state(QueueName::SequenceScheduler, "sequence-4242")
                .await
                .unwrap();
            if state == JobState::Waiting {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(2), "not published");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            schedule
                .members(ScheduleKind::Schedule, 3)
                .await
                .unwrap()
                .is_empty()
        );
        worker.stop().await;
    }
}
