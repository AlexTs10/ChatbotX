//! Idle cost of a running jobs layer (P2.6 task: idle wake-ups/s and RSS).
//!
//! Starts every worker of `worker_specs()` (17 queues with a consumer; the
//! `broadcast` and `quota` queues have none), registers the community
//! schedulers, starts the sequence producer on the SQLite schedule and the
//! four event-bus consumers, then stays idle for `IDLE_SECS` (default 30)
//! and prints, from `/proc` (Linux):
//! - context switches of all threads per second (a proxy for wake-ups),
//! - jobs run (the cron jobs that fired meanwhile),
//! - `VmHWM` / `VmRSS`.
//!
//! Run: `cargo run --release -p chatbotx-jobs --example idle_scheduler`.
//! `MODE=no-crons` skips the schedulers (background wake-ups only);
//! `MODE=baseline` opens the database and the store and starts nothing
//! (the RSS without the running jobs layer).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_jobs::events::config::{BusKind, bus_config};
use chatbotx_jobs::events::{EventBus, EventModule, EventWorker, SqliteEventStore};
use chatbotx_jobs::runtime::{Handler, Runtime, worker_specs};
use chatbotx_jobs::schedules::{Edition, ScheduleConfig, register_schedules, schedules};
use chatbotx_jobs::sequence::producer::{
    PendingDispatchLookup, PendingDispatchRef, ProducerBackend, SchedulerWorker,
    SchedulerWorkerDeps, SchedulerWorkerOptions,
};
use chatbotx_jobs::sequence::schedule::DispatchSchedule;
use chatbotx_jobs::sequence::sqlite_schedule::SqliteDispatchSchedule;
use chatbotx_jobs::shutdown::Shutdown;
use chatbotx_jobs::{BoxFuture, Clock, Enqueuer, JobStore, JobsError, SystemClock};

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

struct NoPending;

