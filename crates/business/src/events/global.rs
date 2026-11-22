//! The process-wide event channels: the flow-trigger / webhook bus
//! (`emitToAllEmitters`) and the durable buses (`emit` of `@chatbotx.io/event-bus`),
//! installed once by the server.
//!
//! ponytail: process-wide, as the original's module singletons. Before
//! [`install`] (tests, tools) every send is a no-op; a test that must see
//! events passes its own bus to the service it calls.

use std::sync::OnceLock;

use chatbotx_jobs::events::EventBuses;
use serde_json::{Map, Value};

use super::{Event, EventBus, TriggerEvent};

static TRIGGERS: OnceLock<EventBus> = OnceLock::new();
static BUSES: OnceLock<EventBuses> = OnceLock::new();

/// Installs the channels; a second call keeps the first ones.
pub fn install(triggers: EventBus, buses: Option<EventBuses>) {
    let _ = TRIGGERS.set(triggers);
    if let Some(buses) = buses {
        let _ = BUSES.set(buses);
    }
}

/// The object fields of `value` without `null`s (`undefined` keys of the original).
fn defined(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().filter(|(_, v)| !v.is_null()).collect(),
        _ => Map::new(),
    }
}

/// One flow-trigger / webhook event (`BaseEventEmitter.emit`).
pub async fn trigger(event_type: &str, workspace_id: i64, contact_id: i64, metadata: Value) {
    let Some(bus) = TRIGGERS.get() else { return };
    bus.publish(Event::Trigger(TriggerEvent {
        event_type: event_type.to_owned(),
        workspace_id: workspace_id.to_string(),
        contact_id: contact_id.to_string(),
        metadata: defined(metadata),
    }))
    .await;
}

/// `emit("analytics:dashboard", payload)`: a bus failure is logged, never raised.
pub async fn dashboard(payload: Value) {
    let Some(buses) = BUSES.get() else { return };
    let payload = Value::Object(defined(payload));
    if let Some(future) = chatbotx_jobs::events::emit(buses, "analytics:dashboard", payload)
        && let Err(err) = future.await
    {
        tracing::warn!(err = %err, "analytics:dashboard emit failed");
    }
}
