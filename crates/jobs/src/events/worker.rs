//! The event worker (`packages/event-bus/src/worker.ts`): starts one
//! consumer per configured bus module and stops them on shutdown.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinHandle;

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
        Box::pin(EventBus::initialize(self))
    }

    fn start_consuming(
        self: Arc<Self>,
        consumer: String,
        listeners: Listeners,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move { EventBus::start_consuming(&self, &consumer, listeners).await })
    }

    fn stop(&self) {
        EventBus::stop(self);
    }

    fn deregister(&self) -> BoxFuture<'_, Result<(), EventBusError>> {
        Box::pin(async move {
            EventBus::deregister(self).await;
            Ok(())
        })
    }
}

/// `EventModule`: a bus and its listeners.
pub struct EventModule {
    pub bus: Arc<dyn ConsumerBus>,
    pub listeners: Listeners,
}

/// `worker-${HOSTNAME}-${pid}` (stable; no timestamp segment).
pub fn default_consumer_name(hostname: Option<&str>, pid: u32) -> String {
    let host = hostname.filter(|h| !h.is_empty()).unwrap_or("chatbotx");
    format!("worker-{host}-{pid}")
}

/// How long `stop` waits for the consume loops before deregistering.
pub const SHUTDOWN_DRAIN_TIMEOUT_MS: u64 = 6000;

#[derive(Default)]
struct WorkerState {
    modules: Vec<Arc<dyn ConsumerBus>>,
    loops: Vec<JoinHandle<()>>,
}

/// `startWorker` / `stopWorker`.
pub struct EventWorker {
    state: Mutex<WorkerState>,
}

impl EventWorker {
    pub fn new() -> EventWorker {
        EventWorker {
            state: Mutex::new(WorkerState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `startWorker(modules, consumerName?)`: initializes each bus and
    /// spawns its consume loop (`None` = [`default_consumer_name`] from
    /// `HOSTNAME` and the pid).
    pub async fn start(
        &self,
        modules: Vec<EventModule>,
        consumer_name: Option<&str>,
    ) -> Result<(), EventBusError> {
        if !self.lock().modules.is_empty() {
            self.stop().await;
        }
        let name = match consumer_name {
            Some(name) => name.to_owned(),
            None => default_consumer_name(
                std::env::var("HOSTNAME").ok().as_deref(),
                std::process::id(),
            ),
        };
        for module in modules {
            self.lock().modules.push(Arc::clone(&module.bus));
            module.bus.initialize().await?;
            let consuming = Arc::clone(&module.bus).start_consuming(name.clone(), module.listeners);
            let handle = tokio::spawn(consuming);
            self.lock().loops.push(handle);
        }
        Ok(())
    }

    /// `stopWorker()`: calls `stop` on every bus before returning; the
    /// future waits (at most 6000 ms) for the consume loops to end, then
    /// deregisters each bus.
    pub fn stop(&self) -> BoxFuture<'static, ()> {
        let WorkerState { modules, loops } = std::mem::take(&mut *self.lock());
        for module in &modules {
            module.stop();
        }
        Box::pin(async move {
            let drain = async {
                for handle in loops {
                    let _ = handle.await;
                }
            };
            let _ =
                tokio::time::timeout(Duration::from_millis(SHUTDOWN_DRAIN_TIMEOUT_MS), drain).await;
            for module in &modules {
                if let Err(err) = module.deregister().await {
                    tracing::error!(err = %err, "[EventBus] deregister failed");
                }
            }
        })
    }
}

impl Default for EventWorker {
    fn default() -> EventWorker {
        EventWorker::new()
    }
}
