//! Shared test support for the ported P3.1 frozen tests. Test code, not
//! crate API. Included with `mod support;`.

#![allow(dead_code)]

pub mod variables;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_core::{Clock, TestClock};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

/// The `ENCRYPTION_KEY` of `packages/vitest-config/src/setup-env.ts`.
pub const TEST_ENCRYPTION_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// `encryptUtils` with the test key and the real clock.
pub fn encryption() -> Encryption {
    Encryption::new(Keys::new(TEST_ENCRYPTION_KEY).expect("test key"))
}

/// `encryptUtils` with the test key and a manual clock.
pub fn encryption_at(now_ms: i64) -> (Encryption, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(now_ms));
    let enc = Encryption::with_clock(
        Keys::new(TEST_ENCRYPTION_KEY).expect("test key"),
        clock.clone() as Arc<dyn Clock>,
    );
    (enc, clock)
}

/// `Date.now()`.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
        .as_millis() as i64
}

/// `Date.parse` of an ISO instant with `Z` (test fixtures only).
pub fn iso_ms(text: &str) -> i64 {
    chatbotx_core::js::parse_date(text).unwrap_or_else(|| panic!("bad fixture date {text}"))
}

/// `Date#toISOString`.
pub fn iso(ms: i64) -> String {
    chatbotx_core::js::to_iso_string(ms).expect("valid time value")
}

// ---------------------------------------------------------------------------
// RecordingLog: keeps tracing events (replaces logger spies)
// ---------------------------------------------------------------------------

/// One captured `tracing` event. Field names are the Rust (snake_case)
/// names of the TypeScript log object keys (`bytesBefore` -> `bytes_before`).
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

/// Captures every `tracing` event of the current thread while it lives.
/// Use with `#[test]` or the default current-thread `#[tokio::test]`.
pub struct RecordingLog {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

impl RecordingLog {
    /// Records every level.
    pub fn install() -> RecordingLog {
        RecordingLog::install_at(LevelFilter::TRACE)
    }

    /// Records events at `max_level` or more severe (a logger created with
    /// that level).
    pub fn install_at(max_level: LevelFilter) -> RecordingLog {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            entries: Arc::clone(&entries),
        }
        .with_filter(max_level);
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

impl<S: Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
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
