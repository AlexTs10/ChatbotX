//! The SQLite job store (spec 2.2-2.11): the `jobs`, `job_dedup`,
//! `job_dedup_next`, `job_schedulers` and `job_meta` tables (migration
//! `crates/db/migrations/0003_jobs.sql`).
//!
//! Every write is one short `Db::write` transaction with no I/O inside.
//! Wake-up signals (queue `Notify`, timer) and job events are sent only
//! after the commit returned.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_db::{Db, DbError, WriteTx};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use crate::clock::Clock;
use crate::ctx::JobCtx;
use crate::enqueue::{BoxFuture, Enqueuer, JobsError};
use crate::options::{AddResult, Backoff, Dedup, JobOptions, NewJob, Retention};
use crate::queue::QueueName;
use crate::schedules::{CronPattern, Edition, Repeat, ScheduleTemplate};
use crate::wait::{CompletionRegistry, JobEvent, JobWaiter, WaitError};

/// BullMQ `job.getState()` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JobState {
    Waiting,
    /// A waiting job with `priority > 0`.
    Prioritized,
    Delayed,
    Active,
    Completed,
    Failed,
    /// No such job.
    Unknown,
}

impl JobState {
    /// `"waiting"`, `"prioritized"`, `"delayed"`, `"active"`, `"completed"`, `"failed"`, `"unknown"`.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Waiting => "waiting",
            JobState::Prioritized => "prioritized",
            JobState::Delayed => "delayed",
            JobState::Active => "active",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Unknown => "unknown",
        }
    }

    /// The BullMQ state of a row (`state` column + priority).
    fn of_row(state: &str, priority: u32) -> JobState {
        match state {
            "waiting" if priority > 0 => JobState::Prioritized,
            "waiting" => JobState::Waiting,
            "delayed" => JobState::Delayed,
            "active" => JobState::Active,
            "completed" => JobState::Completed,
            "failed" => JobState::Failed,
            _ => JobState::Unknown,
        }
    }
}

/// One job row as `getJob` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRecord {
    pub queue: QueueName,
    pub job_id: String,
    pub name: String,
    pub data: Value,
    pub state: JobState,
    /// Stored priority (`job.priority`; changed by `change_priority`).
    pub priority: u32,
    pub attempts_made: u32,
    pub attempts_started: u32,
    /// The options given at enqueue (`job.opts`, never changed later).
    pub opts: JobOptions,
    pub return_value: Option<Value>,
    pub failed_reason: Option<String>,
}

/// The inspection API (spec 2.11, S14) as a trait, so callers can be tested
/// with a fake that fails.
pub trait JobInspector: Send + Sync {
    /// `queue.getJob(id)`.
    fn get_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>>;

    /// `job.getState()`; a missing job is `JobState::Unknown`.
    fn get_state<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>>;

    /// `job.changePriority({ priority })`: only a waiting/prioritized job;
    /// `Ok(false)` when the job is in another state (not an error).
    fn change_priority<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>>;

    /// `queue.remove(id)`: `Ok(false)` for an active (locked) or missing job.
    fn remove<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>>;
}

/// Highest BullMQ priority (`2^21`).
pub const MAX_PRIORITY: u32 = 2_097_152;
/// Default lease of a claim (`lockDuration`).
pub const DEFAULT_LEASE_MS: u64 = 30_000;
/// The failure text of a job that stalled too often (BullMQ).
pub const STALLED_TOO_OFTEN: &str = "job stalled more than allowable limit";
/// Rows touched by one trim or promotion statement.
const BATCH: i64 = 500;
/// Finishes per (queue, state) between two retention trims.
const TRIM_EVERY: u32 = 50;
/// Last stack traces kept per job.
const STACKTRACE_LIMIT: usize = 10;

/// Wake-up signals of one store: a `Notify` per queue (a waiting row may
/// exist) and the timer generation (an earlier delayed job or scheduler).
pub(crate) struct Signals {
    queues: Vec<Notify>,
    timer: watch::Sender<u64>,
}

impl Signals {
    fn new() -> Signals {
        let (timer, _) = watch::channel(0);
        Signals {
            queues: QueueName::ALL.iter().map(|_| Notify::new()).collect(),
            timer,
        }
    }

    pub(crate) fn queue(&self, queue: QueueName) -> &Notify {
        &self.queues[queue.index()]
    }

    pub(crate) fn timer(&self) -> watch::Receiver<u64> {
        self.timer.subscribe()
    }

    fn poke_timer(&self) {
        self.timer.send_modify(|g| *g = g.wrapping_add(1));
    }
}

/// What a write transaction asks to signal after its commit.
#[derive(Debug, Default)]
pub(crate) struct Effects {
    notify: BTreeSet<QueueName>,
    timer: bool,
    events: Vec<JobEvent>,
}

/// How one claimed job ended (input of [`JobStore::finish`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Completed(Value),
    Failed { reason: String, unrecoverable: bool },
    Delayed { run_at_ms: i64 },
}

