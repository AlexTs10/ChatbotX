//! Worker runtime (spec 2.4-2.10, 2.13; port of
//! `apps/worker/src/lib/create-queue-worker.ts`): `WorkerSpec` (the merged
//! worker options), `Runtime::spawn_worker` (one dispatcher per queue) and
//! the worker registry of the `standalone` / `core` processes.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Notify, OnceCell, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::clock::Clock;
use crate::ctx::{BoxError, JobCtx, JobError};
use crate::enqueue::{BoxFuture, JobsError};
use crate::options::Retention;
use crate::queue::QueueName;
use crate::shutdown::{CancelToken, Shutdown};
use crate::store::{DEFAULT_LEASE_MS, JobStore, Outcome};

/// A job handler (the BullMQ processor).
pub type Handler = Arc<dyn Fn(JobCtx) -> BoxFuture<'static, Result<Value, JobError>> + Send + Sync>;

/// One failed attempt, as the `failed` listener sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedJob {
    pub queue: QueueName,
    pub job_id: String,
    pub reason: String,
    /// No retry follows.
    pub final_attempt: bool,
}

/// `onFailed(job, err)`: called after the failure commits, never awaited by
/// the dispatcher; an error is logged ("Failed-job handler threw").
pub type FailedHook =
    Arc<dyn Fn(FailedJob) -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// `limiter: { max, duration }` (fixed window, spec 2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limiter {
    pub max: u32,
    pub duration_ms: u64,
}

/// The worker options of one queue: `defaultWorkerOptions` merged with the
/// caller's options (caller wins).
#[derive(Clone)]
pub struct WorkerSpec {
    pub queue: QueueName,
    pub concurrency: u32,
    pub limiter: Option<Limiter>,
    /// `lockDuration`.
    pub lease_ms: u64,
    pub stalled_interval_ms: u64,
    pub max_stalled: u32,
    pub remove_on_complete: Retention,
    pub remove_on_fail: Retention,
    /// `None` = the default failed logger ("Job has failed").
    pub on_failed: Option<FailedHook>,
}

impl WorkerSpec {
    /// The defaults (spec 1.1): concurrency 5, `KeepCount(1000)` /
    /// `KeepCount(5000)`, lease 30 000 ms, stalled interval 30 000 ms,
    /// max stalled 1, no limiter, default failed logger.
    pub fn new(queue: QueueName) -> WorkerSpec {
        WorkerSpec {
            queue,
            concurrency: 5,
            limiter: None,
            lease_ms: DEFAULT_LEASE_MS,
            stalled_interval_ms: 30_000,
            max_stalled: 1,
            remove_on_complete: Retention::KeepCount(1000),
            remove_on_fail: Retention::KeepCount(5000),
            on_failed: None,
        }
    }

    pub fn concurrency(mut self, concurrency: u32) -> WorkerSpec {
        self.concurrency = concurrency.max(1);
        self
    }

    pub fn limiter(mut self, limiter: Limiter) -> WorkerSpec {
        self.limiter = Some(limiter);
        self
    }

    pub fn lease_ms(mut self, lease_ms: u64) -> WorkerSpec {
        self.lease_ms = lease_ms.max(1);
        self
    }

    /// `stalledInterval`.
    pub fn stalled_interval_ms(mut self, interval_ms: u64) -> WorkerSpec {
        self.stalled_interval_ms = interval_ms.max(1);
        self
    }

    /// `removeOnComplete` / `removeOnFail` worker defaults.
    pub fn retention(mut self, on_complete: Retention, on_fail: Retention) -> WorkerSpec {
        self.remove_on_complete = on_complete;
        self.remove_on_fail = on_fail;
        self
    }

    /// Replaces the default failed logger.
    pub fn on_failed(mut self, hook: FailedHook) -> WorkerSpec {
        self.on_failed = Some(hook);
        self
    }