impl PendingDispatchLookup for NoPending {
    fn list_pending_workspace_ids<'a>(
        &'a self,
        _dispatch_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<PendingDispatchRef>, JobsError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// `(voluntary + nonvoluntary)` context switches per live thread id.
fn context_switches() -> Option<BTreeMap<String, u64>> {
    let mut out = BTreeMap::new();
    for task in std::fs::read_dir("/proc/self/task").ok()? {
        let task = task.ok()?;
        let Ok(status) = std::fs::read_to_string(task.path().join("status")) else {
            continue; // the thread just exited
        };
        let mut total = 0;
        for line in status.lines() {
            if let Some(rest) = line
                .strip_prefix("voluntary_ctxt_switches:")
                .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
            {
                total += rest.trim().parse::<u64>().ok()?;
            }
        }
        out.insert(task.file_name().to_string_lossy().into_owned(), total);
    }
    Some(out)
}

/// Switches between two snapshots: per thread id, a new thread from 0.
/// Threads that exited in between (idle blocking-pool threads) are not
/// counted after their exit.
fn switches_between(
    before: &Option<BTreeMap<String, u64>>,
    after: &Option<BTreeMap<String, u64>>,
) -> Option<u64> {
    let (before, after) = (before.as_ref()?, after.as_ref()?);
    Some(
        after
            .iter()
            .map(|(tid, n)| n.saturating_sub(before.get(tid).copied().unwrap_or(0)))
            .sum(),
    )
}

fn status_field(name: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(name))
                .map(|l| l[name.len()..].trim().to_owned())
        })
        .unwrap_or_else(|| "n/a".to_owned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let idle_secs: u64 = std::env::var("IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let dir = std::env::temp_dir().join(format!("chatbotx-jobs-idle-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let db = Arc::new(Db::open(dir.join("app.db"))?);
        let store = Arc::new(JobStore::open(Arc::clone(&db), Arc::clone(&clock)).await?);
        let shutdown = Shutdown::new(
            None,
            Arc::new(|_| {}),
            Arc::new(|| Box::pin(async { Ok(()) })),
        );

        let config = ScheduleConfig {
            edition: Edition::Community,
            quota_sync_interval_seconds: 60,
            purge_workspaces_interval_minutes: 30,
            purge_broadcasts_interval_minutes: 5,
            license_refresh_minute: 7,
        };
        let mode = std::env::var("MODE").unwrap_or_default();
        if mode == "baseline" {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let before = context_switches();
            tokio::time::sleep(Duration::from_secs(idle_secs)).await;
            let after = context_switches();
            println!("mode: baseline (database and store only)");
            if let Some(n) = switches_between(&before, &after) {
                println!("context switches: {:.2}/s", n as f64 / idle_secs as f64);
            }
            println!("VmHWM: {}", status_field("VmHWM:"));
            println!("VmRSS: {}", status_field("VmRSS:"));
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(());
        }
        let scheduler_count = if mode == "no-crons" {
            0
        } else {
            register_schedules(&config, store.as_ref()).await?;
            schedules(&config).len()
        };

        let jobs_run = Arc::new(AtomicU64::new(0));
        let jobs = Runtime::new(
            Arc::clone(&store),
            Arc::clone(&shutdown),
            Arc::clone(&clock),
        );
        let specs = worker_specs();
        let queue_count = specs.len();
        for spec in specs {
            let counter = Arc::clone(&jobs_run);
            let handler: Handler = Arc::new(move |_job| {
                counter.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Ok(serde_json::Value::Null) })
            });
            jobs.spawn_worker(spec, handler)?;
        }

        let schedule = Arc::new(SqliteDispatchSchedule::open(Arc::clone(&db), None).await?);
        let producer = SchedulerWorker::new(
            SchedulerWorkerOptions::default(),
            &BTreeMap::new(),
            SchedulerWorkerDeps {
                backend: Arc::new(Backend {
                    schedule,
                    queue: Arc::clone(&store),
                }),
                pending: Arc::new(NoPending),
                clock: Arc::clone(&clock),
                shutdown_requested: Arc::new(|| false),
            },
        );
        producer.start().await?;

        let events = Arc::new(SqliteEventStore::open(Arc::clone(&db)).await?);
        let modules = [
            BusKind::Message,
            BusKind::Flow,
            BusKind::Dashboard,
            BusKind::ErrorLog,
        ]
        .into_iter()
        .map(|kind| EventModule {
            bus: Arc::new(EventBus::new(
                events.clone(),
                bus_config(kind),
                Arc::clone(&clock),
            )),
            listeners: BTreeMap::new(),
        })
        .collect();
        let event_worker = EventWorker::new();
        event_worker.start(modules, Some("idle-example")).await?;

        // Let the startup work settle, then measure.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let switches_before = context_switches();
        let jobs_before = jobs_run.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(idle_secs)).await;
        let switches_after = context_switches();
        let jobs_after = jobs_run.load(Ordering::Relaxed);

        println!(
            "queues with a worker: {queue_count} (of 19; broadcast and quota have no consumer)"
        );
        println!("schedulers registered: {scheduler_count} (community; mode {mode:?})");
        println!("idle window: {idle_secs} s");
        match switches_between(&switches_before, &switches_after) {
            Some(n) => println!(
                "context switches: {n} total, {:.2}/s (all threads)",
                n as f64 / idle_secs as f64
            ),
            None => println!("context switches: n/a (no /proc)"),
        }
        println!(
            "jobs run in the window: {} ({:.2}/s, cron jobs)",
            jobs_after - jobs_before,
            (jobs_after - jobs_before) as f64 / idle_secs as f64
        );
        println!("VmHWM: {}", status_field("VmHWM:"));
        println!("VmRSS: {}", status_field("VmRSS:"));

        producer.stop().await;
        event_worker.stop().await;
        shutdown
            .request(chatbotx_jobs::shutdown::ShutdownReason::Sigterm, None)
            .await;
        drop(jobs);
        let _ = std::fs::remove_dir_all(&dir);
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
