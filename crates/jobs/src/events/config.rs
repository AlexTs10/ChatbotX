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
        EventBusConfig {
            stream_key: stream_key.to_owned(),
            consumer_group: consumer_group.to_owned(),
            schemas,
            max_len: 0,
            read_batch_size: 500,
            claim_interval_ms: Some(30_000),
            claim_idle_ms: 180_000,
            claim_batch_size: 500,
            process_timeout_ms: 120_000,
            dead_letter_stream_key: format!("{stream_key}:dead"),
            dead_letter_max_len: 100_000,
            max_deliveries: 5,
            enable_selective_retry: None,
        }
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

/// Payload check of the configured buses: a JSON object. The typed zod
/// schemas of flow-config move to the Phase 3 event types.
fn object_payload(payload: &Value) -> bool {
    payload.is_object()
}

fn schemas(types: &[&str]) -> BTreeMap<String, PayloadSchema> {
    types
        .iter()
        .map(|t| ((*t).to_owned(), object_payload as PayloadSchema))
        .collect()
}

/// `messageEventBus.getConfig()`.
pub fn message_bus_config() -> EventBusConfig {
    EventBusConfig {
        max_len: 100_000,
        ..EventBusConfig::new(
            "events:message",
            "message-events-group",
            schemas(message_event_types()),
        )
    }
}

/// `flowEventBus.getConfig()`.
pub fn flow_bus_config() -> EventBusConfig {
    EventBusConfig {
        max_len: 100_000,
        ..EventBusConfig::new(
            "flow:events",
            "flow-events-group",
            schemas(flow_event_types()),
        )
    }
}

/// `analyticsDashboardEventSchemas` keys.
const DASHBOARD_EVENT_TYPES: [&str; 1] = ["analytics:dashboard"];
/// `errorLogEventSchemas` keys.
const ERROR_LOG_EVENT_TYPES: [&str; 1] = ["error-log:recorded"];

/// `dashboardEventBus.getConfig()`.
pub fn dashboard_bus_config() -> EventBusConfig {
    EventBusConfig {
        max_len: 500_000,
        dead_letter_max_len: 100_000,
        dead_letter_stream_key: "events:analytics-dashboard:dead".to_owned(),
        enable_selective_retry: Some(true),
        max_deliveries: 5,
        ..EventBusConfig::new(
            "events:analytics-dashboard",
            "analytics-dashboard-events-group",
            schemas(&DASHBOARD_EVENT_TYPES),
        )
    }
}

/// `errorLogEventBus.getConfig()`: sized from a byte budget (a `detail` can
/// be 8 KB).
pub fn error_log_bus_config() -> EventBusConfig {
    EventBusConfig {
        max_len: 50_000,
        dead_letter_max_len: 10_000,
        dead_letter_stream_key: "events:error-log:dead".to_owned(),
        // Writes are idempotent on the producer-minted row id.
        enable_selective_retry: Some(true),
        max_deliveries: 5,
        ..EventBusConfig::new(
            "events:error-log",
            "error-log-events-group",
            schemas(&ERROR_LOG_EVENT_TYPES),
        )
    }
}

/// The config of `kind`.
pub fn bus_config(kind: BusKind) -> EventBusConfig {
    match kind {
        BusKind::Message => message_bus_config(),
        BusKind::Flow => flow_bus_config(),
        BusKind::Dashboard => dashboard_bus_config(),
        BusKind::ErrorLog => error_log_bus_config(),
    }
}

/// `messageEventTypeSchema.options` (flow-config).
pub fn message_event_types() -> &'static [&'static str] {
    &[
        "message:sent",
        "message:delivered",
        "message:seen",
        "message:failed",
        "message:received",
    ]
}

/// `flowEventTypeSchema.options` (flow-config).
pub fn flow_event_types() -> &'static [&'static str] {
    &["flow:clicked", "flow:ref"]
}

/// `MessageEventBusByType` / `FlowEventBusByType` / the dashboard and
/// error-log types: the bus that owns `event_type`, `None` if unknown.
pub fn bus_for_event_type(event_type: &str) -> Option<BusKind> {
    if message_event_types().contains(&event_type) {
        Some(BusKind::Message)
    } else if flow_event_types().contains(&event_type) {
        Some(BusKind::Flow)
    } else if DASHBOARD_EVENT_TYPES.contains(&event_type) {
        Some(BusKind::Dashboard)
    } else if ERROR_LOG_EVENT_TYPES.contains(&event_type) {
        Some(BusKind::ErrorLog)
    } else {
        None
    }
}
