//! The event worker (`packages/event-bus/src/worker.ts`): starts one
//! consumer per configured bus module and stops them on shutdown.

use std::sync::Arc;

use crate::enqueue::BoxFuture;
use crate::events::bus::{EventBus, EventBusError, Listeners};

/// The bus operations the worker uses (fakes in tests).
pub trait ConsumerBus: Send + Sync {
    fn initialize(&self) -> BoxFuture<'_, Result<(), EventBusError>>;

    /// The consume loop as a `'static` future (the worker spawns it).
    fn start_consuming(
        self: Arc<Self>,
        consumer: String,
        listeners: Listeners,
    ) -> BoxFuture<'static, ()>;

    /// Synchronous stop request.
    fn stop(&self);

    fn deregister(&self) -> BoxFuture<'_, Result<(), EventBusError>>;
}

impl ConsumerBus for EventBus {
    fn initialize(&self) -> BoxFuture<'_, Result<(), EventBusError>> {
        todo!()
    }

    fn start_consuming(
        self: Arc<Self>,
        consumer: String,
        listeners: Listeners,
    ) -> BoxFuture<'static, ()> {
        todo!()
    }

    fn stop(&self) {
        todo!()
    }

    fn deregister(&self) -> BoxFuture<'_, Result<(), EventBusError>> {
        todo!()
    }
}

/// `EventModule`: a bus and its listeners.
pub struct EventModule {
    pub bus: Arc<dyn ConsumerBus>,
    pub listeners: Listeners,
}

/// `worker-${HOSTNAME}-${pid}` (stable; no timestamp segment).
pub fn default_consumer_name(hostname: Option<&str>, pid: u32) -> String {
    todo!()
}

/// `startWorker` / `stopWorker`.
pub struct EventWorker {
    _private: (),
}

impl EventWorker {
    pub fn new() -> EventWorker {
        todo!()
    }

    /// `startWorker(modules, consumerName?)`: initializes each bus and
    /// spawns its consume loop (`None` = [`default_consumer_name`] from
    /// `HOSTNAME` and the pid).
    pub async fn start(
        &self,
        modules: Vec<EventModule>,
        consumer_name: Option<&str>,
    ) -> Result<(), EventBusError> {
        todo!()
    }

    /// `stopWorker()`: calls `stop` on every bus before returning; the
    /// future waits (at most 6000 ms) for the consume loops to end, then
    /// deregisters each bus.
    pub fn stop(&self) -> BoxFuture<'static, ()> {
        todo!()
    }
}

impl Default for EventWorker {
    fn default() -> EventWorker {
        EventWorker::new()
    }
}
