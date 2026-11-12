//! Port of `packages/logger`: log-safety helpers (`redact.ts`) and the pino
//! hooks (`index.ts`), mapped to `tracing`.
//!
//! - pino's `err` serializer + `redactSecrets` → [`redact_err_serializer`];
//! - the `logMethod` hook → [`scrub_log_args`] (and [`scrub_secrets_in_string`]
//!   for any message a caller builds);
//! - `logDiagnostic` → [`log_diagnostic`]: a `tracing` event at `debug`, or
//!   `info` with `LOG_DEBUG=true`, whose data is one JSON `data` field.

use std::fmt;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::env::{Env, ProcessEnv};

/// `DEFAULT_MAX_LOG_CHARS`.
pub const DEFAULT_MAX_LOG_CHARS: usize = 16_000;

const MAX_REDACT_DEPTH: usize = 64;
const REDACTED: &str = "[redacted]";
const TOO_DEEP: &str = "[omitted: too deeply nested]";

/// Keys whose value is always redacted (exact, case-insensitive).
const SENSITIVE_KEYS: &[&str] = &[
    "access_token",
    "refresh_token",
    "auth_token",
    "id_token",
    "verify_token",
    "token",
    "client_secret",
    "app_secret",
    "secret",
    "secret_key",
    "private_key",
    "password",
    "signature",
    "api_key",
    "apikey",
    "authorization",
    "sdp",
];

const SENSITIVE_QUERY_PARAMS: &[&str] = &[
    "access_token",
    "refresh_token",
    "fb_exchange_token",
    "auth_token",
    "id_token",
    "client_secret",
    "app_secret",
    "signature",
    "api_key",
    "apikey",
    "key",
];

fn sensitive_query_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r#"(?i)([?&](?:{})=)[^&#\s"']+"#,
            SENSITIVE_QUERY_PARAMS.join("|")
        ))
        .expect("valid regex")
    })
}

/// `scrubSecretsInString`: redacts the value of sensitive query params
/// embedded in a string (`?access_token=…` → `?access_token=[redacted]`).
pub fn scrub_secrets_in_string(value: &str) -> String {
    sensitive_query_re()
        .replace_all(value, "${1}[redacted]")
        .into_owned()
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEYS.contains(&lower.as_str())
}