type TrimKey = (QueueName, &'static str);

/// The SQLite job store on a `chatbotx-db` handle.
pub struct JobStore {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    registry: Arc<CompletionRegistry>,
    signals: Arc<Signals>,
    edition: Edition,
    /// Finishes since the last trim and its time, per (queue, state).
    trims: Mutex<HashMap<TrimKey, (u32, i64)>>,
}

impl JobStore {
    /// Opens the store on `db` and creates the jobs tables if they are
    /// missing (the `jobs` migration).
    pub async fn open(db: Arc<Db>, clock: Arc<dyn Clock>) -> Result<JobStore, JobsError> {
        db.migrate()
            .await
            .map_err(|e| JobsError::Store(e.to_string()))?;
        Ok(JobStore {
            db,
            clock,
            registry: CompletionRegistry::new(),
            signals: Arc::new(Signals::new()),
            edition: Edition::Community,
            trims: Mutex::new(HashMap::new()),
        })
    }

    /// The edition (`NEXT_PUBLIC_EDITION`). On the community edition an add
    /// to the `quota` queue is skipped (DECISIONS P2.6: its consumer is the
    /// private enterprise worker). Default: community.
    pub fn with_edition(mut self, edition: Edition) -> JobStore {
        self.edition = edition;
        self
    }

    /// Publishes job events to `registry` instead of the store's own one.
    pub fn with_registry(mut self, registry: Arc<CompletionRegistry>) -> JobStore {
        self.registry = registry;
        self
    }

    /// The database handle.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// The registry that receives the job events of this store.
    pub fn registry(&self) -> &Arc<CompletionRegistry> {
        &self.registry
    }

    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub(crate) fn signals(&self) -> &Arc<Signals> {
        &self.signals
    }

    /// Wakes the dispatcher of `queue` and the timer (call after the commit
    /// of a transaction that used [`JobStore::add_in`]).
    pub fn wake(&self, queue: QueueName) {
        self.signals.queue(queue).notify_one();
        self.signals.poke_timer();
    }

    async fn write_retry<R, F>(&self, f: F) -> Result<R, DbError>
    where
        F: FnOnce(&mut WriteTx<'_>) -> Result<R, DbError> + Clone + Send + 'static,
        R: Send + 'static,
    {
        crate::db_retry::write(&self.db, f).await
    }

    async fn read_retry<R, F>(&self, f: F) -> Result<R, DbError>
    where
        F: FnOnce(&Connection) -> Result<R, DbError> + Clone + Send + 'static,
        R: Send + 'static,
    {
        crate::db_retry::read(&self.db, f).await
    }

    fn apply(&self, effects: Effects) {
        for queue in effects.notify {
            self.signals.queue(queue).notify_one();
        }
        if effects.timer {
            self.signals.poke_timer();
        }
        for event in effects.events {
            self.registry.publish(event);
        }
    }

    /// `add` inside a caller-owned write transaction (transactional outbox,
    /// spec 2.3). Signals are sent by the caller after commit
    /// ([`JobStore::wake`]).
    pub fn add_in(
        &self,
        tx: &mut WriteTx<'_>,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> Result<AddResult, JobsError> {
        let opts = prepare(queue, opts)?;
        if self.skips(queue) {
            return Ok(skipped(&opts));
        }
        let now = self.clock.now_ms();
        let mut effects = Effects::default();
        add_tx(tx, now, queue, name, &data, &opts, None, &mut effects).map_err(store_error)
    }

    fn skips(&self, queue: QueueName) -> bool {
        if queue == QueueName::Quota && self.edition == Edition::Community {
            tracing::debug!(queue = %queue, "quota job skipped on the community edition");
            return true;
        }
        false
    }

    /// `getJobCounts(...states)`.
    pub async fn get_job_counts(
        &self,
        queue: QueueName,
        states: &[JobState],
    ) -> Result<BTreeMap<JobState, u64>, JobsError> {
        let rows: Vec<(String, bool, u64)> = self
            .read_retry(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT state, priority > 0, count(*) FROM jobs WHERE queue = ?1 GROUP BY 1, 2",
                )?;
                let rows = stmt
                    .query_map(params![queue.as_str()], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let mut counts: BTreeMap<JobState, u64> = states.iter().map(|s| (*s, 0)).collect();
        for (state, prioritized, n) in rows {
            let state = JobState::of_row(&state, u32::from(prioritized));
            if let Some(slot) = counts.get_mut(&state) {
                *slot += n;
            }
        }
        Ok(counts)
    }

    /// `getPrioritizedCount`.
    pub async fn prioritized_count(&self, queue: QueueName) -> Result<u64, JobsError> {
        let counts = self.get_job_counts(queue, &[JobState::Prioritized]).await?;
        Ok(counts.get(&JobState::Prioritized).copied().unwrap_or(0))
    }

    /// `getDelayedCount`.
    pub async fn delayed_count(&self, queue: QueueName) -> Result<u64, JobsError> {
        let counts = self.get_job_counts(queue, &[JobState::Delayed]).await?;
        Ok(counts.get(&JobState::Delayed).copied().unwrap_or(0))
    }

    /// Promotes due delayed jobs (`run_at_ms <= now`), returns how many.
    pub async fn promote_due(&self) -> Result<u64, JobsError> {
        let now = self.clock.now_ms();
        let (count, effects) = self
            .write_retry(move |tx| {
                let mut effects = Effects::default();
                let n = promote_tx(tx, now, &mut effects)?;
                Ok((n, effects))
            })
            .await?;
        self.apply(effects);
        Ok(count)
    }

    /// Claims up to `limit` waiting jobs of `queue` in BullMQ order
    /// (`ORDER BY priority, ready_seq`) for `lease_owner` (spec 2.4).
    pub async fn claim(
        &self,
        queue: QueueName,
        lease_owner: &str,
        limit: u32,
    ) -> Result<Vec<JobCtx>, JobsError> {
        self.claim_with_lease(queue, lease_owner, limit, DEFAULT_LEASE_MS)
            .await
    }

    pub(crate) async fn claim_with_lease(
        &self,
        queue: QueueName,
        lease_owner: &str,
        limit: u32,
        lease_ms: u64,
    ) -> Result<Vec<JobCtx>, JobsError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = self.clock.now_ms();
        let owner = lease_owner.to_owned();
        let lease_until = now.saturating_add(ms_i64(lease_ms));
        let jobs = self
            .write_retry(move |tx| claim_tx(tx, queue, &owner, i64::from(limit), now, lease_until))
            .await?;
        Ok(jobs)
    }

    /// Marks a claimed job completed with its return value (spec 2.6).
    pub async fn complete(
        &self,
        queue: QueueName,
        job_id: &str,
        return_value: Value,
    ) -> Result<(), JobsError> {
        self.finish(
            queue,
            job_id,
            None,
            Outcome::Completed(return_value),
            Retention::KeepCount(1000),
            Retention::KeepCount(5000),
        )
        .await
        .map(|_| ())
    }

    /// Records one failed attempt (retry or final failure, spec 2.6).
    pub async fn fail(
        &self,
        queue: QueueName,
        job_id: &str,
        reason: &str,
        unrecoverable: bool,
    ) -> Result<(), JobsError> {
        self.finish(
            queue,
            job_id,
            None,
            Outcome::Failed {
                reason: reason.to_owned(),
                unrecoverable,
            },
            Retention::KeepCount(1000),
            Retention::KeepCount(5000),
        )
        .await
        .map(|_| ())
    }

    /// Ends one active job. `lease_owner = Some(..)` only ends the job while
    /// that owner still holds it. Returns `None` when the job was not
    /// active (lost lease), else whether a retry follows (for failures).
    pub(crate) async fn finish(
        &self,
        queue: QueueName,
        job_id: &str,
        lease_owner: Option<&str>,
        outcome: Outcome,
        keep_complete: Retention,
        keep_fail: Retention,
    ) -> Result<Option<bool>, JobsError> {
        let now = self.clock.now_ms();
        let job_id = job_id.to_owned();
        let owner = lease_owner.map(str::to_owned);
        let state = if matches!(outcome, Outcome::Completed(_)) {
            "completed"
        } else {
            "failed"
        };
        let trim = self.should_trim(queue, state, now);
        let (result, effects) = self
            .write_retry(move |tx| {
                let mut effects = Effects::default();
                let result = finish_tx(
                    tx,
                    now,
                    queue,
                    &job_id,
                    owner.as_deref(),
                    outcome,
                    keep_complete,
                    keep_fail,
                    &mut effects,
                )?;
                if trim {
                    trim_tx(tx, now, queue, state)?;
                }
                Ok((result, effects))
            })
            .await?;
        self.apply(effects);
        Ok(result)
    }

    fn should_trim(&self, queue: QueueName, state: &'static str, now: i64) -> bool {
        let mut trims = self.trims.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = trims.entry((queue, state)).or_insert((0, now));
        entry.0 += 1;
        if entry.0 >= TRIM_EVERY || now - entry.1 >= 1000 {
            *entry = (0, now);
            true
        } else {
            false
        }
    }

    /// Applies the retention rules of every queue and drops expired
    /// deduplication keys (the hourly sweep, spec 2.8).
    pub async fn sweep_retention(&self) -> Result<(), JobsError> {
        let now = self.clock.now_ms();
        self.write_retry(move |tx| {
            for queue in QueueName::ALL {
                for state in ["completed", "failed"] {
                    trim_tx(tx, now, queue, state)?;
                }
            }
            tx.execute(
                "DELETE FROM job_dedup WHERE expires_at_ms IS NOT NULL AND expires_at_ms <= ?1",
                params![now],
            )?;
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Renews the leases of `owner` on the given queues (heartbeat).
    pub(crate) async fn renew_leases(
        &self,
        owner: &str,
        queues: Vec<(QueueName, u64)>,
    ) -> Result<(), JobsError> {
        let now = self.clock.now_ms();
        let owner = owner.to_owned();
        self.write_retry(move |tx| {
            let mut stmt = tx.prepare_cached(
                "UPDATE jobs SET lease_until_ms = ?1 \
                     WHERE state = 'active' AND lease_owner = ?2 AND queue = ?3",
            )?;
            for (queue, lease_ms) in queues {
                stmt.execute(params![
                    now.saturating_add(ms_i64(lease_ms)),
                    owner,
                    queue.as_str()
                ])?;
            }
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Stalled recovery (S9): active rows of another owner, or with an
    /// expired lease, go back to the head of `waiting`; a row that stalled
    /// more than `max_stalled(queue)` times fails (`job stalled more than
    /// allowable limit`), except scheduler jobs. Returns the stalled count.
    pub(crate) async fn recover_stalled(
        &self,
        owner: &str,
        max_stalled: Vec<(QueueName, u32)>,
        keep_fail: Vec<(QueueName, Retention)>,
    ) -> Result<u64, JobsError> {
        let now = self.clock.now_ms();
        let owner = owner.to_owned();
        let (count, effects) = self
            .write_retry(move |tx| {
                let mut effects = Effects::default();
                let n = recover_tx(tx, now, &owner, &max_stalled, &keep_fail, &mut effects)?;
                Ok((n, effects))
            })
            .await?;
        self.apply(effects);
        Ok(count)
    }

    /// The earliest time the timer must wake: the next delayed job or the
    /// next scheduler fire, `None` when there is neither.
    pub(crate) async fn next_wake(&self, with_schedulers: bool) -> Result<Option<i64>, JobsError> {
        Ok(self
            .read_retry(move |conn| {
                let delayed: Option<i64> = conn.query_row(
                    "SELECT min(run_at_ms) FROM jobs WHERE state = 'delayed'",
                    [],
                    |r| r.get(0),
                )?;
                let scheduled: Option<i64> = if with_schedulers {
                    conn.query_row("SELECT min(next_run_at_ms) FROM job_schedulers", [], |r| {
                        r.get(0)
                    })?
                } else {
                    None
                };
                Ok(match (delayed, scheduled) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                })
            })
            .await?)
    }

    /// The timer tick: promotes due delayed jobs and fires due schedulers
    /// in one write transaction.
    pub(crate) async fn fire_due(&self, with_schedulers: bool) -> Result<(), JobsError> {
        let now = self.clock.now_ms();
        let effects = self
            .write_retry(move |tx| {
                let mut effects = Effects::default();
                promote_tx(tx, now, &mut effects)?;
                if with_schedulers {
                    fire_schedulers_tx(tx, now, &mut effects)?;
                }
                Ok(effects)
            })
            .await?;
        self.apply(effects);
        Ok(())
    }

    async fn add_many(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> Result<Vec<AddResult>, JobsError> {
        let mut prepared = Vec::with_capacity(jobs.len());
        for job in jobs {
            let opts = prepare(queue, job.opts)?;
            prepared.push((job.name, job.data, opts));
        }
        if self.skips(queue) {
            return Ok(prepared.iter().map(|(_, _, o)| skipped(o)).collect());
        }
        let now = self.clock.now_ms();
        let (results, effects) = self
            .write_retry(move |tx| {
                let mut effects = Effects::default();
                let mut results = Vec::with_capacity(prepared.len());
                for (name, data, opts) in &prepared {
                    results.push(add_tx(
                        tx,
                        now,
                        queue,
                        name,
                        data,
                        opts,
                        None,
                        &mut effects,
                    )?);
                }
                Ok((results, effects))
            })
            .await?;
        self.apply(effects);
        Ok(results)
    }

    async fn read_job(
        &self,
        queue: QueueName,
        job_id: &str,
    ) -> Result<Option<JobRecord>, JobsError> {
        let job_id = job_id.to_owned();
        Ok(self
            .read_retry(move |conn| read_job_conn(conn, queue, &job_id))
            .await?)
    }
}

fn store_error(e: DbError) -> JobsError {
    JobsError::Store(e.to_string())
}

fn ms_i64(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

fn skipped(opts: &JobOptions) -> AddResult {
    AddResult {
        job_id: opts.job_id.clone().unwrap_or_default(),
        created: false,
    }
}

/// The queue `defaultJobOptions` (spec 1.1): `attempts: 2`, exponential
/// 5000 ms; the VoIP signaling queue has its own (`10 x fixed 2000`,
/// `removeOn* true`).
pub fn queue_default_options(queue: QueueName) -> JobOptions {
    match queue {
        QueueName::WhatsappVoipSignaling => JobOptions {
            attempts: Some(10),
            backoff: Some(Backoff::Fixed(2000)),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        },
        _ => JobOptions {
            attempts: Some(2),
            backoff: Some(Backoff::Exponential(5000)),
            ..JobOptions::default()
        },
    }
}

/// Validation (spec 2.3 step 1) and the queue defaults under the producer
/// options.
fn prepare(queue: QueueName, opts: JobOptions) -> Result<JobOptions, JobsError> {
    if let Some(id) = &opts.job_id {
        if id.contains(':') {
            return Err(JobsError::InvalidOptions(
                "Custom Id cannot contain :".to_owned(),
            ));
        }
        if id.is_empty() {
            return Err(JobsError::InvalidOptions(
                "Custom Id cannot be empty".to_owned(),
            ));
        }
    }
    if opts.priority.is_some_and(|p| p > MAX_PRIORITY) {
        return Err(JobsError::InvalidOptions(format!(
            "Priority should be between 0 and {MAX_PRIORITY}"
        )));
    }
    if let Some(dedup) = &opts.dedup
        && dedup.id.is_empty()
    {
        return Err(JobsError::InvalidOptions(
            "Deduplication id cannot be empty".to_owned(),
        ));
    }
    Ok(JobOptions::merge(&queue_default_options(queue), &opts))
}

// ---------------------------------------------------------------------------
// Options and retention as JSON (BullMQ `job.opts` shape)
// ---------------------------------------------------------------------------

fn retention_json(r: Retention) -> Value {
    match r {
        Retention::Remove => Value::Bool(true),
        Retention::Keep => Value::Bool(false),
        Retention::KeepCount(n) => json!({ "count": n }),
        Retention::KeepAge {
            age_secs,
            count: Some(count),
        } => json!({ "age": age_secs, "count": count }),
        Retention::KeepAge {
            age_secs,
            count: None,
        } => json!({ "age": age_secs }),
    }
}

fn retention_from(v: &Value) -> Option<Retention> {
    match v {
        Value::Bool(true) => Some(Retention::Remove),
        Value::Bool(false) => Some(Retention::Keep),
        Value::Number(n) => n
            .as_u64()
            .map(|n| Retention::KeepCount(u32::try_from(n).unwrap_or(u32::MAX))),
        Value::Object(o) => {
            let count = o
                .get("count")
                .and_then(Value::as_u64)
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX));
            match o.get("age").and_then(Value::as_u64) {
                Some(age_secs) => Some(Retention::KeepAge { age_secs, count }),
                None => count.map(Retention::KeepCount),
            }
        }
        _ => None,
    }
}

pub(crate) fn opts_to_json(o: &JobOptions) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(id) = &o.job_id {
        m.insert("jobId".into(), json!(id));
    }
    if let Some(d) = o.delay_ms {
        m.insert("delay".into(), json!(d));
    }
    if let Some(p) = o.priority {
        m.insert("priority".into(), json!(p));
    }
    if let Some(a) = o.attempts {
        m.insert("attempts".into(), json!(a));
    }
    if let Some(b) = o.backoff {
        let (kind, delay) = match b {
            Backoff::Fixed(d) => ("fixed", d),
            Backoff::Exponential(d) => ("exponential", d),
        };
        m.insert("backoff".into(), json!({ "type": kind, "delay": delay }));
    }
    if let Some(r) = o.remove_on_complete {
        m.insert("removeOnComplete".into(), retention_json(r));
    }
    if let Some(r) = o.remove_on_fail {
        m.insert("removeOnFail".into(), retention_json(r));
    }
    if let Some(d) = &o.dedup {
        let mut de = serde_json::Map::new();
        de.insert("id".into(), json!(d.id));
        if let Some(ttl) = d.ttl_ms {
            de.insert("ttl".into(), json!(ttl));
        }
        if d.extend {
            de.insert("extend".into(), json!(true));
        }
        if d.replace {
            de.insert("replace".into(), json!(true));
        }
        if d.keep_last_if_active {
            de.insert("keepLastIfActive".into(), json!(true));
        }
        m.insert("deduplication".into(), Value::Object(de));
    }
    Value::Object(m)
}

pub(crate) fn opts_from_json(v: &Value) -> JobOptions {
    let u32_of = |key: &str| {
        v.get(key)
            .and_then(Value::as_u64)
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    let backoff = v.get("backoff").and_then(|b| {
        let delay = b.get("delay").and_then(Value::as_u64).unwrap_or(0);
        match b.get("type").and_then(Value::as_str) {
            Some("fixed") => Some(Backoff::Fixed(delay)),
            Some("exponential") => Some(Backoff::Exponential(delay)),
            _ => None,
        }
    });
    let dedup = v.get("deduplication").and_then(|d| {
        let flag = |k: &str| d.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(Dedup {
            id: d.get("id")?.as_str()?.to_owned(),
            ttl_ms: d.get("ttl").and_then(Value::as_u64),
            extend: flag("extend"),
            replace: flag("replace"),
            keep_last_if_active: flag("keepLastIfActive"),
        })
    });
    JobOptions {
        job_id: v.get("jobId").and_then(Value::as_str).map(str::to_owned),
        delay_ms: v.get("delay").and_then(Value::as_u64),
        priority: u32_of("priority"),
        attempts: u32_of("attempts"),
        backoff,
        remove_on_complete: v.get("removeOnComplete").and_then(retention_from),
        remove_on_fail: v.get("removeOnFail").and_then(retention_from),
        dedup,
    }
}

// ---------------------------------------------------------------------------
// Transaction bodies (run on the writer thread, no I/O)
// ---------------------------------------------------------------------------

/// Moves a counter of `job_meta` by `delta` and returns the new value.
fn bump(conn: &Connection, key: &str, delta: i64) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "UPDATE job_meta SET v = v + ?2 WHERE k = ?1 RETURNING v",
        params![key, delta],
        |r| r.get(0),
    )?)
}

fn job_state(conn: &Connection, queue: QueueName, job_id: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row(
            "SELECT state FROM jobs WHERE queue = ?1 AND job_id = ?2",
            params![queue.as_str(), job_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The live deduplication key `(owner job id, expires_at_ms)`; an expired
/// key counts as absent and is deleted.
fn dedup_key(
    conn: &Connection,
    queue: QueueName,
    dedup_id: &str,
    now: i64,
) -> Result<Option<(String, Option<i64>)>, DbError> {
    let row: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT job_id, expires_at_ms FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2",
            params![queue.as_str(), dedup_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((_, Some(expires))) if expires <= now => {
            conn.execute(
                "DELETE FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2",
                params![queue.as_str(), dedup_id],
            )?;
            Ok(None)
        }
        other => Ok(other),
    }
}

fn set_dedup_key(
    conn: &Connection,
    queue: QueueName,
    dedup_id: &str,
    job_id: &str,
    expires_at_ms: Option<i64>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO job_dedup (queue, dedup_id, job_id, expires_at_ms) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (queue, dedup_id) DO UPDATE SET job_id = excluded.job_id, expires_at_ms = excluded.expires_at_ms",
        params![queue.as_str(), dedup_id, job_id, expires_at_ms],
    )?;
    Ok(())
}

/// `setDeduplicationKey`: with the ttl of the options, if any.
fn set_key_with_ttl(
    conn: &Connection,
    queue: QueueName,
    dedup: &Dedup,
    job_id: &str,
    now: i64,
) -> Result<(), DbError> {
    let expires = ttl_of(dedup).map(|ttl| now.saturating_add(ttl));
    set_dedup_key(conn, queue, &dedup.id, job_id, expires)
}

fn ttl_of(dedup: &Dedup) -> Option<i64> {
    dedup.ttl_ms.filter(|t| *t > 0).map(ms_i64)
}

/// `storeDeduplicatedNextJob`: when the owner is active and
/// `keepLastIfActive` is set, keep this add as the next job (latest wins)
/// and persist the key. Returns whether it was stored.
#[allow(clippy::too_many_arguments)]
fn store_next(
    conn: &Connection,
    queue: QueueName,
    dedup: &Dedup,
    owner: &str,
    name: &str,
    data: &Value,
    opts: &JobOptions,
    job_id: &str,
) -> Result<bool, DbError> {
    if !dedup.keep_last_if_active {
        return Ok(false);
    }
    if job_state(conn, queue, owner)?.as_deref() != Some("active") {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO job_dedup_next (queue, dedup_id, name, data, opts, job_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (queue, dedup_id) DO UPDATE SET name = excluded.name, data = excluded.data, \
         opts = excluded.opts, job_id = excluded.job_id",
        params![
            queue.as_str(),
            dedup.id,
            name,
            data.to_string(),
            opts_to_json(opts).to_string(),
            job_id
        ],
    )?;
    conn.execute(
        "UPDATE job_dedup SET expires_at_ms = NULL WHERE queue = ?1 AND dedup_id = ?2",
        params![queue.as_str(), dedup.id],
    )?;
    Ok(true)
}

/// `deduplicateJob` in the Lua order: `None` = go on and insert the job,
/// `Some(owner)` = deduplicated onto `owner`.
#[allow(clippy::too_many_arguments)]
fn deduplicate(
    conn: &Connection,
    now: i64,
    queue: QueueName,
    dedup: &Dedup,
    job_id: &str,
    name: &str,
    data: &Value,
    opts: &JobOptions,
) -> Result<Option<String>, DbError> {
    let current = dedup_key(conn, queue, &dedup.id, now)?;
    let ttl = ttl_of(dedup);
    if dedup.replace {
        let Some((owner, _)) = current else {
            if dedup.keep_last_if_active {
                set_dedup_key(conn, queue, &dedup.id, job_id, None)?;
            } else {
                set_key_with_ttl(conn, queue, dedup, job_id, now)?;
            }
            return Ok(None);
        };
        let removed = conn.execute(
            "DELETE FROM jobs WHERE queue = ?1 AND job_id = ?2 AND state = 'delayed'",
            params![queue.as_str(), owner],
        )? > 0;
        if removed {
            if dedup.keep_last_if_active {
                set_dedup_key(conn, queue, &dedup.id, job_id, None)?;
            } else if !dedup.extend && ttl.is_some() {
                // `SET KEEPTTL`: new owner, same expiry.
                conn.execute(
                    "UPDATE job_dedup SET job_id = ?3 WHERE queue = ?1 AND dedup_id = ?2",
                    params![queue.as_str(), dedup.id, job_id],
                )?;
            } else {
                set_key_with_ttl(conn, queue, dedup, job_id, now)?;
            }
            return Ok(None);
        }
        store_next(conn, queue, dedup, &owner, name, data, opts, job_id)?;
        return Ok(Some(owner));
    }
    // deduplicateJobWithoutReplace
    if let Some(ttl) = ttl {
        if dedup.extend {
            return match current {
                Some((owner, _)) => {
                    if store_next(conn, queue, dedup, &owner, name, data, opts, job_id)? {
                        return Ok(Some(owner));
                    }
                    let expires = if dedup.keep_last_if_active {
                        None
                    } else {
                        Some(now.saturating_add(ttl))
                    };
                    set_dedup_key(conn, queue, &dedup.id, &owner, expires)?;
                    Ok(Some(owner))
                }
                None => {
                    let expires = if dedup.keep_last_if_active {
                        None
                    } else {
                        Some(now.saturating_add(ttl))
                    };
                    set_dedup_key(conn, queue, &dedup.id, job_id, expires)?;
                    Ok(None)
                }
            };
        }
    }
    match current {
        None => {
            // SET NX (with PX ttl unless keepLastIfActive).
            let expires = match ttl {
                Some(ttl) if !dedup.keep_last_if_active => Some(now.saturating_add(ttl)),
                _ => None,
            };
            set_dedup_key(conn, queue, &dedup.id, job_id, expires)?;
            Ok(None)
        }
        Some((owner, _)) => {
            store_next(conn, queue, dedup, &owner, name, data, opts, job_id)?;
            Ok(Some(owner))
        }
    }
}

/// One add (spec 2.3, steps 2-4). `scheduler_id` is set for jobs created by
/// a job scheduler (their `repeat:` ids skip the `:` check).
#[allow(clippy::too_many_arguments)]
fn add_tx(
    conn: &Connection,
    now: i64,
    queue: QueueName,
    name: &str,
    data: &Value,
    opts: &JobOptions,
    scheduler_id: Option<&str>,
    effects: &mut Effects,
) -> Result<AddResult, DbError> {
    let job_id = match &opts.job_id {
        Some(id) => id.clone(),
        None => bump(conn, "job_id", 1)?.to_string(),
    };
    // S2: the job id deduplicates across every state until the row is gone.
    if job_state(conn, queue, &job_id)?.is_some() {
        return Ok(AddResult {
            job_id,
            created: false,
        });
    }
    if let Some(dedup) = &opts.dedup
        && let Some(owner) = deduplicate(conn, now, queue, dedup, &job_id, name, data, opts)?
    {
        return Ok(AddResult {
            job_id: owner,
            created: false,
        });
    }
    insert_job(
        conn,
        now,
        queue,
        name,
        data,
        opts,
        &job_id,
        scheduler_id,
        effects,
    )?;
    Ok(AddResult {
        job_id,
        created: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn insert_job(
    conn: &Connection,
    now: i64,
    queue: QueueName,
    name: &str,
    data: &Value,
    opts: &JobOptions,
    job_id: &str,
    scheduler_id: Option<&str>,
    effects: &mut Effects,
) -> Result<(), DbError> {
    let delay = opts.delay_ms.unwrap_or(0);
    let (state, run_at) = if delay > 0 {
        ("delayed", now.saturating_add(ms_i64(delay)))
    } else {
        ("waiting", now)
    };
    let seq = bump(conn, "ready_seq", 1)?;
    let (backoff_kind, backoff_ms) = match opts.backoff {
        Some(Backoff::Fixed(d)) => (Some("fixed"), ms_i64(d)),
        Some(Backoff::Exponential(d)) => (Some("exponential"), ms_i64(d)),
        None => (None, 0),
    };
    conn.prepare_cached(
        "INSERT INTO jobs (queue, name, job_id, data, opts, state, priority, ready_seq, run_at_ms, \
         created_at_ms, attempts, backoff_kind, backoff_ms, dedup_id, scheduler_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?
    .execute(params![
        queue.as_str(),
        name,
        job_id,
        data.to_string(),
        opts_to_json(opts).to_string(),
        state,
        opts.priority.unwrap_or(0),
        seq,
        run_at,
        now,
        opts.attempts,
        backoff_kind,
        backoff_ms,
        opts.dedup.as_ref().map(|d| d.id.clone()),
        scheduler_id,
    ])?;
    if state == "waiting" {
        effects.notify.insert(queue);
    } else {
        effects.timer = true;
    }
    Ok(())
}

/// Promotes due delayed jobs to the tail of `waiting` (S4), oldest first.
fn promote_tx(conn: &Connection, now: i64, effects: &mut Effects) -> Result<u64, DbError> {
    let mut total = 0u64;
    loop {
        let due: Vec<(i64, String)> = {
            let mut stmt = conn.prepare_cached(
                "SELECT id, queue FROM jobs WHERE state = 'delayed' AND run_at_ms <= ?1 \
                 ORDER BY run_at_ms, id LIMIT ?2",
            )?;
            stmt.query_map(params![now, BATCH], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        if due.is_empty() {
            break;
        }
        let n = due.len() as i64;
        let last = bump(conn, "ready_seq", n)?;
        let mut seq = last - n;
        let mut stmt = conn.prepare_cached(
            "UPDATE jobs SET state = 'waiting', ready_seq = ?2 WHERE id = ?1 AND state = 'delayed'",
        )?;
        for (id, queue) in &due {
            seq += 1;
            stmt.execute(params![id, seq])?;
            if let Some(q) = QueueName::parse(queue) {
                effects.notify.insert(q);
            }
        }
        total += due.len() as u64;
        if n < BATCH {
            break;
        }
    }
    Ok(total)
}

fn claim_tx(
    conn: &Connection,
    queue: QueueName,
    owner: &str,
    limit: i64,
    now: i64,
    lease_until: i64,
) -> Result<Vec<JobCtx>, DbError> {
    let mut stmt = conn.prepare_cached(
        "UPDATE jobs SET state = 'active', lease_owner = ?1, lease_until_ms = ?2, processed_at_ms = ?3, \
         attempts_started = attempts_started + 1 \
         WHERE id IN (SELECT id FROM jobs WHERE queue = ?4 AND state = 'waiting' \
                      ORDER BY priority, ready_seq LIMIT ?5) \
         RETURNING id, job_id, name, data, attempts, attempts_made, attempts_started, priority, ready_seq",
    )?;
    let mut rows: Vec<(i64, JobCtx)> = stmt
        .query_map(
            params![owner, lease_until, now, queue.as_str(), limit],
            |r| {
                let data: String = r.get(3)?;
                Ok((
                    r.get::<_, i64>(8)?,
                    JobCtx {
                        id: r.get(0)?,
                        job_id: r.get(1)?,
                        name: r.get(2)?,
                        data: serde_json::from_str(&data).unwrap_or(Value::Null),
                        attempts: r.get::<_, Option<u32>>(4)?,
                        attempts_made: r.get(5)?,
                        attempts_started: r.get(6)?,
                        priority: r.get(7)?,
                        lease_token: Some(owner.to_owned()),
                    },
                ))
            },
        )?
        .collect::<Result<_, _>>()?;
    rows.sort_by_key(|(seq, job)| (job.priority, *seq));
    Ok(rows.into_iter().map(|(_, job)| job).collect())
}

/// The retention rule of a finished job: its own option, else the worker
/// default. `None` = delete the row now.
fn keep_columns(rule: Retention, now: i64) -> Option<(Option<i64>, Option<i64>)> {
    match rule {
        Retention::Remove | Retention::KeepCount(0) => None,
        Retention::Keep => Some((None, None)),
        Retention::KeepCount(n) => Some((Some(i64::from(n)), None)),
        Retention::KeepAge { age_secs, count } => Some((
            count.map(i64::from),
            Some(now.saturating_add(ms_i64(age_secs.saturating_mul(1000)))),
        )),
    }
}

/// `removeDeduplicationKeyIfNeededOnFinalization` + `requeueDeduplicatedJob`.
fn finalize_dedup(
    conn: &Connection,
    now: i64,
    queue: QueueName,
    dedup_id: Option<&str>,
    job_id: &str,
    effects: &mut Effects,
) -> Result<(), DbError> {
    let Some(dedup_id) = dedup_id else {
        return Ok(());
    };
    let key: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT job_id, expires_at_ms FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2",
            params![queue.as_str(), dedup_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match key {
        Some((_, Some(expires))) if expires <= now => {
            conn.execute(
                "DELETE FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2",
                params![queue.as_str(), dedup_id],
            )?;
        }
        Some((owner, None)) if owner == job_id => {
            conn.execute(
                "DELETE FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2",
                params![queue.as_str(), dedup_id],
            )?;
        }
        _ => {}
    }
    let next: Option<(String, String, String, String)> = conn
        .query_row(
            "DELETE FROM job_dedup_next WHERE queue = ?1 AND dedup_id = ?2 RETURNING name, data, opts, job_id",
            params![queue.as_str(), dedup_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((name, data, opts, next_id)) = next {
        let opts = opts_from_json(&serde_json::from_str(&opts).unwrap_or(Value::Null));
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        if job_state(conn, queue, &next_id)?.is_none() {
            match opts.dedup.as_ref() {
                Some(d) if d.keep_last_if_active => {
                    set_dedup_key(conn, queue, &d.id, &next_id, None)?;
                }
                Some(d) => set_key_with_ttl(conn, queue, d, &next_id, now)?,
                None => {}
            }
            insert_job(
                conn, now, queue, &name, &data, &opts, &next_id, None, effects,
            )?;
        }
    }
    Ok(())
}

/// The fields of an active row that a finish needs.
struct ActiveRow {
    id: i64,
    attempts: Option<u32>,
    attempts_made: u32,
    backoff_kind: Option<String>,
    backoff_ms: i64,
    dedup_id: Option<String>,
    opts: JobOptions,
    stacktrace: Option<String>,
}

fn active_row(
    conn: &Connection,
    queue: QueueName,
    job_id: &str,
    owner: Option<&str>,
) -> Result<Option<ActiveRow>, DbError> {
    let row = conn
        .query_row(
            "SELECT id, attempts, attempts_made, backoff_kind, backoff_ms, dedup_id, opts, stacktrace, lease_owner \
             FROM jobs WHERE queue = ?1 AND job_id = ?2 AND state = 'active'",
            params![queue.as_str(), job_id],
            |r| {
                let opts: String = r.get(6)?;
                Ok((
                    ActiveRow {
                        id: r.get(0)?,
                        attempts: r.get(1)?,
                        attempts_made: r.get(2)?,
                        backoff_kind: r.get(3)?,
                        backoff_ms: r.get(4)?,
                        dedup_id: r.get(5)?,
                        opts: opts_from_json(&serde_json::from_str(&opts).unwrap_or(Value::Null)),
                        stacktrace: r.get(7)?,
                    },
                    r.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        Some((row, holder)) if owner.is_none() || holder.as_deref() == owner => Some(row),
        _ => None,
    })
}

/// Moves a finished row to `completed` / `failed` and applies its
/// retention, or deletes it (`Remove`).
#[allow(clippy::too_many_arguments)]
fn retire(
    conn: &Connection,
    now: i64,
    id: i64,
    state: &str,
    rule: Retention,
    return_value: Option<&Value>,
    failed_reason: Option<&str>,
    stacktrace: Option<&str>,
) -> Result<(), DbError> {
    match keep_columns(rule, now) {
        None => {
            conn.execute("DELETE FROM jobs WHERE id = ?1", params![id])?;
        }
        Some((keep_count, keep_until)) => {
            conn.execute(
                "UPDATE jobs SET state = ?2, finished_at_ms = ?3, lease_owner = NULL, lease_until_ms = NULL, \
                 keep_count = ?4, keep_until_ms = ?5, return_value = ?6, failed_reason = ?7, stacktrace = ?8 \
                 WHERE id = ?1",
                params![
                    id,
                    state,
                    now,
                    keep_count,
                    keep_until,
                    return_value.map(Value::to_string),
                    failed_reason,
                    stacktrace
                ],
            )?;
        }
    }
    Ok(())
}

fn push_stacktrace(previous: Option<&str>, reason: &str) -> String {
    let mut lines: Vec<Value> = previous
        .and_then(|p| serde_json::from_str::<Vec<Value>>(p).ok())
        .unwrap_or_default();
    lines.push(Value::String(reason.to_owned()));
    if lines.len() > STACKTRACE_LIMIT {
        let drop = lines.len() - STACKTRACE_LIMIT;
        lines.drain(..drop);
    }
    Value::Array(lines).to_string()
}

#[allow(clippy::too_many_arguments)]
fn finish_tx(
    conn: &Connection,
    now: i64,
    queue: QueueName,
    job_id: &str,
    owner: Option<&str>,
    outcome: Outcome,
    keep_complete: Retention,
    keep_fail: Retention,
    effects: &mut Effects,
) -> Result<Option<bool>, DbError> {
    let Some(row) = active_row(conn, queue, job_id, owner)? else {
        return Ok(None);
    };
    match outcome {
        Outcome::Completed(value) => {
            let rule = row.opts.remove_on_complete.unwrap_or(keep_complete);
            retire(
                conn,
                now,
                row.id,
                "completed",
                rule,
                Some(&value),
                None,
                None,
            )?;
            finalize_dedup(conn, now, queue, row.dedup_id.as_deref(), job_id, effects)?;
            effects.events.push(JobEvent::Completed {
                queue,
                job_id: job_id.to_owned(),
                return_value: value,
            });
            Ok(Some(false))
        }
        Outcome::Delayed { run_at_ms } => {
            conn.execute(
                "UPDATE jobs SET state = 'delayed', run_at_ms = ?2, lease_owner = NULL, lease_until_ms = NULL \
                 WHERE id = ?1",
                params![row.id, run_at_ms],
            )?;
            effects.timer = true;
            effects.events.push(JobEvent::Delayed {
                queue,
                job_id: job_id.to_owned(),
                run_at_ms,
            });
            Ok(Some(false))
        }
        Outcome::Failed {
            reason,
            unrecoverable,
        } => {
            let made = row.attempts_made.saturating_add(1);
            let attempts = row.attempts.unwrap_or(1);
            let stack = push_stacktrace(row.stacktrace.as_deref(), &reason);
            let retry = !unrecoverable && made < attempts;
            if retry {
                let delay = match row.backoff_kind.as_deref() {
                    Some("fixed") => Backoff::Fixed(row.backoff_ms as u64).delay_ms(made),
                    Some("exponential") => {
                        Backoff::Exponential(row.backoff_ms as u64).delay_ms(made)
                    }
                    _ => 0,
                };
                if delay > 0 {
                    conn.execute(
                        "UPDATE jobs SET state = 'delayed', run_at_ms = ?2, attempts_made = ?3, \
                         failed_reason = ?4, stacktrace = ?5, lease_owner = NULL, lease_until_ms = NULL \
                         WHERE id = ?1",
                        params![row.id, now.saturating_add(ms_i64(delay)), made, reason, stack],
                    )?;
                    effects.timer = true;
                } else {
                    let seq = bump(conn, "ready_seq", 1)?;
                    conn.execute(
                        "UPDATE jobs SET state = 'waiting', ready_seq = ?2, attempts_made = ?3, \
                         failed_reason = ?4, stacktrace = ?5, lease_owner = NULL, lease_until_ms = NULL \
                         WHERE id = ?1",
                        params![row.id, seq, made, reason, stack],
                    )?;
                    effects.notify.insert(queue);
                }
            } else {
                conn.execute(
                    "UPDATE jobs SET attempts_made = ?2 WHERE id = ?1",
                    params![row.id, made],
                )?;
                let rule = row.opts.remove_on_fail.unwrap_or(keep_fail);
                retire(
                    conn,
                    now,
                    row.id,
                    "failed",
                    rule,
                    None,
                    Some(&reason),
                    Some(&stack),
                )?;
                finalize_dedup(conn, now, queue, row.dedup_id.as_deref(), job_id, effects)?;
            }
            effects.events.push(JobEvent::Failed {
                queue,
                job_id: job_id.to_owned(),
                reason,
                final_attempt: !retry,
            });
            Ok(Some(retry))
        }
    }
}

/// Per-job retention (DECISIONS P2.6): a row is deleted only by its own
/// rule (more than `keep_count` newer finished rows, or past
/// `keep_until_ms`).
fn trim_tx(conn: &Connection, now: i64, queue: QueueName, state: &str) -> Result<(), DbError> {
    conn.prepare_cached(
        "DELETE FROM jobs WHERE id IN (\
           SELECT id FROM (\
             SELECT id, keep_count, row_number() OVER (ORDER BY finished_at_ms DESC, id DESC) AS rn \
             FROM jobs WHERE queue = ?1 AND state = ?2) \
           WHERE keep_count IS NOT NULL AND rn > keep_count LIMIT ?3)",
    )?
    .execute(params![queue.as_str(), state, BATCH])?;
    conn.prepare_cached(
        "DELETE FROM jobs WHERE id IN (\
           SELECT id FROM jobs WHERE queue = ?1 AND state = ?2 \
           AND keep_until_ms IS NOT NULL AND keep_until_ms <= ?3 LIMIT ?4)",
    )?
    .execute(params![queue.as_str(), state, now, BATCH])?;
    Ok(())
}

/// An active row found by the stalled recovery.
struct StalledRow {
    id: i64,
    queue: String,
    job_id: String,
    stalled_count: u32,
    scheduler_id: Option<String>,
    dedup_id: Option<String>,
    opts: String,
}

fn recover_tx(
    conn: &Connection,
    now: i64,
    owner: &str,
    max_stalled: &[(QueueName, u32)],
    keep_fail: &[(QueueName, Retention)],
    effects: &mut Effects,
) -> Result<u64, DbError> {
    let stalled: Vec<StalledRow> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, queue, job_id, stalled_count, scheduler_id, dedup_id, opts FROM jobs \
             WHERE state = 'active' AND (lease_owner IS NOT ?1 OR lease_until_ms < ?2)",
        )?;
        stmt.query_map(params![owner, now], |r| {
            Ok(StalledRow {
                id: r.get(0)?,
                queue: r.get(1)?,
                job_id: r.get(2)?,
                stalled_count: r.get(3)?,
                scheduler_id: r.get(4)?,
                dedup_id: r.get(5)?,
                opts: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?
    };
    for row in &stalled {
        let StalledRow {
            id,
            queue: queue_name,
            job_id,
            stalled_count: count,
            scheduler_id,
            dedup_id,
            opts,
        } = row;
        let Some(queue) = QueueName::parse(queue_name) else {
            continue;
        };
        let limit = max_stalled
            .iter()
            .find(|(q, _)| *q == queue)
            .map_or(1, |(_, m)| *m);
        let count = count.saturating_add(1);
        effects.events.push(JobEvent::Stalled {
            queue,
            job_id: job_id.clone(),
        });
        if count > limit && scheduler_id.is_none() {
            let opts = opts_from_json(&serde_json::from_str(opts).unwrap_or(Value::Null));
            let default = keep_fail
                .iter()
                .find(|(q, _)| *q == queue)
                .map_or(Retention::KeepCount(5000), |(_, r)| *r);
            conn.execute(
                "UPDATE jobs SET stalled_count = ?2 WHERE id = ?1",
                params![id, count],
            )?;
            retire(
                conn,
                now,
                *id,
                "failed",
                opts.remove_on_fail.unwrap_or(default),
                None,
                Some(STALLED_TOO_OFTEN),
                None,
            )?;
            finalize_dedup(conn, now, queue, dedup_id.as_deref(), job_id, effects)?;
            effects.events.push(JobEvent::Failed {
                queue,
                job_id: job_id.clone(),
                reason: STALLED_TOO_OFTEN.to_owned(),
                final_attempt: true,
            });
        } else {
            // Back to the head of `waiting` (BullMQ RPUSH): runs next.
            let head = bump(conn, "head_seq", -1)?;
            conn.execute(
                "UPDATE jobs SET state = 'waiting', stalled_count = ?2, ready_seq = ?3, \
                 lease_owner = NULL, lease_until_ms = NULL WHERE id = ?1",
                params![id, count, head],
            )?;
            effects.notify.insert(queue);
        }
    }
    Ok(stalled.len() as u64)
}

/// The next fire time of a scheduler strictly after `after_ms`.
fn scheduler_next(
    pattern: Option<&str>,
    every_ms: Option<i64>,
    prev_ms: Option<i64>,
    now: i64,
) -> i64 {
    if let Some(pattern) = pattern {
        return match CronPattern::parse(pattern) {
            Ok(cron) => cron.next_after(now),
            Err(err) => {
                tracing::error!(pattern, err = %err, "invalid scheduler pattern; scheduler paused");
                i64::MAX
            }
        };
    }
    let every = every_ms.unwrap_or(1).max(1);
    match prev_ms {
        // First run at upsert time.
        None => now,
        Some(prev) => {
            let next = prev.saturating_add(every);
            if next > now {
                next
            } else {
                // Missed slots are not backfilled: re-align after `now`.
                (now.div_euclid(every) + 1) * every
            }
        }
    }
}

fn fire_schedulers_tx(conn: &Connection, now: i64, effects: &mut Effects) -> Result<(), DbError> {
    type Due = (
        String,
        String,
        Option<String>,
        Option<i64>,
        String,
        String,
        i64,
    );
    let due: Vec<Due> = {
        let mut stmt = conn.prepare_cached(
            "SELECT queue, scheduler_id, pattern, every_ms, template_name, template_data, next_run_at_ms \
             FROM job_schedulers WHERE next_run_at_ms <= ?1",
        )?;
        stmt.query_map(params![now], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<Result<_, _>>()?
    };
    for (queue_name, id, pattern, every, name, data, slot) in due {
        let Some(queue) = QueueName::parse(&queue_name) else {
            continue;
        };
        // At most one pending job per scheduler (S13).
        let pending: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM jobs WHERE queue = ?1 AND scheduler_id = ?2 AND state = 'waiting')",
            params![queue_name, id],
            |r| r.get(0),
        )?;
        if !pending {
            let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
            let opts = JobOptions {
                job_id: Some(format!("repeat:{id}:{slot}")),
                ..queue_default_options(queue)
            };
            add_tx(conn, now, queue, &name, &data, &opts, Some(&id), effects)?;
        }
        let next = scheduler_next(pattern.as_deref(), every, Some(slot), now);
        conn.execute(
            "UPDATE job_schedulers SET next_run_at_ms = ?3, iteration = iteration + 1 \
             WHERE queue = ?1 AND scheduler_id = ?2",
            params![queue_name, id, next],
        )?;
    }
    Ok(())
}

fn read_job_conn(
    conn: &Connection,
    queue: QueueName,
    job_id: &str,
) -> Result<Option<JobRecord>, DbError> {
    Ok(conn
        .query_row(
            "SELECT name, data, state, priority, attempts_made, attempts_started, opts, return_value, failed_reason \
             FROM jobs WHERE queue = ?1 AND job_id = ?2",
            params![queue.as_str(), job_id],
            |r| {
                let data: String = r.get(1)?;
                let state: String = r.get(2)?;
                let priority: u32 = r.get(3)?;
                let opts: String = r.get(6)?;
                let rv: Option<String> = r.get(7)?;
                Ok(JobRecord {
                    queue,
                    job_id: job_id.to_owned(),
                    name: r.get(0)?,
                    data: serde_json::from_str(&data).unwrap_or(Value::Null),
                    state: JobState::of_row(&state, priority),
                    priority,
                    attempts_made: r.get(4)?,
                    attempts_started: r.get(5)?,
                    opts: opts_from_json(&serde_json::from_str(&opts).unwrap_or(Value::Null)),
                    return_value: rv.and_then(|s| serde_json::from_str(&s).ok()),
                    failed_reason: r.get(8)?,
                })
            },
        )
        .optional()?)
}

impl Enqueuer for JobStore {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let name = name.to_owned();
        Box::pin(async move {
            let mut results = self
                .add_many(queue, vec![NewJob { name, data, opts }])
                .await?;
            results
                .pop()
                .ok_or_else(|| JobsError::Store("add returned no result".to_owned()))
        })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(self.add_many(queue, jobs))
    }
}

impl JobInspector for JobStore {
    fn get_job<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<JobRecord>, JobsError>> {
        Box::pin(self.read_job(queue, job_id))
    }

    fn get_state<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<JobState, JobsError>> {
        Box::pin(async move {
            Ok(self
                .read_job(queue, job_id)
                .await?
                .map_or(JobState::Unknown, |j| j.state))
        })
    }

    fn change_priority<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        priority: u32,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            if priority > MAX_PRIORITY {
                return Err(JobsError::InvalidOptions(format!(
                    "Priority should be between 0 and {MAX_PRIORITY}"
                )));
            }
            let job_id = job_id.to_owned();
            let changed = self
                .write_retry(move |tx| {
                    match job_state(tx, queue, &job_id)?.as_deref() {
                        // A new FIFO slot inside the priority (BullMQ re-adds).
                        Some("waiting") => {
                            let seq = bump(tx, "ready_seq", 1)?;
                            tx.execute(
                                "UPDATE jobs SET priority = ?3, ready_seq = ?4 WHERE queue = ?1 AND job_id = ?2",
                                params![queue.as_str(), job_id, priority, seq],
                            )?;
                            Ok(true)
                        }
                        Some("delayed") => {
                            tx.execute(
                                "UPDATE jobs SET priority = ?3 WHERE queue = ?1 AND job_id = ?2",
                                params![queue.as_str(), job_id, priority],
                            )?;
                            Ok(true)
                        }
                        _ => Ok(false),
                    }
                })
                .await?;
            if changed {
                self.signals.queue(queue).notify_one();
            }
            Ok(changed)
        })
    }

    fn remove<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
    ) -> BoxFuture<'a, Result<bool, JobsError>> {
        Box::pin(async move {
            let job_id = job_id.to_owned();
            Ok(self
                .write_retry(move |tx| {
                    let row: Option<(String, Option<String>)> = tx
                        .query_row(
                            "SELECT state, dedup_id FROM jobs WHERE queue = ?1 AND job_id = ?2",
                            params![queue.as_str(), job_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    let Some((state, dedup_id)) = row else {
                        return Ok(false);
                    };
                    // BullMQ refuses to remove a locked (active) job.
                    if state == "active" {
                        return Ok(false);
                    }
                    tx.execute(
                        "DELETE FROM jobs WHERE queue = ?1 AND job_id = ?2",
                        params![queue.as_str(), job_id],
                    )?;
                    if let Some(dedup_id) = dedup_id {
                        // removeDeduplicationKeyIfNeededOnRemoval
                        let removed = tx.execute(
                            "DELETE FROM job_dedup WHERE queue = ?1 AND dedup_id = ?2 AND job_id = ?3",
                            params![queue.as_str(), dedup_id, job_id],
                        )?;
                        if removed > 0 {
                            tx.execute(
                                "DELETE FROM job_dedup_next WHERE queue = ?1 AND dedup_id = ?2",
                                params![queue.as_str(), dedup_id],
                            )?;
                        }
                    }
                    Ok(true)
                })
                .await?)
        })
    }
}

impl JobWaiter for JobStore {
    fn wait_until_finished<'a>(
        &'a self,
        queue: QueueName,
        job_id: &'a str,
        timeout_ms: u64,
    ) -> BoxFuture<'a, Result<Value, WaitError>> {
        // Subscribe first, then read the row (spec 2.9): no finish is lost.
        let mut rx = self.registry.subscribe();
        Box::pin(async move {
            let settled = |record: &JobRecord| match record.state {
                JobState::Completed => Some(Ok(record.return_value.clone().unwrap_or(Value::Null))),
                JobState::Failed => Some(Err(WaitError::Failed(
                    record.failed_reason.clone().unwrap_or_default(),
                ))),
                _ => None,
            };
            match self.read_job(queue, job_id).await {
                Ok(Some(record)) => {
                    if let Some(outcome) = settled(&record) {
                        return outcome;
                    }
                }
                Ok(None) => {
                    // Gone: removed right after it finished, or never existed.
                    while let Ok(event) = rx.try_recv() {
                        if let Some(outcome) = crate::wait::finish_outcome(&event, queue, job_id) {
                            return outcome;
                        }
                    }
                    return Err(WaitError::CompletionStateUnknown(format!(
                        "Missing key for job {job_id}"
                    )));
                }
                Err(e) => return Err(WaitError::CompletionStateUnknown(e.to_string())),
            }
            self.registry
                .wait_on(rx, queue, job_id, timeout_ms, || {
                    Box::pin(async move {
                        match self.read_job(queue, job_id).await {
                            Ok(Some(record)) => settled(&record),
                            Ok(None) => None,
                            Err(e) => Some(Err(WaitError::CompletionStateUnknown(e.to_string()))),
                        }
                    })
                })
                .await
        })
    }
}

// ---------------------------------------------------------------------------
// Job schedulers (S13)
// ---------------------------------------------------------------------------

impl JobStore {
    /// `upsertJobScheduler` on `queue`: a changed or new repeat recomputes
    /// the next fire time from now; the template is replaced.
    pub async fn upsert_scheduler(
        &self,
        queue: QueueName,
        id: &str,
        repeat: &Repeat,
        template: &ScheduleTemplate,
    ) -> Result<(), JobsError> {
        if let Repeat::Pattern(p) = repeat {
            CronPattern::parse(p)?;
        }
        let now = self.clock.now_ms();
        let id = id.to_owned();
        let (pattern, every) = match repeat {
            Repeat::Pattern(p) => (Some(p.clone()), None),
            Repeat::EveryMs(ms) => (None, Some(ms_i64(*ms).max(1))),
        };
        let name = template.name.clone();
        let data = template.data.to_string();
        self.write_retry(move |tx| {
                let existing: Option<(Option<String>, Option<i64>)> = tx
                    .query_row(
                        "SELECT pattern, every_ms FROM job_schedulers WHERE queue = ?1 AND scheduler_id = ?2",
                        params![queue.as_str(), id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let unchanged = existing.as_ref() == Some(&(pattern.clone(), every));
                if unchanged {
                    tx.execute(
                        "UPDATE job_schedulers SET template_name = ?3, template_data = ?4 \
                         WHERE queue = ?1 AND scheduler_id = ?2",
                        params![queue.as_str(), id, name, data],
                    )?;
                } else {
                    let next = scheduler_next(pattern.as_deref(), every, None, now);
                    tx.execute(
                        "INSERT INTO job_schedulers (queue, scheduler_id, pattern, every_ms, template_name, \
                         template_data, next_run_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                         ON CONFLICT (queue, scheduler_id) DO UPDATE SET pattern = excluded.pattern, \
                         every_ms = excluded.every_ms, template_name = excluded.template_name, \
                         template_data = excluded.template_data, next_run_at_ms = excluded.next_run_at_ms",
                        params![queue.as_str(), id, pattern, every, name, data, next],
                    )?;
                    // The pending job of the old repeat is replaced.
                    tx.execute(
                        "DELETE FROM jobs WHERE queue = ?1 AND scheduler_id = ?2 AND state IN ('waiting', 'delayed')",
                        params![queue.as_str(), id],
                    )?;
                }
                Ok(())
            })
            .await?;
        self.signals.poke_timer();
        Ok(())
    }

    /// `removeJobScheduler`: the scheduler and its pending job.
    pub async fn remove_scheduler(&self, queue: QueueName, id: &str) -> Result<bool, JobsError> {
        let id = id.to_owned();
        Ok(self
            .write_retry(move |tx| {
                let removed = tx.execute(
                    "DELETE FROM job_schedulers WHERE queue = ?1 AND scheduler_id = ?2",
                    params![queue.as_str(), id],
                )? > 0;
                tx.execute(
                    "DELETE FROM jobs WHERE queue = ?1 AND scheduler_id = ?2 AND state IN ('waiting', 'delayed')",
                    params![queue.as_str(), id],
                )?;
                Ok(removed)
            })
            .await?)
    }

    /// The scheduler ids of `queue` with their next fire time.
    pub async fn schedulers(&self, queue: QueueName) -> Result<Vec<(String, i64)>, JobsError> {
        Ok(self
            .read_retry(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT scheduler_id, next_run_at_ms FROM job_schedulers WHERE queue = ?1 ORDER BY scheduler_id",
                )?;
                let rows = stmt
                    .query_map(params![queue.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    #[derive(Default)]
    struct ManualClock(AtomicI64);

    impl Clock for ManualClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl ManualClock {
        fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    const Q: QueueName = QueueName::Chat;
    const OWNER: &str = "owner-a";

    async fn store() -> (Arc<JobStore>, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock(AtomicI64::new(1_000_000)));
        let db = crate::temp_db();
        let store = JobStore::open(db, clock.clone()).await.expect("store");
        (Arc::new(store), clock)
    }

    fn id(job_id: &str) -> JobOptions {
        JobOptions {
            job_id: Some(job_id.to_owned()),
            ..JobOptions::default()
        }
    }

    async fn add(store: &JobStore, opts: JobOptions) -> AddResult {
        store
            .add(Q, "job", json!({ "n": 1 }), opts)
            .await
            .expect("add")
    }

    async fn state(store: &JobStore, job_id: &str) -> JobState {
        store.get_state(Q, job_id).await.expect("state")
    }

    async fn claim_ids(store: &JobStore, limit: u32) -> Vec<String> {
        store
            .claim(Q, OWNER, limit)
            .await
            .expect("claim")
            .into_iter()
            .map(|j| j.job_id)
            .collect()
    }

    async fn finish(store: &JobStore, job_id: &str, outcome: Outcome) -> Option<bool> {
        store
            .finish(
                Q,
                job_id,
                Some(OWNER),
                outcome,
                Retention::Keep,
                Retention::Keep,
            )
            .await
            .expect("finish")
    }

    fn failed(reason: &str) -> Outcome {
        Outcome::Failed {
            reason: reason.to_owned(),
            unrecoverable: false,
        }
    }

    #[tokio::test]
    async fn unprioritized_jobs_run_first_then_priority_then_fifo() {
        let (store, _) = store().await;
        add(
            &store,
            JobOptions {
                priority: Some(5),
                ..id("p5")
            },
        )
        .await;
        add(
            &store,
            JobOptions {
                priority: Some(1),
                ..id("p1")
            },
        )
        .await;
        add(&store, id("plain-a")).await;
        add(
            &store,
            JobOptions {
                priority: Some(1),
                ..id("p1-b")
            },
        )
        .await;
        add(&store, id("plain-b")).await;

        assert_eq!(state(&store, "p1").await, JobState::Prioritized);
        assert_eq!(state(&store, "plain-a").await, JobState::Waiting);
        assert_eq!(
            claim_ids(&store, 10).await,
            ["plain-a", "plain-b", "p1", "p1-b", "p5"]
        );
        assert_eq!(state(&store, "p5").await, JobState::Active);
        assert_eq!(state(&store, "missing").await, JobState::Unknown);
    }

    #[tokio::test]
    async fn a_known_job_id_deduplicates_in_every_state_and_colons_are_rejected() {
        let (store, _) = store().await;
        assert!(add(&store, id("same")).await.created);
        assert!(!add(&store, id("same")).await.created);
        claim_ids(&store, 1).await;
        finish(&store, "same", Outcome::Completed(json!("ok"))).await;
        let again = add(&store, id("same")).await;
        assert_eq!((again.job_id.as_str(), again.created), ("same", false));

        let err = store
            .add(Q, "job", json!({}), id("a:b"))
            .await
            .expect_err("colon");
        assert_eq!(
            err,
            JobsError::InvalidOptions("Custom Id cannot contain :".to_owned())
        );
        let auto = add(&store, JobOptions::default()).await;
        assert!(auto.created && auto.job_id.parse::<i64>().is_ok());
    }

    #[tokio::test]
    async fn delayed_jobs_wait_until_promoted() {
        let (store, clock) = store().await;
        add(
            &store,
            JobOptions {
                delay_ms: Some(500),
                ..id("later")
            },
        )
        .await;
        assert_eq!(state(&store, "later").await, JobState::Delayed);
        assert_eq!(store.promote_due().await.expect("promote"), 0);
        assert!(claim_ids(&store, 1).await.is_empty());

        clock.advance(500);
        assert_eq!(store.promote_due().await.expect("promote"), 1);
        assert_eq!(claim_ids(&store, 1).await, ["later"]);
    }

    #[tokio::test]
    async fn a_failure_retries_with_exponential_backoff_then_fails_finally() {
        let (store, clock) = store().await;
        let mut events = store.registry().subscribe();
        add(
            &store,
            JobOptions {
                attempts: Some(3),
                backoff: Some(Backoff::Exponential(1000)),
                ..id("flaky")
            },
        )
        .await;

        claim_ids(&store, 1).await;
        assert_eq!(finish(&store, "flaky", failed("boom 1")).await, Some(true));
        assert_eq!(state(&store, "flaky").await, JobState::Delayed);
        clock.advance(999);
        assert_eq!(store.promote_due().await.unwrap(), 0);
        clock.advance(1);
        assert_eq!(store.promote_due().await.unwrap(), 1);

        claim_ids(&store, 1).await;
        assert_eq!(finish(&store, "flaky", failed("boom 2")).await, Some(true));
        clock.advance(1999);
        assert_eq!(
            store.promote_due().await.unwrap(),
            0,
            "2nd retry waits 2000 ms"
        );
        clock.advance(1);
        assert_eq!(store.promote_due().await.unwrap(), 1);

        let job = store.claim(Q, OWNER, 1).await.unwrap().remove(0);
        assert_eq!((job.attempts_made, job.attempts_started), (2, 3));
        assert_eq!(finish(&store, "flaky", failed("boom 3")).await, Some(false));
        let record = store.get_job(Q, "flaky").await.unwrap().expect("kept");
        assert_eq!(record.state, JobState::Failed);
        assert_eq!(record.failed_reason.as_deref(), Some("boom 3"));
        assert_eq!(record.attempts_made, 3);

        let mut finals = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let JobEvent::Failed { final_attempt, .. } = event {
                finals.push(final_attempt);
            }
        }
        assert_eq!(finals, [false, false, true]);
    }

    #[tokio::test]
    async fn unrecoverable_and_delayed_outcomes() {
        let (store, clock) = store().await;
        add(
            &store,
            JobOptions {
                attempts: Some(5),
                ..id("fatal")
            },
        )
        .await;
        claim_ids(&store, 1).await;
        let outcome = Outcome::Failed {
            reason: "bad input".to_owned(),
            unrecoverable: true,
        };
        assert_eq!(finish(&store, "fatal", outcome).await, Some(false));
        assert_eq!(state(&store, "fatal").await, JobState::Failed);

        add(&store, id("parked")).await;
        claim_ids(&store, 1).await;
        let run_at_ms = clock.now_ms() + 100;
        finish(&store, "parked", Outcome::Delayed { run_at_ms }).await;
        assert_eq!(state(&store, "parked").await, JobState::Delayed);
        clock.advance(100);
        store.promote_due().await.unwrap();
        let job = store.claim(Q, OWNER, 1).await.unwrap().remove(0);
        // A deferral is not a failure: attempts_made stays, attempts_started grows.
        assert_eq!((job.attempts_made, job.attempts_started), (0, 2));
    }

    #[tokio::test]
    async fn a_finish_by_another_lease_owner_is_ignored() {
        let (store, _) = store().await;
        add(&store, id("mine")).await;
        claim_ids(&store, 1).await;
        let other = store
            .finish(
                Q,
                "mine",
                Some("owner-b"),
                Outcome::Completed(json!(1)),
                Retention::Keep,
                Retention::Keep,
            )
            .await
            .unwrap();
        assert_eq!(other, None);
        assert_eq!(state(&store, "mine").await, JobState::Active);
    }

    #[tokio::test]
    async fn retention_is_per_job() {
        let (store, clock) = store().await;
        add(
            &store,
            JobOptions {
                remove_on_complete: Some(Retention::Remove),
                ..id("gone")
            },
        )
        .await;
        add(
            &store,
            JobOptions {
                remove_on_complete: Some(Retention::Keep),
                ..id("kept")
            },
        )
        .await;
        for n in 0..3 {
            add(
                &store,
                JobOptions {
                    remove_on_complete: Some(Retention::KeepCount(1)),
                    ..id(&format!("count-{n}"))
                },
            )
            .await;
        }
        add(
            &store,
            JobOptions {
                remove_on_complete: Some(Retention::KeepAge {
                    age_secs: 10,
                    count: None,
                }),
                ..id("aged")
            },
        )
        .await;
        for job_id in claim_ids(&store, 10).await {
            clock.advance(1);
            finish(&store, &job_id, Outcome::Completed(json!(null))).await;
        }
        store.sweep_retention().await.unwrap();

        assert_eq!(state(&store, "gone").await, JobState::Unknown);
        assert!(
            add(&store, id("gone")).await.created,
            "the id is free again"
        );
        assert_eq!(state(&store, "kept").await, JobState::Completed);
        // Only rows with a count rule are trimmed by count (newest kept).
        assert_eq!(state(&store, "count-0").await, JobState::Unknown);
        assert_eq!(state(&store, "count-1").await, JobState::Unknown);
        assert_eq!(state(&store, "aged").await, JobState::Completed);
        clock.advance(10_000);
        store.sweep_retention().await.unwrap();
        assert_eq!(state(&store, "aged").await, JobState::Unknown);
        assert_eq!(state(&store, "kept").await, JobState::Completed);
    }

    fn dedup(id: &str) -> Dedup {
        Dedup {
            id: id.to_owned(),
            ..Dedup::default()
        }
    }

    #[tokio::test]
    async fn simple_dedup_holds_until_the_job_finishes() {
        let (store, _) = store().await;
        let opts = |job: &str| JobOptions {
            dedup: Some(dedup("tap")),
            ..id(job)
        };
        assert!(add(&store, opts("a")).await.created);
        let second = add(&store, opts("b")).await;
        assert_eq!((second.job_id.as_str(), second.created), ("a", false));
        claim_ids(&store, 1).await;
        finish(&store, "a", Outcome::Completed(json!(1))).await;
        assert!(add(&store, opts("c")).await.created, "key freed on finish");
    }

    #[tokio::test]
    async fn throttle_dedup_outlives_the_job_until_the_ttl() {
        let (store, clock) = store().await;
        let opts = |job: &str| JobOptions {
            dedup: Some(Dedup {
                ttl_ms: Some(1000),
                ..dedup("restore")
            }),
            ..id(job)
        };
        assert!(add(&store, opts("a")).await.created);
        claim_ids(&store, 1).await;
        finish(&store, "a", Outcome::Completed(json!(1))).await;
        assert!(!add(&store, opts("b")).await.created, "ttl key stays");
        clock.advance(1000);
        assert!(
            add(&store, opts("c")).await.created,
            "expired key is absent"
        );
    }

    #[tokio::test]
    async fn debounce_replaces_the_delayed_job_with_the_latest_data() {
        let (store, _) = store().await;
        let add_debounced = |n: i64| {
            let store = Arc::clone(&store);
            async move {
                store
                    .add(
                        Q,
                        "reply",
                        json!({ "n": n }),
                        JobOptions {
                            delay_ms: Some(5000),
                            dedup: Some(Dedup {
                                ttl_ms: Some(5000),
                                extend: true,
                                replace: true,
                                ..dedup("conv-1")
                            }),
                            ..JobOptions::default()
                        },
                    )
                    .await
                    .unwrap()
            }
        };
        let first = add_debounced(1).await;
        let second = add_debounced(2).await;
        assert!(first.created && second.created);
        assert_eq!(state(&store, &first.job_id).await, JobState::Unknown);
        let record = store.get_job(Q, &second.job_id).await.unwrap().unwrap();
        assert_eq!(
            (record.state, record.data),
            (JobState::Delayed, json!({ "n": 2 }))
        );
    }

    #[tokio::test]
    async fn keep_last_if_active_requeues_the_latest_add_after_the_active_job() {
        let (store, _) = store().await;
        let add_kept = |n: i64| {
            let store = Arc::clone(&store);
            async move {
                store
                    .add(
                        Q,
                        "welcome",
                        json!({ "n": n }),
                        JobOptions {
                            dedup: Some(Dedup {
                                ttl_ms: Some(5000),
                                keep_last_if_active: true,
                                ..dedup("flow-1")
                            }),
                            ..id(&format!("w{n}"))
                        },
                    )
                    .await
                    .unwrap()
            }
        };
        assert!(add_kept(1).await.created);
        claim_ids(&store, 1).await;
        assert!(!add_kept(2).await.created);
        assert!(!add_kept(3).await.created);
        assert_eq!(state(&store, "w3").await, JobState::Unknown);

        finish(&store, "w1", Outcome::Completed(json!(null))).await;
        let next = store
            .get_job(Q, "w3")
            .await
            .unwrap()
            .expect("latest requeued");
        assert_eq!(
            (next.state, next.data),
            (JobState::Waiting, json!({ "n": 3 }))
        );
        assert_eq!(state(&store, "w2").await, JobState::Unknown);
    }

    #[tokio::test]
    async fn change_priority_and_remove() {
        let (store, _) = store().await;
        add(
            &store,
            JobOptions {
                priority: Some(10),
                ..id("bg")
            },
        )
        .await;
        assert!(store.change_priority(Q, "bg", 1).await.unwrap());
        assert_eq!(store.get_job(Q, "bg").await.unwrap().unwrap().priority, 1);
        claim_ids(&store, 1).await;
        assert!(!store.change_priority(Q, "bg", 5).await.unwrap());
        assert!(
            !store.remove(Q, "bg").await.unwrap(),
            "active job is locked"
        );
        add(&store, id("queued")).await;
        assert!(store.remove(Q, "queued").await.unwrap());
        assert!(!store.remove(Q, "queued").await.unwrap());
    }

    #[tokio::test]
    async fn job_counts_split_waiting_and_prioritized() {
        let (store, _) = store().await;
        add(&store, id("a")).await;
        add(
            &store,
            JobOptions {
                priority: Some(3),
                ..id("b")
            },
        )
        .await;
        add(
            &store,
            JobOptions {
                delay_ms: Some(10),
                ..id("c")
            },
        )
        .await;
        let counts = store
            .get_job_counts(
                Q,
                &[
                    JobState::Waiting,
                    JobState::Prioritized,
                    JobState::Delayed,
                    JobState::Active,
                ],
            )
            .await
            .unwrap();
        let expected: BTreeMap<JobState, u64> = [
            (JobState::Waiting, 1),
            (JobState::Prioritized, 1),
            (JobState::Delayed, 1),
            (JobState::Active, 0),
        ]
        .into_iter()
        .collect();
        assert_eq!(counts, expected);
        assert_eq!(store.prioritized_count(Q).await.unwrap(), 1);
        assert_eq!(store.delayed_count(Q).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn quota_jobs_are_skipped_on_the_community_edition() {
        let clock = Arc::new(ManualClock::default());
        let db = Arc::new(Db::open_in_memory().unwrap());
        let community = JobStore::open(Arc::clone(&db), clock.clone())
            .await
            .unwrap();
        let skipped = community
            .add(QueueName::Quota, "sync", json!({}), id("q1"))
            .await
            .unwrap();
        assert!(!skipped.created);
        let cloud = JobStore::open(db, clock)
            .await
            .unwrap()
            .with_edition(Edition::Cloud);
        assert!(
            cloud
                .add(QueueName::Quota, "sync", json!({}), id("q1"))
                .await
                .unwrap()
                .created
        );
    }

    #[tokio::test]
    async fn stalled_jobs_go_back_to_the_head_then_fail_after_the_limit() {
        let (store, _) = store().await;
        add(&store, id("first")).await;
        add(&store, id("stuck")).await;
        assert_eq!(claim_ids(&store, 1).await, ["first"]);
        assert_eq!(claim_ids(&store, 1).await, ["stuck"]);
        finish(&store, "first", Outcome::Completed(json!(1))).await;
        add(&store, id("newer")).await;

        // Another process starts: our lease is foreign to it.
        let recovered = store
            .recover_stalled("owner-b", vec![(Q, 1)], Vec::new())
            .await
            .unwrap();
        assert_eq!(recovered, 1);
        assert_eq!(state(&store, "stuck").await, JobState::Waiting);
        let next = store.claim(Q, "owner-b", 1).await.unwrap();
        assert_eq!(next[0].job_id, "stuck", "a stalled job runs next");

        store
            .recover_stalled("owner-c", vec![(Q, 1)], Vec::new())
            .await
            .unwrap();
        let record = store.get_job(Q, "stuck").await.unwrap().unwrap();
        assert_eq!(record.state, JobState::Failed);
        assert_eq!(record.failed_reason.as_deref(), Some(STALLED_TOO_OFTEN));
    }

    #[tokio::test]
    async fn schedulers_fire_once_per_slot_without_backfill() {
        let (store, clock) = store().await;
        clock.0.store(60_000 * 100 + 30_000, Ordering::SeqCst);
        let template = ScheduleTemplate {
            name: "tick".to_owned(),
            data: json!({ "type": "tick", "data": {} }),
        };
        let every_minute = Repeat::Pattern("* * * * *".to_owned());
        store
            .upsert_scheduler(QueueName::Schedule, "tick", &every_minute, &template)
            .await
            .unwrap();
        let next = store.next_wake(true).await.unwrap();
        assert_eq!(next, Some(60_000 * 101));

        // Downtime of 5 minutes: one catch-up job, next slot after now.
        clock.0.store(60_000 * 106 + 1, Ordering::SeqCst);
        store.fire_due(true).await.unwrap();
        store.fire_due(true).await.unwrap();
        let counts = store
            .get_job_counts(QueueName::Schedule, &[JobState::Waiting])
            .await
            .unwrap();
        assert_eq!(counts[&JobState::Waiting], 1);
        let job_id = format!("repeat:tick:{}", 60_000 * 101);
        let job = store.get_job(QueueName::Schedule, &job_id).await.unwrap();
        assert_eq!(job.map(|j| j.data), Some(template.data.clone()));
        assert_eq!(store.next_wake(true).await.unwrap(), Some(60_000 * 107));

        // While its job still waits, a due scheduler adds no second job.
        clock.0.store(60_000 * 107, Ordering::SeqCst);
        store.fire_due(true).await.unwrap();
        let counts = store
            .get_job_counts(QueueName::Schedule, &[JobState::Waiting])
            .await
            .unwrap();
        assert_eq!(counts[&JobState::Waiting], 1);

        assert!(
            store
                .remove_scheduler(QueueName::Schedule, "tick")
                .await
                .unwrap()
        );
        assert_eq!(store.next_wake(true).await.unwrap(), None);
        let counts = store
            .get_job_counts(QueueName::Schedule, &[JobState::Waiting])
            .await
            .unwrap();
        assert_eq!(counts[&JobState::Waiting], 0, "pending job removed");
    }

    #[tokio::test]
    async fn every_schedulers_realign_after_missed_slots() {
        assert_eq!(scheduler_next(None, Some(1000), None, 5500), 5500);
        assert_eq!(scheduler_next(None, Some(1000), Some(5500), 5600), 6500);
        assert_eq!(scheduler_next(None, Some(1000), Some(5500), 9100), 10_000);
    }

    #[tokio::test]
    async fn wait_until_finished_reads_a_finished_row_and_waits_for_events() {
        let (store, _) = store().await;
        add(&store, id("done")).await;
        claim_ids(&store, 1).await;
        finish(&store, "done", Outcome::Completed(json!({ "ok": true }))).await;
        assert_eq!(
            store.wait_until_finished(Q, "done", 1000).await,
            Ok(json!({ "ok": true }))
        );
        assert!(matches!(
            store.wait_until_finished(Q, "never", 1000).await,
            Err(WaitError::CompletionStateUnknown(_))
        ));

        add(&store, id("later")).await;
        let waiter = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.wait_until_finished(Q, "later", 5000).await })
        };
        tokio::task::yield_now().await;
        claim_ids(&store, 1).await;
        finish(&store, "later", failed("nope")).await;
        assert_eq!(
            waiter.await.unwrap(),
            Err(WaitError::Failed("nope".to_owned()))
        );
    }

    #[test]
    fn options_round_trip_through_json() {
        let opts = JobOptions {
            job_id: Some("x".to_owned()),
            delay_ms: Some(5),
            priority: Some(7),
            attempts: Some(3),
            backoff: Some(Backoff::Fixed(10)),
            remove_on_complete: Some(Retention::KeepAge {
                age_secs: 60,
                count: Some(5),
            }),
            remove_on_fail: Some(Retention::Remove),
            dedup: Some(Dedup {
                id: "d".to_owned(),
                ttl_ms: Some(9),
                extend: true,
                replace: false,
                keep_last_if_active: true,
            }),
        };
        assert_eq!(opts_from_json(&opts_to_json(&opts)), opts);
        assert_eq!(
            opts_from_json(&json!({ "removeOnComplete": 100, "removeOnFail": false })),
            JobOptions {
                remove_on_complete: Some(Retention::KeepCount(100)),
                remove_on_fail: Some(Retention::Keep),
                ..JobOptions::default()
            }
        );
    }
}
