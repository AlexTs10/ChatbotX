//! Stream configuration (`EventBusConfig`) and the four configured buses
//! (`src/{message,flow,dashboard,error-log}/event-bus.ts`).

use std::collections::BTreeMap;

use serde_json::Value;

/// A payload schema (the zod schema of one event type): `true` = valid.
pub type PayloadSchema = fn(&Value) -> bool;

/// `EventBusConfig`. Durations in ms; `claim_interval_ms: None` =
/// `Number.POSITIVE_INFINITY` (never reclaim).
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    pub stream_key: String,
    pub consumer_group: String,
    pub schemas: BTreeMap<String, PayloadSchema>,
    /// Cap on unacked entries (the `XADD MAXLEN ~` intent).
    pub max_len: u64,
    /// Entries per read (default 500).
    pub read_batch_size: u32,
    /// Reclaim at most once per interval (default 30 000).
    pub claim_interval_ms: Option<u64>,
    /// Reclaim entries idle longer than this (default 180 000).
    pub claim_idle_ms: u64,
    /// Entries per reclaim.
    pub claim_batch_size: u32,
    /// Batch watchdog (default 120 000).
    pub process_timeout_ms: u64,
    /// Default `<stream_key>:dead`.
    pub dead_letter_stream_key: String,
    pub dead_letter_max_len: u64,
    /// Dead-letter when the delivery count is greater than this (default 5).
    pub max_deliveries: u32,
    /// `None` = not set (ack the whole batch even on handler failure).
    pub enable_selective_retry: Option<bool>,
}

impl EventBusConfig {
    /// A config with the `BaseEventBus` defaults for every optional field.
    pub fn new(
        stream_key: &str,
        consumer_group: &str,
        schemas: BTreeMap<String, PayloadSchema>,
    ) -> EventBusConfig {
        todo!()
    }
}

/// The four configured buses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusKind {
    Message,
    Flow,
    Dashboard,
    ErrorLog,
}

/// `messageEventBus.getConfig()`.
pub fn message_bus_config() -> EventBusConfig {
    todo!()
}

/// `flowEventBus.getConfig()`.
pub fn flow_bus_config() -> EventBusConfig {
    todo!()
}

/// `dashboardEventBus.getConfig()`.
pub fn dashboard_bus_config() -> EventBusConfig {
    todo!()
}

/// `errorLogEventBus.getConfig()`.
pub fn error_log_bus_config() -> EventBusConfig {
    todo!()
}

/// The config of `kind`.
pub fn bus_config(kind: BusKind) -> EventBusConfig {
    todo!()
}

/// `messageEventTypeSchema.options` (flow-config).
pub fn message_event_types() -> &'static [&'static str] {
    todo!()
}

/// `flowEventTypeSchema.options` (flow-config).
pub fn flow_event_types() -> &'static [&'static str] {
    todo!()
}

/// `MessageEventBusByType` / `FlowEventBusByType` / the dashboard and
/// error-log types: the bus that owns `event_type`, `None` if unknown.
pub fn bus_for_event_type(event_type: &str) -> Option<BusKind> {
    todo!()
}
