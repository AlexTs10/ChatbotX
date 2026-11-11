//! Event bus on SQLite (spec 4.2), replacing `packages/event-bus` (Redis
//! Streams with consumer groups). One durable `event_entry` queue per
//! stream, batch claim, ack = delete, reclaim of idle entries, dead letters
//! in `event_dead_letter`.

pub mod bus;
pub mod config;
pub mod emit;
pub mod entry;
pub mod store;
pub mod worker;

pub use bus::{
    EventBus, EventBusError, EventListener, ListenerFn, ListenerOutcome, Listeners,
    PayloadTransformer, event_bus_message_id_key,
};
pub use config::{BusKind, EventBusConfig, PayloadSchema};
pub use emit::{EventBuses, EventEmitter, emit};
pub use store::{DeadLetter, EventStore, EventStoreError, SqliteEventStore, StreamEntry};
pub use worker::{ConsumerBus, EventModule, EventWorker, default_consumer_name};