    /// The listeners a spawned worker attaches (`"error"`, `"failed"`).
    pub fn listeners(&self) -> Vec<&'static str> {
        vec!["error", "failed"]
    }
}

impl std::fmt::Debug for WorkerSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerSpec")
            .field("queue", &self.queue)
            .field("concurrency", &self.concurrency)
            .field("limiter", &self.limiter)
            .field("lease_ms", &self.lease_ms)
            .field("stalled_interval_ms", &self.stalled_interval_ms)
            .field("max_stalled", &self.max_stalled)
            .field("remove_on_complete", &self.remove_on_complete)
            .field("remove_on_fail", &self.remove_on_fail)
            .field("on_failed", &self.on_failed.is_some())
            .finish()
    }
}

/// Fixed-window rate limiter (BullMQ `rateLimiterKey`: the window starts at
/// the first job started in it).
struct WindowLimiter {
    limit: Limiter,
    window: Mutex<(Option<Instant>, u32)>,
}

impl WindowLimiter {
    /// How many jobs may start now, or how long to wait.
    fn allowance(&self) -> Result<u32, Duration> {
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let length = Duration::from_millis(self.limit.duration_ms);
        if let Some(start) = window.0
            && Instant::now() >= start + length
        {
            *window = (None, 0);
        }
        let left = self.limit.max.saturating_sub(window.1);
        match (left, window.0) {
            (0, Some(start)) => Err((start + length).saturating_duration_since(Instant::now())),
            (0, None) => Err(length),
            (n, _) => Ok(n),
        }
    }

    fn record(&self, started: u32) {
        if started == 0 {
            return;
        }
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if window.0.is_none() {
            window.0 = Some(Instant::now());
        }
        window.1 = window.1.saturating_add(started);
    }
}

struct WorkerInner {
    spec: WorkerSpec,
    handler: Handler,
    runtime: Arc<RuntimeInner>,
    permits: Arc<Semaphore>,
    closing: CancelToken,
    close_count: AtomicU32,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    limiter: Option<WindowLimiter>,
}

/// A running worker of one queue.
pub struct WorkerHandle {
    inner: Arc<WorkerInner>,
}

impl WorkerHandle {
    pub fn queue(&self) -> QueueName {
        self.inner.spec.queue
    }

    pub fn spec(&self) -> &WorkerSpec {
        &self.inner.spec
    }

    /// `worker.close()`: stop claiming, wait for running jobs.
    pub fn close(&self) -> BoxFuture<'static, ()> {
        Box::pin(Arc::clone(&self.inner).close())
    }

    /// Test hook: how many times the worker was closed.
    pub fn close_count(&self) -> u32 {
        self.inner.close_count.load(Ordering::SeqCst)
    }

    /// The `failed` listener: the custom `on_failed` hook, else the error
    /// log "Job has failed" (fields `queue`, `job_id`, `err`). A hook error
    /// is logged as "Failed-job handler threw". Runs detached; the handle
    /// lets a test wait for it.
    pub fn report_failed(&self, job: FailedJob) -> JoinHandle<()> {
        report_failed(&self.inner.spec, job)
    }

    /// The `error` listener: logs "Queue worker error" (fields `queue`,
    /// `err`); never requests shutdown.
    pub fn report_error(&self, err: &JobsError) {
        report_error(self.inner.spec.queue, err);
    }
}

fn report_failed(spec: &WorkerSpec, job: FailedJob) -> JoinHandle<()> {
    let hook = spec.on_failed.clone();
    tokio::spawn(async move {
        match hook {
            Some(hook) => {
                let queue = job.queue;
                let job_id = job.job_id.clone();
                if let Err(err) = hook(job).await {
                    tracing::error!(
                        queue = %queue,
                        job_id = %job_id,
                        err = %err,
                        "Failed-job handler threw"
                    );
                }
            }
            None => {
                tracing::error!(
                    queue = %job.queue,
                    job_id = %job.job_id,
                    err = %job.reason,
                    "Job has failed"
                );
            }
        }
    })
}

