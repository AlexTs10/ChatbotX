//! Port of `packages/business/__tests__/realtime-relay-metrics.test.ts`
//! (P2.8 spec 4.6, R235-R236). `vi.setSystemTime`/`advanceTimersByTime` is
//! a `TestClock`; the logger mock is `RecordingLog`.

mod support;

use std::sync::Arc;

use chatbotx_rt::TestClock;
use chatbotx_rt::relay_metrics::RelayMetrics;
use support::RecordingLog;
use tracing::Level;

/// 2026-01-01T00:00:00.000Z.
const NEW_YEAR_MS: i64 = 1_767_225_600_000;

/// Port of: packages/business/__tests__/realtime-relay-metrics.test.ts:24 "realtime relay metrics > flushes only after its time window unless forced, then resets"
#[test]
fn realtime_relay_metrics_flushes_only_after_its_time_window_unless_forced_then_resets() {
    let log = RecordingLog::install();
    let clock = Arc::new(TestClock::new(NEW_YEAR_MS));
    let metrics = RelayMetrics::new(clock.clone());
    metrics.reset();
    metrics.record_event("typing", 12);

    metrics.flush(false);
    assert!(log.with_message("realtime_relay_metric").is_empty());

    clock.advance(10_000);
    metrics.flush(false);
    let lines = log.with_message("realtime_relay_metric");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].level, Level::INFO);
    assert_eq!(lines[0].field("bytes"), Some("12"));
    assert_eq!(lines[0].field("events"), Some("1"));
    assert_eq!(lines[0].field("metric"), Some("realtime_relay"));

    metrics.flush(true);
    assert_eq!(log.with_message("realtime_relay_metric").len(), 1);
}

/// Port of: packages/business/__tests__/realtime-relay-metrics.test.ts:48 "realtime relay metrics > logs backpressure once per metrics window"
#[test]
fn realtime_relay_metrics_logs_backpressure_once_per_metrics_window() {
    let log = RecordingLog::install();
    let metrics = RelayMetrics::new(Arc::new(TestClock::new(NEW_YEAR_MS)));

    assert!(metrics.record_backpressure());
    assert!(!metrics.record_backpressure());

    metrics.flush(true);
    let lines = log.with_message("realtime_relay_metric");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].field("backpressure_logged"), Some("true"));
    assert_eq!(lines[0].field("errors"), Some("2"));

    assert!(metrics.record_backpressure());
}
