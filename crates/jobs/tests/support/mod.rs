//! Shared test support for the ported P2.6 frozen tests (spec 5,
//! "Porting conventions"). Test code, not crate API: the fakes here are
//! complete; the crate code under test is not.
//!
//! Area-specific support lives next to this file and is included by the
//! test files that need it with `#[path = "support/<area>.rs"] mod <area>;`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_db::Db;
use chatbotx_jobs::{
    AddResult, BoxFuture, Clock, Enqueuer, JobOptions, JobsError, NewJob, QueueName,
};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

// ---------------------------------------------------------------------------
// TestClock: manual "now" (replaces `vi.useFakeTimers` / `Date.now` mocks)
// ---------------------------------------------------------------------------

/// A clock that only moves when the test moves it.
#[derive(Debug, Default)]
pub struct TestClock {
    now: AtomicI64,
}

impl TestClock {
    pub fn new(now_ms: i64) -> Arc<TestClock> {
        Arc::new(TestClock {
            now: AtomicI64::new(now_ms),
        })
    }

    pub fn set(&self, now_ms: i64) {
        self.now.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// FakeEnqueuer: records every add (replaces the vitest queue mocks)
// ---------------------------------------------------------------------------

/// One recorded `add` (or one job of an `add_bulk`).
#[derive(Debug, Clone, PartialEq)]
pub struct EnqueueCall {
    pub queue: QueueName,
    pub name: String,
    pub data: Value,
    pub opts: JobOptions,
}

/// Records `(queue, name, data, opts)` of every `add`, and every `add_bulk`
/// as one batch. Returns `AddResult { job_id: opts.job_id or a counter,
/// created: true }`, or the configured error.
#[derive(Debug, Default)]
pub struct FakeEnqueuer {
    calls: Mutex<Vec<EnqueueCall>>,
    bulks: Mutex<Vec<(QueueName, Vec<NewJob>)>>,
    fail_with: Mutex<Option<JobsError>>,
    next_id: AtomicU64,
}

impl FakeEnqueuer {
    pub fn new() -> Arc<FakeEnqueuer> {
        Arc::new(FakeEnqueuer::default())
    }

    /// Every `add` and `add_bulk` fails with `error`.
    pub fn failing(error: JobsError) -> Arc<FakeEnqueuer> {
        let fake = FakeEnqueuer::default();
        *fake.fail_with.lock().unwrap() = Some(error);
        Arc::new(fake)
    }

    pub fn fail_with(&self, error: Option<JobsError>) {
        *self.fail_with.lock().unwrap() = error;
    }

    /// Single `add` calls, in order.
    pub fn calls(&self) -> Vec<EnqueueCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Single `add` calls on `queue`.
    pub fn calls_for(&self, queue: QueueName) -> Vec<EnqueueCall> {
        self.calls()
            .into_iter()
            .filter(|c| c.queue == queue)
            .collect()
    }

    /// `add` calls on `queue` (bulk jobs not counted).
    pub fn count(&self, queue: QueueName) -> usize {
        self.calls_for(queue).len()
    }

    /// `add_bulk` calls, in order.
    pub fn bulks(&self) -> Vec<(QueueName, Vec<NewJob>)> {
        self.bulks.lock().unwrap().clone()
    }

    /// Every job, single and bulk, in call order of each kind (singles first).
    pub fn all_jobs(&self) -> Vec<EnqueueCall> {
        let mut out = self.calls();
        for (queue, jobs) in self.bulks() {
            for job in jobs {
                out.push(EnqueueCall {
                    queue,
                    name: job.name,
                    data: job.data,
                    opts: job.opts,
                });
            }
        }
        out
    }

    /// No `add` and no `add_bulk` call at all.
    pub fn is_empty(&self) -> bool {
        self.calls.lock().unwrap().is_empty() && self.bulks.lock().unwrap().is_empty()
    }

    fn result_for(&self, opts: &JobOptions) -> AddResult {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        AddResult {
            job_id: opts.job_id.clone().unwrap_or_else(|| n.to_string()),
            created: true,
        }
    }
}

impl Enqueuer for FakeEnqueuer {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let call = EnqueueCall {
            queue,
            name: name.to_owned(),
            data,
            opts,
        };
        Box::pin(async move {
            let result = self.result_for(&call.opts);
            self.calls.lock().unwrap().push(call);
            match self.fail_with.lock().unwrap().clone() {
                Some(e) => Err(e),
                None => Ok(result),
            }
        })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async move {
            let results = jobs.iter().map(|j| self.result_for(&j.opts)).collect();
            self.bulks.lock().unwrap().push((queue, jobs));
            match self.fail_with.lock().unwrap().clone() {
                Some(e) => Err(e),
                None => Ok(results),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// RecordingLog: keeps tracing events (replaces logger spies)
// ---------------------------------------------------------------------------

/// One captured `tracing` event. Field names are the Rust (snake_case)
/// names of the TypeScript log object keys (`jobId` -> `job_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

impl LogEntry {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

/// Captures every `tracing` event of the current thread while it lives
/// (`tracing::subscriber::set_default`). Use with `#[test]` or the default
/// current-thread `#[tokio::test]`, so spawned tasks log on the same thread.
pub struct RecordingLog {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

impl RecordingLog {
    pub fn install() -> RecordingLog {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            entries: Arc::clone(&entries),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        RecordingLog {
            entries,
            _guard: guard,
        }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Entries at `level`.
    pub fn at(&self, level: Level) -> Vec<LogEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.level == level)
            .collect()
    }

    /// The first entry with exactly this message.
    pub fn find(&self, message: &str) -> Option<LogEntry> {
        self.entries().into_iter().find(|e| e.message == message)
    }

    /// The first entry at `level` whose message contains `part`.
    pub fn find_containing(&self, level: Level, part: &str) -> Option<LogEntry> {
        self.at(level)
            .into_iter()
            .find(|e| e.message.contains(part))
    }

    pub fn has(&self, level: Level, message: &str) -> bool {
        self.at(level).iter().any(|e| e.message == message)
    }
}

struct CaptureLayer {
    entries: Arc<Mutex<Vec<LogEntry>>>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl FieldVisitor {
    fn put(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_owned());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.to_string());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.to_string());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.entries.lock().unwrap().push(LogEntry {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }

    fn on_new_span(&self, _attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {}

    fn on_record(&self, _span: &Id, _values: &Record<'_>, _ctx: Context<'_, S>) {}
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

/// A private in-memory database with the `chatbotx-db` migrations applied.
pub async fn migrated_db() -> Arc<Db> {
    let db = Db::open_in_memory().expect("open in-memory db");
    db.migrate().await.expect("migrate");
    Arc::new(db)
}

/// Sorted keys of a JSON object (empty for other values).
pub fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}
