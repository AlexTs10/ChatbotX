//! Graceful shutdown coordinator (spec 2.13, port of
//! `apps/worker/src/lib/shutdown.ts`).
//!
//! One `Shutdown` per process: named closers (one per worker) run
//! concurrently with `allSettled` semantics, then the finalizers (realtime
//! flush, completion registry close) run, then the injected exit function
//! is called once. `exit` is injected so tests record the code instead of
//! ending the process.

use std::sync::Arc;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::ctx::BoxError;
use crate::enqueue::BoxFuture;

/// `process.exit(code)`.
pub type ExitFn = Arc<dyn Fn(i32) + Send + Sync>;

/// `ensureBootstrapped()`: runs once before the first worker starts.
pub type BootstrapFn = Arc<dyn Fn() -> BoxFuture<'static, Result<(), BoxError>> + Send + Sync>;

/// A closer (`onShutdown`) or a finalizer (`onShutdownFinalize`).
pub type ShutdownTask = Box<dyn FnOnce() -> BoxFuture<'static, Result<(), BoxError>> + Send>;

/// The `start` function given to [`Shutdown::run_worker`].
pub type StartFn = Box<dyn FnOnce() -> BoxFuture<'static, Result<(), BoxError>> + Send>;

/// Fatal-error drain bound when no timeout is configured (30 000 ms).
pub const FATAL_SHUTDOWN_TIMEOUT_MS: u64 = 30_000;

/// Why shutdown was requested (`requestShutdown(reason, exitCode)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownReason {
    Sigterm,
    Sigint,
    /// A panic in a task (`uncaughtException`).
    UncaughtException,
    /// A failed detached future (`unhandledRejection`).
    UnhandledRejection,
    /// A worker failed to start.
    StartupFailure(String),
}

impl ShutdownReason {
    /// `"SIGTERM"`, `"SIGINT"`, `"uncaughtException"`, `"unhandledRejection"`, ...
    pub fn as_str(&self) -> &str {
        todo!()
    }
}

/// The crash kinds that the installed handlers route to a fatal exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKind {
    /// `uncaughtException`: logged fatal "Uncaught exception".
    UncaughtException,
    /// `unhandledRejection`: logged fatal "Unhandled rejection".
    UnhandledRejection,
}

/// A small cancel token on `tokio::sync::watch` (no `tokio-util`).
#[derive(Debug, Clone)]
pub struct CancelToken {
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl CancelToken {
    pub fn new() -> CancelToken {
        todo!()
    }

    /// Cancels the token (`AbortController.abort()`); idempotent.
    pub fn cancel(&self) {
        todo!()
    }

    /// `signal.aborted`.
    pub fn is_cancelled(&self) -> bool {
        todo!()
    }

    /// Resolves once the token is cancelled.
    pub async fn cancelled(&self) {
        todo!()
    }
}

impl Default for CancelToken {
    fn default() -> CancelToken {
        CancelToken::new()
    }
}

/// The process-wide shutdown coordinator.
pub struct Shutdown {
    _private: (),
}

impl Shutdown {
    /// `timeout_ms` = `WORKER_SHUTDOWN_TIMEOUT_MS` (`None` = no deadline for
    /// a graceful drain; a fatal drain is bounded at
    /// [`FATAL_SHUTDOWN_TIMEOUT_MS`]).
    pub fn new(timeout_ms: Option<u64>, exit: ExitFn, bootstrap: BootstrapFn) -> Arc<Shutdown> {
        todo!()
    }

    /// `onShutdown(name, closer)`. The first registration installs the
    /// signal and crash handlers (once). A closer registered after shutdown
    /// began is ignored and logged (warn, field `worker`).
    pub fn on_shutdown(self: &Arc<Self>, name: &str, closer: ShutdownTask) {
        todo!()
    }

    /// `onShutdownFinalize(name, fn)`: the first finalizer for a name wins.
    pub fn on_shutdown_finalize(&self, name: &str, finalizer: ShutdownTask) {
        todo!()
    }

    /// Names of the registered closers, in registration order.
    pub fn closer_names(&self) -> Vec<String> {
        todo!()
    }

    /// `isShutdownRequested()`.
    pub fn is_requested(&self) -> bool {
        todo!()
    }

    /// Cancelled when shutdown is requested (dispatchers stop claiming).
    pub fn cancel_token(&self) -> CancelToken {
        todo!()
    }

    /// `requestShutdown(reason, exitCode)`: logs "Shutting down", runs every
    /// closer concurrently, then the finalizers, then calls `exit` once
    /// (0, or 1 when a closer failed or `exit_code` says so). A second
    /// request while one runs does nothing more. The returned future ends
    /// when this request's drain ends.
    pub fn request(
        self: &Arc<Self>,
        reason: ShutdownReason,
        exit_code: Option<i32>,
    ) -> BoxFuture<'static, ()> {
        todo!()
    }

    /// Test hook: whether a drain deadline timer is armed.
    pub fn deadline_armed(&self) -> bool {
        todo!()
    }

    /// `runWorker(name, start)`: bootstrap, then `start` unless shutdown
    /// began ("Worker start skipped: shutting down"). A bootstrap or start
    /// error logs "Failed to start worker" (field `worker`) and requests
    /// shutdown with exit code 1; a start error after shutdown began logs
    /// "Worker start aborted by shutdown" (warn) instead.
    pub fn run_worker(self: &Arc<Self>, name: &str, start: StartFn) -> JoinHandle<()> {
        todo!()
    }

    /// The handlers installed so far: `"SIGINT"`, `"SIGTERM"`,
    /// `"uncaughtException"` (task panic), `"unhandledRejection"`; each
    /// appears once however often [`Shutdown::on_shutdown`] ran.
    pub fn installed_handlers(&self) -> Vec<&'static str> {
        todo!()
    }

    /// What the installed crash handlers do: log fatal ("Uncaught
    /// exception" / "Unhandled rejection", field `err`) and request
    /// shutdown with exit code 1.
    pub fn report_crash(
        self: &Arc<Self>,
        kind: CrashKind,
        err: BoxError,
    ) -> BoxFuture<'static, ()> {
        todo!()
    }
}
