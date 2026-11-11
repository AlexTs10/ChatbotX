//! `emit(type, payload)` (`packages/event-bus/src/index.ts`): routes an
//! event type to the bus that owns it.

use std::sync::Arc;

use serde_json::Value;

use crate::enqueue::BoxFuture;
use crate::events::bus::{EventBus, EventBusError};

/// The emit side of a bus. `Err` = a synchronous failure (the bus threw
/// before returning a promise).
pub trait EventEmitter: Send + Sync {
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Value,
    ) -> Result<BoxFuture<'a, Result<String, EventBusError>>, EventBusError>;
}

impl EventEmitter for EventBus {
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Value,
    ) -> Result<BoxFuture<'a, Result<String, EventBusError>>, EventBusError> {
        todo!()
    }
}

/// The four configured buses.
#[derive(Clone)]
pub struct EventBuses {
    pub message: Arc<dyn EventEmitter>,
    pub flow: Arc<dyn EventEmitter>,
    pub dashboard: Arc<dyn EventEmitter>,
    pub error_log: Arc<dyn EventEmitter>,
}

/// Routes `event_type` to its bus. An unknown type resolves to `""`. A
/// synchronous bus failure is logged ("Failed to emit event: <type>",
/// fields `err`, `payload`) and returns `None` (`undefined`).
pub fn emit<'a>(
    buses: &'a EventBuses,
    event_type: &'a str,
    payload: Value,
) -> Option<BoxFuture<'a, Result<String, EventBusError>>> {
    todo!()
}
