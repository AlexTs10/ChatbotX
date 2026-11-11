//! Port of `packages/event-bus/__tests__/index.test.ts` (spec 5.3 "index"):
//! `emit` routes an event type to its bus.

#[path = "support/events.rs"]
mod events_support;
mod support;

use std::sync::Arc;

use chatbotx_jobs::events::{EventBuses, emit};
use events_support::FakeEmitter;
use serde_json::json;
use support::RecordingLog;
use tracing::Level;

/// Port of: packages/event-bus/__tests__/index.test.ts:28 "emit > routes known event types to their owning bus"
#[tokio::test]
async fn routes_known_event_types_to_their_owning_bus() {
    let message = FakeEmitter::resolving("message-id");
    let flow = FakeEmitter::resolving("flow-id");
    let dashboard = FakeEmitter::resolving("dashboard-id");
    let error_log = FakeEmitter::resolving("error-log-id");
    let buses = EventBuses {
        message: Arc::clone(&message) as _,
        flow: Arc::clone(&flow) as _,
        dashboard: Arc::clone(&dashboard) as _,
        error_log: Arc::clone(&error_log) as _,
    };

    let routed = |event_type: &'static str| {
        let buses = buses.clone();
        async move {
            emit(&buses, event_type, json!({}))
                .expect("a promise")
                .await
                .expect("resolves")
        }
    };
    assert_eq!(routed("message:sent").await, "message-id");
    assert_eq!(routed("flow:clicked").await, "flow-id");
    assert_eq!(routed("analytics:dashboard").await, "dashboard-id");
    assert_eq!(routed("error-log:recorded").await, "error-log-id");
    assert_eq!(routed("unknown:event").await, "");

    assert_eq!(message.call_count(), 1);
    assert_eq!(flow.call_count(), 1);
    assert_eq!(dashboard.call_count(), 1);
    assert_eq!(error_log.call_count(), 1);
}

/// Port of: packages/event-bus/__tests__/index.test.ts:60 "emit > logs synchronous emit routing failures"
#[test]
fn logs_synchronous_emit_routing_failures() {
    let log = RecordingLog::install();
    let buses = EventBuses {
        message: FakeEmitter::throwing("emit failed"),
        flow: FakeEmitter::resolving("flow-id"),
        dashboard: FakeEmitter::resolving("dashboard-id"),
        error_log: FakeEmitter::resolving("error-log-id"),
    };

    assert!(emit(&buses, "message:sent", json!({})).is_none());

    let entry = log
        .find("Failed to emit event: message:sent")
        .expect("error logged");
    assert_eq!(entry.level, Level::ERROR);
    assert!(entry.field("err").is_some());
    assert_eq!(entry.field("payload"), Some("{}"));
}
