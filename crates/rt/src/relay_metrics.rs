//! Producer relay metrics (`realtime-relay-metrics.ts`): one log line
//! `realtime_relay_metric` per 10 s window with traffic, backpressure
//! logged once per window.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chatbotx_kv::Clock;

/// `REALTIME_METRIC_WINDOW_MS`.
pub const REALTIME_METRIC_WINDOW_MS: i64 = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RelayWindow {
    backpressure_logged: bool,
    bytes: u64,
    errors: u64,
    event_types: BTreeMap<String, u64>,
    events: u64,
    flushes: u64,
    max_batch_events: u64,
    window_started_at: i64,
}

/// The relay window.
pub struct RelayMetrics {
    clock: Arc<dyn Clock>,
    window: Mutex<RelayWindow>,
}

impl RelayMetrics {
    fn window(&self) -> MutexGuard<'_, RelayWindow> {
        self.window.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn new(clock: Arc<dyn Clock>) -> RelayMetrics {
        let window = RelayWindow {
            window_started_at: clock.now_ms(),
            ..RelayWindow::default()
        };
        RelayMetrics {
            clock,
            window: Mutex::new(window),
        }
    }

    /// `recordRealtimeRelayEvent`.
    pub fn record_event(&self, event_type: &str, byte_length: usize) {
        let mut window = self.window();
        window.events += 1;
        window.bytes += byte_length as u64;
        *window.event_types.entry(event_type.to_owned()).or_default() += 1;
    }

    /// `recordRealtimeRelayFlush`.
    pub fn record_flush(&self, event_count: usize) {
        let mut window = self.window();
        window.flushes += 1;
        window.max_batch_events = window.max_batch_events.max(event_count as u64);
    }

    /// `recordRealtimeRelayError`.
    pub fn record_error(&self) {
        self.window().errors += 1;
    }

    /// `recordRealtimeRelayBackpressure`: counts an error; `true` only for
    /// the first call of the window (the caller logs then).
    pub fn record_backpressure(&self) -> bool {
        let mut window = self.window();
        window.errors += 1;
        if window.backpressure_logged {
            return false;
        }
        window.backpressure_logged = true;
        true
    }

    /// `flushRealtimeRelayMetrics(force)`: logs and resets a non-empty
    /// window once it is 10 s old, or at once when forced.
    pub fn flush(&self, force: bool) {
        let now = self.clock.now_ms();
        let mut window = self.window();
        let is_empty = window.events == 0 && window.flushes == 0 && window.errors == 0;
        if is_empty || (!force && now - window.window_started_at < REALTIME_METRIC_WINDOW_MS) {
            return;
        }
        let done = std::mem::replace(
            &mut *window,
            RelayWindow {
                window_started_at: now,
                ..RelayWindow::default()
            },
        );
        drop(window);
        tracing::info!(
            metric = "realtime_relay",
            backpressure_logged = done.backpressure_logged,
            bytes = done.bytes,
            errors = done.errors,
            event_types = ?done.event_types,
            events = done.events,
            flushes = done.flushes,
            max_batch_events = done.max_batch_events,
            window_started_at = done.window_started_at,
            "realtime_relay_metric"
        );
    }

    /// `resetRealtimeRelayMetricsForTests`.
    pub fn reset(&self) {
        *self.window() = RelayWindow {
            window_started_at: self.clock.now_ms(),
            ..RelayWindow::default()
        };
    }
}
