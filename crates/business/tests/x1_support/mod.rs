//! Shared fakes of the P3.6-X1 tests: a recording job enqueuer (replaces
//! `queue.add` spies), a recording tracing layer (replaces logger spies) and
//! a fake active-condition check (replaces `hasActiveTriggers` mocks).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use chatbotx_business::events::BoxFuture;
use chatbotx_business::events::cache::{ActiveConditions, CacheError, ConditionSource, Conditions};
use chatbotx_jobs::{AddResult, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

/// One `queue.add` call.
#[derive(Debug, Clone, PartialEq)]
pub struct AddCall {
    pub queue: QueueName,
    pub name: String,
    pub data: Value,
    pub opts: JobOptions,
}

/// Records every add; `fail_next` makes the next add fail.
#[derive(Default)]
pub struct RecordingEnqueuer {
    calls: Mutex<Vec<AddCall>>,
    fail_next: Mutex<Option<String>>,
}

impl RecordingEnqueuer {
    pub fn new() -> Arc<RecordingEnqueuer> {
        Arc::new(RecordingEnqueuer::default())
    }

    pub fn calls(&self) -> Vec<AddCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn fail_next(&self, message: &str) {
        *self.fail_next.lock().unwrap() = Some(message.to_string());
    }
}

impl Enqueuer for RecordingEnqueuer {
    fn add(
        &self,
        queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_else(|| "1".to_string());
        self.calls.lock().unwrap().push(AddCall {
            queue,
            name: name.to_string(),
            data,
            opts,
        });
        let failure = self.fail_next.lock().unwrap().take();
        Box::pin(async move {
            match failure {
                Some(message) => Err(JobsError::Store(message)),
                None => Ok(AddResult {
                    job_id,
                    created: true,
                }),
            }
        })
    }

    fn add_bulk(
        &self,
        queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let mut out = Vec::new();
        for job in jobs {
            out.push(AddResult {
                job_id: job.opts.job_id.clone().unwrap_or_default(),
                created: true,
            });
            self.calls.lock().unwrap().push(AddCall {
                queue,
                name: job.name,
                data: job.data,
                opts: job.opts,
            });
        }
        Box::pin(async move { Ok(out) })
    }
}

/// One `hasActive` call: (workspace id, event types, source id).
pub type HasActiveCall = (String, Vec<String>, Option<String>);

/// A fake `hasActiveTriggers` / `hasActiveWebhooks` with a fixed answer.
pub struct FakeActive {
    pub answer: Mutex<bool>,
    calls: Mutex<Vec<HasActiveCall>>,
}

impl FakeActive {
    pub fn new(answer: bool) -> Arc<FakeActive> {
        Arc::new(FakeActive {
            answer: Mutex::new(answer),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> Vec<HasActiveCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ActiveConditions for FakeActive {
    fn has_active<'a>(
        &'a self,
        workspace_id: &'a str,
        event_types: &'a [&'a str],
        source_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        self.calls.lock().unwrap().push((
            workspace_id.to_string(),
            event_types.iter().map(|t| (*t).to_string()).collect(),
            source_id.map(str::to_string),
        ));
        let answer = *self.answer.lock().unwrap();
        Box::pin(async move { Ok(answer) })
    }
}

/// A fake `findMany` of the active rows: every call returns `conditions`.
pub struct FakeConditions {
    pub conditions: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeConditions {
    pub fn new(conditions: &[(&str, Option<&str>)]) -> Arc<FakeConditions> {
        Arc::new(FakeConditions {
            conditions: Mutex::new(
                conditions
                    .iter()
                    .map(|(t, s)| ((*t).to_string(), s.map(str::to_string)))
                    .collect(),
            ),
        })
    }
}

impl ConditionSource for FakeConditions {
    fn conditions<'a>(
        &'a self,
        _workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Conditions, CacheError>> {
        let rows = self.conditions.lock().unwrap().clone();
        Box::pin(async move { Ok(rows) })
    }
}

/// One captured `tracing` event (fields in snake_case).
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

/// Captures the `tracing` events of the current thread while it lives (use
/// with the current-thread `#[tokio::test]`).
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
        let guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        RecordingLog {
            entries,
            _guard: guard,
        }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// The first entry with exactly this message.
    pub fn find(&self, message: &str) -> Option<LogEntry> {
        self.entries().into_iter().find(|e| e.message == message)
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
}
