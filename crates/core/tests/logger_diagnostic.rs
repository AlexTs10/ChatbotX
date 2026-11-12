//! Port of `packages/logger/__tests__/diagnostic.test.ts`. The pino capture
//! logger becomes a `tracing` recorder with a level filter; "re-import the
//! module with LOG_DEBUG stubbed" becomes `resolve_diagnostic_level` on an
//! injected env, passed to `log_diagnostic_at`. pino level 20/30 = DEBUG/INFO.

mod support;

use std::cell::Cell;

use chatbotx_core::logger::{DiagnosticLevel, log_diagnostic_at, resolve_diagnostic_level};
use serde_json::{Map, Value, json};
use support::RecordingLog;
use tracing::Level;
use tracing::level_filters::LevelFilter;

fn foo_bar() -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("foo".to_owned(), json!("bar"));
    data
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:6 "resolveDiagnosticLevel > returns info when LOG_DEBUG is exactly 'true'"
#[test]
fn resolvediagnosticlevel_returns_info_when_log_debug_is_exactly_true() {
    assert_eq!(
        resolve_diagnostic_level(&[("LOG_DEBUG", "true")]),
        DiagnosticLevel::Info
    );
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:10 "resolveDiagnosticLevel > returns debug when LOG_DEBUG is 'false'"
#[test]
fn resolvediagnosticlevel_returns_debug_when_log_debug_is_false() {
    assert_eq!(
        resolve_diagnostic_level(&[("LOG_DEBUG", "false")]),
        DiagnosticLevel::Debug
    );
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:14 "resolveDiagnosticLevel > returns debug when LOG_DEBUG is unset"
#[test]
fn resolvediagnosticlevel_returns_debug_when_log_debug_is_unset() {
    assert_eq!(
        resolve_diagnostic_level(&chatbotx_core::env::EmptyEnv),
        DiagnosticLevel::Debug
    );
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:18 "resolveDiagnosticLevel > returns debug when LOG_DEBUG is '1'"
#[test]
fn resolvediagnosticlevel_returns_debug_when_log_debug_is_1() {
    assert_eq!(
        resolve_diagnostic_level(&[("LOG_DEBUG", "1")]),
        DiagnosticLevel::Debug
    );
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:22 "resolveDiagnosticLevel > returns debug when LOG_DEBUG is 'TRUE' (strict match only)"
#[test]
fn resolvediagnosticlevel_returns_debug_when_log_debug_is_true_strict_match_only() {
    assert_eq!(
        resolve_diagnostic_level(&[("LOG_DEBUG", "TRUE")]),
        DiagnosticLevel::Debug
    );
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:56 "logDiagnostic > emits at debug by default (LOG_DEBUG unset)"
#[test]
fn logdiagnostic_emits_at_debug_by_default_log_debug_unset() {
    let level = resolve_diagnostic_level(&[("LOG_DEBUG", "")]);
    let log = RecordingLog::install_at(LevelFilter::DEBUG);

    log_diagnostic_at(level, foo_bar, "diagnostic message");

    let lines = log.entries();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].level, Level::DEBUG);
    let data: Value = serde_json::from_str(lines[0].field("data").unwrap()).unwrap();
    assert_eq!(data["foo"], "bar");
    assert_eq!(lines[0].message, "diagnostic message");
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:71 "logDiagnostic > promotes to info when LOG_DEBUG=true, surviving an info-level logger"
#[test]
fn logdiagnostic_promotes_to_info_when_log_debug_true_surviving_an_info_level_logger() {
    let level = resolve_diagnostic_level(&[("LOG_DEBUG", "true")]);
    let log = RecordingLog::install_at(LevelFilter::INFO);

    log_diagnostic_at(level, foo_bar, "diagnostic message");

    let lines = log.entries();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].level, Level::INFO);
    assert_eq!(lines[0].message, "diagnostic message");
}

/// Port of: packages/logger/__tests__/diagnostic.test.ts:82 "logDiagnostic > does not invoke buildData when the resolved level is disabled"
#[test]
fn logdiagnostic_does_not_invoke_builddata_when_the_resolved_level_is_disabled() {
    let level = resolve_diagnostic_level(&[("LOG_DEBUG", "")]);
    let log = RecordingLog::install_at(LevelFilter::INFO);
    let called = Cell::new(false);

    log_diagnostic_at(
        level,
        || {
            called.set(true);
            foo_bar()
        },
        "diagnostic message",
    );

    assert!(!called.get());
    assert_eq!(log.entries().len(), 0);
}