fn redact_at_depth(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(text) => Value::String(scrub_secrets_in_string(text)),
        Value::Array(_) | Value::Object(_) if depth >= MAX_REDACT_DEPTH => {
            Value::String(TOO_DEEP.to_owned())
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_at_depth(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let value = if is_sensitive_key(key) {
                        Value::String(REDACTED.to_owned())
                    } else {
                        redact_at_depth(item, depth + 1)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `redactSecrets`: a new value with every sensitive-named key replaced by
/// `"[redacted]"` and every string scrubbed; depth-bounded.
pub fn redact_secrets(value: &Value) -> Value {
    redact_at_depth(value, 0)
}

/// A thrown error as the logger sees it: `name`, `message`, `stack` and the
/// other own enumerable properties (e.g. `originError`, `request`).
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedError {
    pub name: String,
    pub message: String,
    pub stack: Option<String>,
    pub fields: Map<String, Value>,
}

impl LoggedError {
    /// `new Error(message)`: name `Error`, a stack that starts with
    /// `Error: <message>`.
    pub fn new(message: impl Into<String>) -> LoggedError {
        let message = message.into();
        LoggedError {
            name: "Error".to_owned(),
            stack: Some(format!("Error: {message}\n    at <rust>")),
            message,
            fields: Map::new(),
        }
    }

    /// An error from a Rust error value (its `Display` is the message).
    pub fn from_error(err: &(dyn std::error::Error + 'static)) -> LoggedError {
        let mut logged = LoggedError::new(err.to_string());
        if let Some(source) = err.source() {
            logged.fields.insert(
                "cause".to_owned(),
                LoggedError::from_error(source).serialize(),
            );
        }
        logged
    }

    /// `Object.assign(error, { key: value })`.
    pub fn with_field(mut self, key: &str, value: Value) -> LoggedError {
        self.fields.insert(key.to_owned(), value);
        self
    }

    /// `Object.assign(error, { key: nestedError })`.
    pub fn with_error_field(self, key: &str, nested: LoggedError) -> LoggedError {
        let serialized = nested.serialize();
        self.with_field(key, serialized)
    }

    /// `pino.stdSerializers.err`: `{ type, message, stack, ...fields }`.
    pub fn serialize(&self) -> Value {
        let mut out = Map::new();
        out.insert("type".to_owned(), Value::String(self.name.clone()));
        out.insert("message".to_owned(), Value::String(self.message.clone()));
        if let Some(stack) = &self.stack {
            out.insert("stack".to_owned(), Value::String(stack.clone()));
        }
        for (key, value) in &self.fields {
            out.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Value::Object(out)
    }
}

/// What `redactErrSerializer` / `toLogSafeError` receive: an `Error`, or any
/// other thrown value.
#[derive(Debug, Clone, PartialEq)]
pub enum Thrown {
    Error(LoggedError),
    Value(Value),
}

/// `redactErrSerializer`: the error expanded with pino's serializer, then
/// [`redact_secrets`]; a plain value is redacted as it is.
pub fn redact_err_serializer(err: &Thrown) -> Value {
    match err {
        Thrown::Error(error) => redact_secrets(&error.serialize()),
        Thrown::Value(value) => redact_secrets(value),
    }
}

/// `toLogSafeError`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogSafeError {
    pub name: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
}

/// `toLogSafeError`: name + scrubbed message and stack; nested fields are
/// dropped. A non-error becomes `{ name: "NonError", message: String(err) }`.
pub fn to_log_safe_error(err: &Thrown) -> LogSafeError {
    match err {
        Thrown::Error(error) => LogSafeError {
            name: error.name.clone(),
            message: scrub_secrets_in_string(&error.message),
            stack: error
                .stack
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(scrub_secrets_in_string),
        },
        Thrown::Value(value) => LogSafeError {
            name: "NonError".to_owned(),
            message: scrub_secrets_in_string(&js_string_of(value)),
            stack: None,
        },
    }
}

/// `String(value)` for a JSON value.
fn js_string_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), crate::js::number_to_string),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string_of(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `capText`: at most `max_chars` UTF-16 units, then
/// `…[truncated N chars]`; a surrogate pair on the boundary is not split.
pub fn cap_text(text: &str, max_chars: usize) -> String {
    let total = crate::js::utf16_len(text);
    if total <= max_chars {
        return text.to_owned();
    }
    let kept = crate::js::slice_utf16(text, 0, max_chars);
    let end = crate::js::utf16_len(kept);
    format!("{kept}…[truncated {} chars]", total - end)
}

/// One argument of a log call (pino's `logger.info(obj, msg)`).
#[derive(Debug, Clone, PartialEq)]
pub enum LogArg {
    /// A message string.
    Message(String),
    /// A bare `Error`.
    Error(LoggedError),
    /// A merge object; `err` is its `err` key when that is an `Error`.
    Object {
        err: Option<LoggedError>,
        fields: Map<String, Value>,
    },
}

/// `scrubLogArgs`: message strings are scrubbed; when an error is logged
/// with no message, a scrubbed copy of its message is appended (so the
/// derived `msg` cannot carry a credential).
pub fn scrub_log_args(args: &[LogArg]) -> Vec<LogArg> {
    let mut out: Vec<LogArg> = args.to_vec();
    let mut has_message = false;
    for arg in &mut out {
        if let LogArg::Message(text) = arg {
            *text = scrub_secrets_in_string(text);
            has_message = true;
        }
    }
    if !has_message {
        let derived = match out.first() {
            Some(LogArg::Error(error)) => Some(error.message.clone()),
            Some(LogArg::Object {
                err: Some(error), ..
            }) => Some(error.message.clone()),
            _ => None,
        };
        if let Some(message) = derived {
            out.push(LogArg::Message(scrub_secrets_in_string(&message)));
        }
    }
    out
}

/// The level `logDiagnostic` writes at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Debug,
}

impl DiagnosticLevel {
    /// `"info"` / `"debug"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticLevel::Info => "info",
            DiagnosticLevel::Debug => "debug",
        }
    }
}

/// `resolveDiagnosticLevel`: `info` only when `LOG_DEBUG` is exactly
/// `"true"`.
pub fn resolve_diagnostic_level(env: &dyn Env) -> DiagnosticLevel {
    if env.var("LOG_DEBUG").as_deref() == Some("true") {
        DiagnosticLevel::Info
    } else {
        DiagnosticLevel::Debug
    }
}

/// The process diagnostic level, read once from the environment.
pub fn diagnostic_level() -> DiagnosticLevel {
    static LEVEL: OnceLock<DiagnosticLevel> = OnceLock::new();
    *LEVEL.get_or_init(|| resolve_diagnostic_level(&ProcessEnv))
}

/// `logDiagnostic` at the process level ([`diagnostic_level`]).
pub fn log_diagnostic(build_data: impl FnOnce() -> Map<String, Value>, message: &str) {
    log_diagnostic_at(diagnostic_level(), build_data, message);
}

/// `logDiagnostic` at an explicit level: `build_data` runs only when that
/// level is enabled; its object is the event's `data` field (JSON).
pub fn log_diagnostic_at(
    level: DiagnosticLevel,
    build_data: impl FnOnce() -> Map<String, Value>,
    message: &str,
) {
    // The data is built when a layer that accepted the event formats the
    // field, so `build_data` never runs for a level nobody records (the
    // callsite's cached interest can say "enabled" for a subscriber whose
    // per-layer filter then drops the event).
    let data = LazyData::new(build_data);
    match level {
        DiagnosticLevel::Info => tracing::info!(data = %data, "{message}"),
        DiagnosticLevel::Debug => tracing::debug!(data = %data, "{message}"),
    }
}

/// The `data` field of a diagnostic: `build_data`'s JSON, built on the
/// first format and reused by later layers.
struct LazyData<F> {
    build: std::cell::Cell<Option<F>>,
    json: std::cell::OnceCell<String>,
}

impl<F: FnOnce() -> Map<String, Value>> LazyData<F> {
    fn new(build: F) -> Self {
        LazyData {
            build: std::cell::Cell::new(Some(build)),
            json: std::cell::OnceCell::new(),
        }
    }
}

impl<F: FnOnce() -> Map<String, Value>> fmt::Display for LazyData<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = self.json.get_or_init(|| match self.build.take() {
            Some(build) => Value::Object(build()).to_string(),
            None => String::new(),
        });
        f.write_str(json)
    }
}

/// The `tracing` level filter of a pino `LOG_LEVEL` (`silent` = off; an
/// unknown value = info, pino's default).
pub fn level_filter_from_log_level(value: Option<&str>) -> tracing::level_filters::LevelFilter {
    use tracing::level_filters::LevelFilter;
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some("silent") => LevelFilter::OFF,
        Some("fatal") | Some("error") => LevelFilter::ERROR,
        Some("warn") => LevelFilter::WARN,
        Some("debug") => LevelFilter::DEBUG,
        Some("trace") => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    }
}