fn report_error(queue: QueueName, err: &JobsError) {
    tracing::error!(queue = %queue, err = %err, "Queue worker error");
}

impl WorkerInner {
    async fn close(self: Arc<Self>) {
        self.closing.cancel();
        let dispatcher = self
            .dispatcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(dispatcher) = dispatcher {
            let _ = dispatcher.await;
        }
        // Every permit back = no job of this worker runs any more.
        let _ = self.permits.acquire_many(self.spec.concurrency).await;
        self.close_count.fetch_add(1, Ordering::SeqCst);
    }

    fn stopping(&self) -> bool {
        self.closing.is_cancelled() || self.runtime.shutdown_token.is_cancelled()
    }

    async fn stop_signal(&self) {
        tokio::select! {
            () = self.closing.cancelled() => {}
            () = self.runtime.shutdown_token.cancelled() => {}
        }
    }

    /// Sleeps `duration` or until the worker stops; `false` = stop.
    async fn pause(&self, duration: Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(duration) => !self.stopping(),
            () = self.stop_signal() => false,
        }
    }

    /// The dispatcher (spec 2.4, 2.10, 2.12): claims `min(free slots,
    /// limiter quota)` jobs per write transaction and waits on the queue
    /// `Notify` when nothing waits. No timer when idle.
    async fn dispatch(self: Arc<Self>) {
        let runtime = Arc::clone(&self.runtime);
        runtime.recovered().await;
        let queue = self.spec.queue;
        let store = Arc::clone(&runtime.store);
        loop {
            if self.stopping() {
                break;
            }
            let first = tokio::select! {
                permit = Arc::clone(&self.permits).acquire_owned() => match permit {
                    Ok(p) => p,
                    Err(_) => break,
                },
                () = self.stop_signal() => break,
            };
            let mut permits = vec![first];
            while let Ok(p) = Arc::clone(&self.permits).try_acquire_owned() {
                permits.push(p);
            }
            let mut want = permits.len() as u32;
            if let Some(limiter) = &self.limiter {
                match limiter.allowance() {
                    Ok(n) => want = want.min(n),
                    Err(wait) => {
                        drop(permits);
                        if !self.pause(wait).await {
                            break;
                        }
                        continue;
                    }
                }
            }
            let notify = store.signals().queue(queue);
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let jobs = match store
                .claim_with_lease(queue, &runtime.owner, want, self.spec.lease_ms)
                .await
            {
                Ok(jobs) => jobs,
                Err(err) => {
                    report_error(queue, &err);
                    drop(permits);
                    if !self.pause(Duration::from_millis(1000)).await {
                        break;
                    }
                    continue;
                }
            };
            if let Some(limiter) = &self.limiter {
                limiter.record(jobs.len() as u32);
            }
            if jobs.is_empty() {
                drop(permits);
                tokio::select! {
                    () = &mut notified => {}
                    () = self.stop_signal() => break,
                }
                continue;
            }
            permits.truncate(jobs.len());
            for (job, permit) in jobs.into_iter().zip(permits) {
                let worker = Arc::clone(&self);
                tokio::spawn(async move {
                    worker.run_job(job).await;
                    drop(permit);
                });
            }
        }
    }

    async fn run_job(self: Arc<Self>, job: JobCtx) {
        let runtime = Arc::clone(&self.runtime);
        runtime.job_started();
        let queue = self.spec.queue;
        let job_id = job.job_id.clone();
        // A panic in the handler fails the attempt instead of the process.
        let result = tokio::spawn((self.handler)(job)).await;
        let outcome = match result {
            Ok(Ok(value)) => Outcome::Completed(value),
            Ok(Err(JobError::Retryable(err))) => Outcome::Failed {
                reason: err.to_string(),
                unrecoverable: false,
            },
            Ok(Err(JobError::Unrecoverable(err))) => Outcome::Failed {
                reason: err.to_string(),
                unrecoverable: true,
            },
            Ok(Err(JobError::Delayed { run_at_ms })) => Outcome::Delayed { run_at_ms },
            Err(join) => {
                let reason = if join.is_panic() {
                    "job handler panicked".to_owned()
                } else {
                    "job handler was cancelled".to_owned()
                };
                tracing::error!(queue = %queue, job_id = %job_id, err = %reason, "Job handler crashed");
                Outcome::Failed {
                    reason,
                    unrecoverable: false,
                }
            }
        };
        let failed_reason = match &outcome {
            Outcome::Failed { reason, .. } => Some(reason.clone()),
            _ => None,
        };
        match runtime
            .store
            .finish(
                queue,
                &job_id,
                Some(&runtime.owner),
                outcome,
                self.spec.remove_on_complete,
                self.spec.remove_on_fail,
            )
            .await
        {
            Ok(Some(retry)) => {
                if let Some(reason) = failed_reason {
                    // Detached, never awaited (BullMQ does not await listeners).
                    drop(report_failed(
                        &self.spec,
                        FailedJob {
                            queue,
                            job_id,
                            reason,
                            final_attempt: !retry,
                        },
                    ));
                }
            }
            Ok(None) => {
                tracing::warn!(queue = %queue, job_id = %job_id, "Job lease lost before it finished");
            }
            Err(err) => report_error(queue, &err),
        }
        runtime.job_finished();
    }
}

