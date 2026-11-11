//! `BaseEventBus` (spec 4.2): emit with schema check and payload
//! transformer, consumer group initialization, the consume loop (reclaim,
//! read, group by type, run listeners with a watchdog, ack / selective ack,
//! dead letters), stop and deregister.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::clock::Clock;
use crate::ctx::BoxError;
use crate::enqueue::BoxFuture;
use crate::events::config::EventBusConfig;
use crate::events::store::EventStore;
use crate::shutdown::CancelToken;

/// The payload key that carries the entry id (`EVENT_BUS_MESSAGE_ID`).
pub fn event_bus_message_id_key() -> &'static str {
    todo!()
}

/// Bus errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventBusError {
    /// The store failed; the text is the cause.
    Store(String),
    /// Anything else, with its message.
    Other(String),
}

impl fmt::Display for EventBusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for EventBusError {}

/// What a listener returns: the ids it failed (only used with selective
/// retry; those stay unacked).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListenerOutcome {
    pub failed_message_ids: Vec<String>,
}

/// A listener handler: the payloads of one batch of its event type (each
/// with the entry id under [`event_bus_message_id_key`]) and the batch
/// cancel token (the `AbortSignal`, cancelled on the watchdog timeout).
pub type ListenerFn = Arc<
    dyn Fn(Vec<Value>, CancelToken) -> BoxFuture<'static, Result<ListenerOutcome, BoxError>>
        + Send
        + Sync,
>;

/// `{ name, handler }`.
#[derive(Clone)]
pub struct EventListener {
    pub name: String,
    pub handler: ListenerFn,
}

/// Listeners by event type.
pub type Listeners = BTreeMap<String, Vec<EventListener>>;

/// `setPayloadHandler((eventType, payload) => payload)`.
pub type PayloadTransformer = Arc<dyn Fn(&str, Value) -> Value + Send + Sync>;

/// One bus over one stream and one consumer group.
pub struct EventBus {
    _private: (),
}

impl EventBus {
    pub fn new(
        store: Arc<dyn EventStore>,
        config: EventBusConfig,
        clock: Arc<dyn Clock>,
    ) -> EventBus {
        todo!()
    }

    /// `getConfig()`.
    pub fn config(&self) -> &EventBusConfig {
        todo!()
    }

    /// `setPayloadHandler(fn)`.
    pub fn with_payload_handler(self, transformer: PayloadTransformer) -> EventBus {
        todo!()
    }

    /// `cloneForGroup(group)`: same stream and options, another group.
    pub fn clone_for_group(&self, consumer_group: &str) -> EventBus {
        todo!()
    }

    /// Creates the consumer group once; an existing group is not an error.
    pub async fn initialize(&self) -> Result<(), EventBusError> {
        todo!()
    }

    /// Schema check (invalid → `Ok("")`, nothing written), transformer,
    /// append with `max_len` and the clock time; returns the entry id.
    pub async fn emit(&self, event_type: &str, payload: Value) -> Result<String, EventBusError> {
        todo!()
    }

    /// One iteration of the consume loop (rate-limited reclaim, then one
    /// read of `read_batch_size`, then processing and ack).
    pub async fn consume_once(
        &self,
        consumer: &str,
        listeners: &Listeners,
    ) -> Result<(), EventBusError> {
        todo!()
    }

    /// `startConsuming(consumer, listeners)`: loops until [`EventBus::stop`];
    /// an iteration error is logged ("[EventBus] consume loop error") and
    /// retried after 1000 ms; when the loop ends the reader is closed.
    pub async fn start_consuming(&self, consumer: &str, listeners: Listeners) {
        todo!()
    }

    /// The token that [`EventBus::stop`] cancels (for code that must stop
    /// the bus from elsewhere).
    pub fn stop_token(&self) -> CancelToken {
        todo!()
    }

    /// Stops the consume loop (synchronous).
    pub fn stop(&self) {
        todo!()
    }

    /// Removes the consumer when it has no unacked entries; otherwise keeps
    /// it and logs why.
    pub async fn deregister(&self) {
        todo!()
    }
}
