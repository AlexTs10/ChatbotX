//! In-process event bus (P3.6-X1), the dispatch side of `packages/events`
//! (`event-dispatcher.ts`: `emitToAllEmitters` over the trigger and webhook
//! emitters). A publisher sends one typed [`Event`]; every subscriber gets
//! it in turn. A failing subscriber is logged and the next one still runs
//! (`Promise.allSettled` + `logger.error`).
//!
//! The durable Redis-stream buses (`packages/event-bus`) are
//! `chatbotx_jobs::events` (P2.6); this bus is the in-request fan-out that
//! feeds them (and the trigger queue) through subscribers.

pub mod cache;
pub mod contact_inbox;
pub mod context;
pub mod emitter;
pub mod registry;
pub mod trigger;

use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Map, Value};

pub use chatbotx_kv::BoxFuture;

/// One flow-trigger event: `BaseEventEmitter.emit(eventType, { workspaceId,
/// contactId, metadata })`. `metadata.sourceId` (when set) narrows the
/// triggers that listen for it.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerEvent {
    /// The `triggerEventTypes` member (`"tagApplied"`, `"newContact"`, ...).
    pub event_type: String,
    pub workspace_id: String,
    pub contact_id: String,
    pub metadata: Map<String, Value>,
}

impl TriggerEvent {
    /// `metadata.sourceId` as a string, if any.
    pub fn source_id(&self) -> Option<&str> {
        self.metadata.get("sourceId").and_then(Value::as_str)
    }
}

/// An event on the in-process bus.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A flow-trigger / webhook event (the `emit*` functions of
    /// `event-dispatcher.ts`).
    Trigger(TriggerEvent),
    /// An event for the durable buses (`packages/event-bus` `emit(type,
    /// payload)`: `contact:created`, `message:*`, ...).
    Bus { event_type: String, payload: Value },
}

/// A subscriber error (logged by the bus, never returned to the publisher).
pub type SubscriberError = String;

/// One subscriber of the bus (`TriggerEventEmitter`, `WebhookEventEmitter`,
/// a test recorder).
pub trait Subscriber: Send + Sync {
    /// A short name for the logs.
    fn name(&self) -> &str;

    /// Handles one event.
    fn handle<'a>(&'a self, event: &'a Event) -> BoxFuture<'a, Result<(), SubscriberError>>;
}

/// The bus: a list of subscribers. Cloning shares the list.
#[derive(Clone, Default)]
pub struct EventBus {
    subscribers: Arc<RwLock<Vec<Arc<dyn Subscriber>>>>,
}

impl EventBus {
    /// A bus without subscribers (publishing is a no-op).
    pub fn new() -> EventBus {
        EventBus::default()
    }

    /// Adds `subscriber`; it gets every event published after this call.
    pub fn subscribe(&self, subscriber: Arc<dyn Subscriber>) {
        self.subscribers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(subscriber);
    }

    /// The number of subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Sends `event` to every subscriber, in subscription order, and waits for
    /// them. A subscriber error is logged ("Failed to emit event: <type>").
    pub async fn publish(&self, event: Event) {
        let subscribers = self
            .subscribers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for subscriber in subscribers {
            if let Err(err) = subscriber.handle(&event).await {
                tracing::error!(
                    err = %err,
                    subscriber = subscriber.name(),
                    "Failed to emit event: {}",
                    event_name(&event)
                );
            }
        }
    }

    /// [`EventBus::publish`] of each event, in order.
    pub async fn publish_all(&self, events: impl IntoIterator<Item = Event>) {
        for event in events {
            self.publish(event).await;
        }
    }
}

/// The event type of `event` (for the logs).
pub fn event_name(event: &Event) -> &str {
    match event {
        Event::Trigger(trigger) => &trigger.event_type,
        Event::Bus { event_type, .. } => event_type,
    }
}

/// A subscriber that keeps every event (tests and wiring checks).
#[derive(Default)]
pub struct RecordingSubscriber {
    events: Mutex<Vec<Event>>,
}

impl RecordingSubscriber {
    pub fn new() -> Arc<RecordingSubscriber> {
        Arc::new(RecordingSubscriber::default())
    }

    /// The events received so far.
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The trigger events received so far.
    pub fn trigger_events(&self) -> Vec<TriggerEvent> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Trigger(trigger) => Some(trigger),
                Event::Bus { .. } => None,
            })
            .collect()
    }
}

impl Subscriber for RecordingSubscriber {
    fn name(&self) -> &str {
        "recording"
    }

    fn handle<'a>(&'a self, event: &'a Event) -> BoxFuture<'a, Result<(), SubscriberError>> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.clone());
        Box::pin(std::future::ready(Ok(())))
    }
}
