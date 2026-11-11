//! The events worker entry (`apps/worker/src/events/worker.ts`): runs the
//! event worker under the shutdown coordinator (`runWorker("events", ..)`),
//! with a closer that stops it and the realtime-flush finalizer.

use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::ctx::BoxError;
use crate::enqueue::BoxFuture;
use crate::shutdown::Shutdown;

/// `startWorker` / `stopWorker` of the configured modules (fakes in tests).
pub trait EventWorkerControl: Send + Sync {
    fn start(&self) -> BoxFuture<'static, Result<(), BoxError>>;
    fn stop(&self) -> BoxFuture<'static, Result<(), BoxError>>;
}

/// `flushAllPendingWorkspaceRealtimeEvents`.
pub type RealtimeFlush = Arc<dyn Fn() -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// Registers the `events` closer (stops the worker; a stop started while
/// `start` was still running is repeated once `start` ends) and the
/// realtime-flush finalizer, then runs `start` through
/// [`Shutdown::run_worker`] under the name `events`.
pub fn run_events_worker(
    shutdown: &Arc<Shutdown>,
    worker: Arc<dyn EventWorkerControl>,
    flush_realtime: RealtimeFlush,
) -> JoinHandle<()> {
    todo!()
}
