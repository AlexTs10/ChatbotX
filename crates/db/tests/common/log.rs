//! Log capture for the ported repository tests (P3.3): replaces the vitest `logger` spies.
//!
//! Include with:
//! ```ignore
//! #[path = "common/log.rs"]
//! mod log_common;
//! ```
//!
//! The repository code logs on the db reader/writer threads, so the recorder is the
//! process-wide default subscriber (installed once per test binary). Tests look for their
//! own message; they never assert that a message is absent.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// One captured event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

struct Recorder {
    entries: Mutex<Vec<LogEntry>>,
    next_span: AtomicU64,
}

static RECORDER: OnceLock<&'static Recorder> = OnceLock::new();

fn recorder() -> &'static Recorder {
    RECORDER.get_or_init(|| {
        let rec: &'static Recorder = Box::leak(Box::new(Recorder {
            entries: Mutex::new(Vec::new()),
            next_span: AtomicU64::new(1),
        }));
        let _ = tracing::subscriber::set_global_default(Handle(rec));
        rec
    })
}

/// Installs the recorder (idempotent). Call before the code under test runs.
pub fn install() {
    let _ = recorder();
}

/// Every event captured so far in this test binary.
pub fn entries() -> Vec<LogEntry> {
    recorder()
        .entries
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The captured events at `level` with exactly `message`.
pub fn find(level: Level, message: &str) -> Vec<LogEntry> {
    entries()
        .into_iter()
        .filter(|e| e.level == level && e.message == message)
        .collect()
}

struct Handle(&'static Recorder);

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
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

impl Subscriber for Handle {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.0.next_span.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let entry = LogEntry {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        };
        self.0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry);
    }
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}