/// Per-queue settings the heartbeat and the stalled recovery read.
#[derive(Clone, Copy)]
struct QueueSettings {
    queue: QueueName,
    lease_ms: u64,
    stalled_interval_ms: u64,
    max_stalled: u32,
    keep_fail: Retention,
}

struct RuntimeInner {
    store: Arc<JobStore>,
    shutdown: Arc<Shutdown>,
    shutdown_token: CancelToken,
    owner: String,
    running: AtomicUsize,
    became_active: Notify,
    settings: Mutex<Vec<QueueSettings>>,
    recovered: OnceCell<()>,
    background: AtomicBool,
    with_schedulers: AtomicBool,
}

impl RuntimeInner {
    fn settings(&self) -> Vec<QueueSettings> {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn job_started(&self) {
        if self.running.fetch_add(1, Ordering::SeqCst) == 0 {
            self.became_active.notify_one();
        }
    }

    fn job_finished(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    /// Startup stalled recovery (S9), once, before the first claim.
    async fn recovered(&self) {
        self.recovered
            .get_or_init(|| async {
                if let Err(err) = self.recover().await {
                    tracing::warn!(err = %err, "stalled job recovery failed");
                }
            })
            .await;
    }

    async fn recover(&self) -> Result<u64, JobsError> {
        let settings = self.settings();
        let stalled = self
            .store
            .recover_stalled(
                &self.owner,
                settings.iter().map(|s| (s.queue, s.max_stalled)).collect(),
                settings.iter().map(|s| (s.queue, s.keep_fail)).collect(),
            )
            .await?;
        if stalled > 0 {
            tracing::warn!(count = stalled, "recovered stalled jobs");
        }
        Ok(stalled)
    }

    /// Lease renewal (one statement per queue) every `min(lease)/2`, and the
    /// stalled check every `min(stalledInterval)`, only while a job runs.
    async fn heartbeat(self: Arc<Self>) {
        loop {
            if self.running.load(Ordering::SeqCst) == 0 {
                self.became_active.notified().await;
                continue;
            }
            let settings = self.settings();
            let renew_every = settings
                .iter()
                .map(|s| s.lease_ms / 2)
                .min()
                .unwrap_or(DEFAULT_LEASE_MS / 2)
                .max(10);
            let stalled_every = settings
                .iter()
                .map(|s| s.stalled_interval_ms)
                .min()
                .unwrap_or(30_000)
                .max(10);
            let mut since_stalled = 0u64;
            while self.running.load(Ordering::SeqCst) > 0 {
                tokio::time::sleep(Duration::from_millis(renew_every)).await;
                let leases = settings.iter().map(|s| (s.queue, s.lease_ms)).collect();
                if let Err(err) = self.store.renew_leases(&self.owner, leases).await {
                    tracing::warn!(err = %err, "lease renewal failed");
                }
                since_stalled += renew_every;
                if since_stalled >= stalled_every {
                    since_stalled = 0;
                    if let Err(err) = self.recover().await {
                        tracing::warn!(err = %err, "stalled check failed");
                    }
                }
            }
        }
    }

    /// The one timer of the process (spec 2.7, 2.12): sleeps until the next
    /// delayed job or scheduler fire, then promotes / fires in one write.
    async fn timer(self: Arc<Self>) {
        let mut generation = self.store.signals().timer();
        let clock = Arc::clone(self.store.clock());
        loop {
            if self.shutdown_token.is_cancelled() {
                break;
            }
            generation.borrow_and_update();
            let schedulers = self.with_schedulers.load(Ordering::SeqCst);
            let next = match self.store.next_wake(schedulers).await {
                Ok(next) => next,
                Err(err) => {
                    tracing::warn!(err = %err, "timer read failed");
                    if !self.sleep_or_stop(Duration::from_millis(1000)).await {
                        break;
                    }
                    continue;
                }
            };
            let now = clock.now_ms();
            if let Some(next) = next
                && next <= now
            {
                if let Err(err) = self.store.fire_due(schedulers).await {
                    tracing::warn!(err = %err, "timer tick failed");
                    if !self.sleep_or_stop(Duration::from_millis(1000)).await {
                        break;
                    }
                }
                continue;
            }
            let wait = next.map(|n| Duration::from_millis(u64::try_from(n - now).unwrap_or(0)));
            tokio::select! {
                () = sleep_opt(wait) => {}
                changed = generation.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                () = self.shutdown_token.cancelled() => break,
            }
        }
    }

    async fn sleep_or_stop(&self, duration: Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(duration) => true,
            () = self.shutdown_token.cancelled() => false,
        }
    }

    /// The hourly retention sweep (spec 2.8).
    async fn sweeper(self: Arc<Self>) {
        loop {
            if !self
                .sleep_or_stop(Duration::from_secs(RETENTION_SWEEP_SECS))
                .await
            {
                break;
            }
            if let Err(err) = self.store.sweep_retention().await {
                tracing::warn!(err = %err, "retention sweep failed");
            }
        }
    }
}

/// Interval of the retention sweep.
pub const RETENTION_SWEEP_SECS: u64 = 3600;

async fn sleep_opt(wait: Option<Duration>) {
    match wait {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

/// Owns the dispatchers of one process.
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Runtime {
    pub fn new(store: Arc<JobStore>, shutdown: Arc<Shutdown>, clock: Arc<dyn Clock>) -> Runtime {
        let _ = clock;
        let owner = format!("{:016x}", crate::rand::next_u64());
        let shutdown_token = shutdown.cancel_token();
        Runtime {
            inner: Arc::new(RuntimeInner {
                store,
                shutdown,
                shutdown_token,
                owner,
                running: AtomicUsize::new(0),
                became_active: Notify::new(),
                settings: Mutex::new(Vec::new()),
                recovered: OnceCell::new(),
                background: AtomicBool::new(false),
                with_schedulers: AtomicBool::new(false),
            }),
        }
    }

    /// The lease owner id of this process (random 64 bits).
    pub fn lease_owner(&self) -> &str {
        &self.inner.owner
    }

    /// The store the workers claim from.
    pub fn store(&self) -> &Arc<JobStore> {
        &self.inner.store
    }

    /// Jobs running now.
    pub fn running_jobs(&self) -> usize {
        self.inner.running.load(Ordering::SeqCst)
    }

    fn start_background(&self) {
        if self.inner.background.swap(true, Ordering::SeqCst) {
            return;
        }
        tokio::spawn(Arc::clone(&self.inner).timer());
        tokio::spawn(Arc::clone(&self.inner).heartbeat());
        tokio::spawn(Arc::clone(&self.inner).sweeper());
    }

    /// `createQueueWorker(queue, processor, options)`: starts the dispatcher
    /// and registers a closer named after the queue. After shutdown began:
    /// `Err(JobsError::Shutdown("Cannot start <queue> worker: shutdown
    /// already requested"))`, no worker, no closer.
    pub fn spawn_worker(
        &self,
        spec: WorkerSpec,
        handler: Handler,
    ) -> Result<WorkerHandle, JobsError> {
        let queue = spec.queue;
        if self.inner.shutdown.is_requested() {
            return Err(JobsError::Shutdown(format!(
                "Cannot start {queue} worker: shutdown already requested"
            )));
        }
        {
            let mut settings = self
                .inner
                .settings
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            settings.retain(|s| s.queue != queue);
            settings.push(QueueSettings {
                queue,
                lease_ms: spec.lease_ms,
                stalled_interval_ms: spec.stalled_interval_ms,
                max_stalled: spec.max_stalled,
                keep_fail: spec.remove_on_fail,
            });
        }
        if queue == QueueName::Schedule {
            self.inner.with_schedulers.store(true, Ordering::SeqCst);
            self.inner.store.wake(queue);
        }
        self.start_background();
        let concurrency = spec.concurrency.max(1);
        let limiter = spec.limiter.map(|limit| WindowLimiter {
            limit,
            window: Mutex::new((None, 0)),
        });
        let inner = Arc::new(WorkerInner {
            spec,
            handler,
            runtime: Arc::clone(&self.inner),
            permits: Arc::new(Semaphore::new(concurrency as usize)),
            closing: CancelToken::new(),
            close_count: AtomicU32::new(0),
            dispatcher: Mutex::new(None),
            limiter,
        });
        let dispatcher = tokio::spawn(Arc::clone(&inner).dispatch());
        *inner
            .dispatcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(dispatcher);
        let closer = Arc::clone(&inner);
        self.inner.shutdown.on_shutdown(
            queue.as_str(),
            Box::new(move || {
                Box::pin(async move {
                    closer.close().await;
                    Ok(())
                })
            }),
        );
        Ok(WorkerHandle { inner })
    }
}

/// Which process layout (spec 1.1): `worker standalone` (all consumers +
/// schedule + sequence) or `worker core` (no schedule, no sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerMode {
    Standalone,
    Core,
}

/// `lockDuration` of the integration and heavy workers (spec 1.2).
pub const LONG_LEASE_MS: u64 = 600_000;

/// The worker registry: one spec per queue that has a consumer in this repo
/// (every queue except `broadcast` and `quota`, spec 1.2). Concurrency is
/// the env default.
pub fn worker_specs() -> Vec<WorkerSpec> {
    use QueueName as Q;
    vec![
        WorkerSpec::new(Q::Integration)
            .concurrency(10)
            .lease_ms(LONG_LEASE_MS)
            .stalled_interval_ms(LONG_LEASE_MS),
        WorkerSpec::new(Q::Chat).concurrency(5),
        WorkerSpec::new(Q::AiAgent).concurrency(5),
        WorkerSpec::new(Q::Heavy)
            .concurrency(1)
            .lease_ms(LONG_LEASE_MS)
            .stalled_interval_ms(60_000),
        WorkerSpec::new(Q::Schedule).concurrency(5),
        WorkerSpec::new(Q::Trigger)
            .concurrency(100)
            .retention(Retention::Remove, Retention::KeepCount(100)),
        WorkerSpec::new(Q::Webhook)
            .concurrency(50)
            .retention(Retention::Remove, Retention::KeepCount(100)),
        WorkerSpec::new(Q::Default).concurrency(5),
        WorkerSpec::new(Q::SequenceScheduler)
            .concurrency(100)
            .retention(Retention::KeepCount(0), Retention::KeepCount(0)),
        WorkerSpec::new(Q::Notification).concurrency(10),
        WorkerSpec::new(Q::CallTranscription)
            .concurrency(1)
            .limiter(Limiter {
                max: 10,
                duration_ms: 60_000,
            }),
        WorkerSpec::new(Q::ProfileSnapshot)
            .concurrency(1)
            .limiter(Limiter {
                max: 10,
                duration_ms: 1000,
            }),
        WorkerSpec::new(Q::WhatsappVoipSignaling).concurrency(10),
        WorkerSpec::new(Q::Low).concurrency(30),
        WorkerSpec::new(Q::Leaderboard).concurrency(10),
        WorkerSpec::new(Q::MarketingMessage).concurrency(10),
        WorkerSpec::new(Q::Template).concurrency(2),
    ]
}

/// The queues whose workers `mode` starts (dev runs the same binary).
pub fn started_queues(mode: WorkerMode) -> Vec<QueueName> {
    worker_specs()
        .into_iter()
        .map(|s| s.queue)
        .filter(|q| {
            mode == WorkerMode::Standalone
                || !matches!(q, QueueName::Schedule | QueueName::SequenceScheduler)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use serde_json::json;

    use super::*;
    use crate::clock::SystemClock;
    use crate::enqueue::Enqueuer;
    use crate::options::{Backoff, JobOptions};
    use crate::schedules::{Repeat, ScheduleTemplate};
    use crate::store::{JobInspector, JobState};
    use crate::wait::JobWaiter;

    async fn setup() -> (Arc<JobStore>, Arc<Shutdown>, Runtime) {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let db = crate::temp_db();
        let store = Arc::new(JobStore::open(db, Arc::clone(&clock)).await.expect("store"));
        let shutdown = Shutdown::new(
            None,
            Arc::new(|_| {}),
            Arc::new(|| Box::pin(async { Ok(()) })),
        );
        let runtime = Runtime::new(Arc::clone(&store), Arc::clone(&shutdown), clock);
        (store, shutdown, runtime)
    }

    fn handler(f: impl Fn(JobCtx) -> Result<Value, JobError> + Send + Sync + 'static) -> Handler {
        let f = Arc::new(f);
        Arc::new(move |job| {
            let f = Arc::clone(&f);
            Box::pin(async move { f(job) })
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_worker_runs_jobs_and_reports_results() {
        let (store, shutdown, runtime) = setup().await;
        let _worker = runtime
            .spawn_worker(
                WorkerSpec::new(QueueName::Chat),
                handler(|job| Ok(json!({ "echo": job.data }))),
            )
            .expect("worker");
        let added = store
            .add(QueueName::Chat, "send", json!(7), JobOptions::default())
            .await
            .unwrap();
        let result = store
            .wait_until_finished(QueueName::Chat, &added.job_id, 5000)
            .await;
        assert_eq!(result, Ok(json!({ "echo": 7 })));
        shutdown
            .request(crate::shutdown::ShutdownReason::Sigterm, None)
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_attempt_is_retried_after_its_backoff_by_the_timer() {
        let (store, shutdown, runtime) = setup().await;
        let calls = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&calls);
        let _worker = runtime
            .spawn_worker(
                WorkerSpec::new(QueueName::Low),
                handler(move |_| {
                    if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err(JobError::retryable("first try fails"))
                    } else {
                        Ok(json!("second try"))
                    }
                }),
            )
            .expect("worker");
        let opts = JobOptions {
            job_id: Some("retry-me".to_owned()),
            attempts: Some(2),
            backoff: Some(Backoff::Fixed(50)),
            ..JobOptions::default()
        };
        store
            .add(QueueName::Low, "x", json!({}), opts)
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let state = store.get_state(QueueName::Low, "retry-me").await.unwrap();
            if state == JobState::Completed {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "still {state:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        shutdown
            .request(crate::shutdown::ShutdownReason::Sigterm, None)
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrency_bounds_the_running_jobs_and_close_drains_them() {
        let (store, _shutdown, runtime) = setup().await;
        let running = Arc::new(AtomicU32::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let (r, p) = (Arc::clone(&running), Arc::clone(&peak));
        let slow: Handler = Arc::new(move |_job| {
            let (r, p) = (Arc::clone(&r), Arc::clone(&p));
            Box::pin(async move {
                let now = r.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(Value::Null)
            })
        });
        let worker = runtime
            .spawn_worker(WorkerSpec::new(QueueName::Webhook).concurrency(2), slow)
            .expect("worker");
        for n in 0..6 {
            let opts = JobOptions {
                job_id: Some(format!("w{n}")),
                ..JobOptions::default()
            };
            store
                .add(QueueName::Webhook, "x", json!(n), opts)
                .await
                .unwrap();
        }
        let waited = store
            .wait_until_finished(QueueName::Webhook, "w5", 5000)
            .await;
        assert!(waited.is_ok(), "{waited:?}");
        worker.close().await;
        assert_eq!(running.load(Ordering::SeqCst), 0);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(worker.close_count(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_limiter_caps_starts_per_window() {
        let limiter = WindowLimiter {
            limit: Limiter {
                max: 2,
                duration_ms: 1000,
            },
            window: Mutex::new((None, 0)),
        };
        assert_eq!(limiter.allowance(), Ok(2));
        limiter.record(2);
        assert!(limiter.allowance().is_err());
        let (store, _shutdown, runtime) = setup().await;
        let starts = Arc::new(AtomicU32::new(0));
        let s = Arc::clone(&starts);
        let _worker = runtime
            .spawn_worker(
                WorkerSpec::new(QueueName::ProfileSnapshot)
                    .concurrency(5)
                    .limiter(Limiter {
                        max: 2,
                        duration_ms: 60_000,
                    }),
                handler(move |_| {
                    s.fetch_add(1, Ordering::SeqCst);
                    Ok(Value::Null)
                }),
            )
            .expect("worker");
        for n in 0..4 {
            let opts = JobOptions {
                job_id: Some(format!("p{n}")),
                ..JobOptions::default()
            };
            store
                .add(QueueName::ProfileSnapshot, "x", json!(n), opts)
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            2,
            "the window allows 2 starts"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_schedule_worker_fires_a_scheduler_job() {
        let (store, shutdown, runtime) = setup().await;
        let fired = Arc::new(AtomicU32::new(0));
        let f = Arc::clone(&fired);
        let template = ScheduleTemplate {
            name: "tick".to_owned(),
            data: json!({ "type": "tick", "data": {} }),
        };
        store
            .upsert_scheduler(
                QueueName::Schedule,
                "tick",
                &Repeat::EveryMs(60_000),
                &template,
            )
            .await
            .unwrap();
        let _worker = runtime
            .spawn_worker(
                WorkerSpec::new(QueueName::Schedule),
                handler(move |job| {
                    assert!(job.job_id.starts_with("repeat:tick:"));
                    f.fetch_add(1, Ordering::SeqCst);
                    Ok(Value::Null)
                }),
            )
            .expect("worker");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while fired.load(Ordering::SeqCst) == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "scheduler did not fire"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        shutdown
            .request(crate::shutdown::ShutdownReason::Sigterm, None)
            .await;
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_registry_covers_every_consumer_queue() {
        let specs = worker_specs();
        assert_eq!(specs.len(), 17);
        let total: u32 = specs.iter().map(|s| s.concurrency).sum();
        assert_eq!(total, 355, "spec 2.10 slot total");
        assert!(!started_queues(WorkerMode::Core).contains(&QueueName::Schedule));
    }
}
