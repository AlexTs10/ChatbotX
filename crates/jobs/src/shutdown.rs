//! Graceful shutdown coordinator (spec 2.13, port of
//! `apps/worker/src/lib/shutdown.ts`).
//!
//! One `Shutdown` per process: named closers (one per worker) run
//! concurrently with `allSettled` semantics, then the finalizers (realtime
//! flush, completion registry close) run, then the injected exit function
//! is called once. `exit` is injected so tests record the code instead of
//! ending the process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OnceCell, watch};
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
        match self {
            ShutdownReason::Sigterm => "SIGTERM",
            ShutdownReason::Sigint => "SIGINT",
            ShutdownReason::UncaughtException => "uncaughtException",
            ShutdownReason::UnhandledRejection => "unhandledRejection",
            ShutdownReason::StartupFailure(text) => text,
        }
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
        let (tx, rx) = watch::channel(false);
        CancelToken {
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Cancels the token (`AbortController.abort()`); idempotent.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    /// `signal.aborted`.
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once the token is cancelled.
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        // The sender lives in `self`, so `wait_for` only ends on a cancel.
        let _ = rx.wait_for(|cancelled| *cancelled).await;
    }
}

impl Default for CancelToken {
    fn default() -> CancelToken {
        CancelToken::new()
    }
}

type Named = (String, ShutdownTask);

#[derive(Default)]
struct State {
    closers: Vec<Named>,
    finalizers: Vec<Named>,
    installed: bool,
    requested: bool,
}

/// The process-wide shutdown coordinator.
pub struct Shutdown {
    timeout_ms: Option<u64>,
    exit: ExitFn,
    bootstrap: BootstrapFn,
    bootstrapped: OnceCell<Result<(), String>>,
    state: Mutex<State>,
    exited: AtomicBool,
    deadline_armed: AtomicBool,
    token: CancelToken,
}

/// The bootstrap error, rebuilt from its message for each waiting entry.
#[derive(Debug)]
struct BootstrapError(String);

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BootstrapError {}

impl Shutdown {
    /// `timeout_ms` = `WORKER_SHUTDOWN_TIMEOUT_MS` (`None` = no deadline for
    /// a graceful drain; a fatal drain is bounded at
    /// [`FATAL_SHUTDOWN_TIMEOUT_MS`]).
    pub fn new(timeout_ms: Option<u64>, exit: ExitFn, bootstrap: BootstrapFn) -> Arc<Shutdown> {
        Arc::new(Shutdown {
            timeout_ms,
            exit,
            bootstrap,
            bootstrapped: OnceCell::new(),
            state: Mutex::new(State::default()),
            exited: AtomicBool::new(false),
            deadline_armed: AtomicBool::new(false),
            token: CancelToken::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Calls the exit function once.
    fn exit_once(&self, code: i32) {
        if !self.exited.swap(true, Ordering::SeqCst) {
            (self.exit)(code);
        }
    }

    /// `onShutdown(name, closer)`. The first registration installs the
    /// signal and crash handlers (once). A closer registered after shutdown
    /// began is ignored and logged (warn, field `worker`).
    pub fn on_shutdown(self: &Arc<Self>, name: &str, closer: ShutdownTask) {
        let install = {
            let mut state = self.lock();
            if state.requested {
                drop(state);
                tracing::warn!(
                    worker = name,
                    "Ignoring worker shutdown closer registered after shutdown began"
                );
                return;
            }
            match state.closers.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => slot.1 = closer,
                None => state.closers.push((name.to_owned(), closer)),
            }
            !std::mem::replace(&mut state.installed, true)
        };
        if install {
            self.install_signal_handlers();
        }
    }

    /// SIGINT / SIGTERM request a graceful drain. Crashes reach the fatal
    /// path through [`Shutdown::report_crash`] (task supervision).
    fn install_signal_handlers(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            for (kind, reason) in [
                (SignalKind::interrupt(), ShutdownReason::Sigint),
                (SignalKind::terminate(), ShutdownReason::Sigterm),
            ] {
                let weak = Arc::downgrade(self);
                handle.spawn(async move {
                    let Ok(mut stream) = signal(kind) else {
                        return;
                    };
                    if stream.recv().await.is_some()
                        && let Some(shutdown) = weak.upgrade()
                    {
                        shutdown.request(reason, None).await;
                    }
                });
            }
        }
        #[cfg(not(unix))]
        {
            let weak = Arc::downgrade(self);
            handle.spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok()
                    && let Some(shutdown) = weak.upgrade()
                {
                    shutdown.request(ShutdownReason::Sigint, None).await;
                }
            });
        }
    }

    /// `onShutdownFinalize(name, fn)`: the first finalizer for a name wins.
    pub fn on_shutdown_finalize(&self, name: &str, finalizer: ShutdownTask) {
        let mut state = self.lock();
        if state.finalizers.iter().any(|(n, _)| n == name) {
            return;
        }
        state.finalizers.push((name.to_owned(), finalizer));
    }

    /// Names of the registered closers, in registration order.
    pub fn closer_names(&self) -> Vec<String> {
        self.lock().closers.iter().map(|(n, _)| n.clone()).collect()
    }

    /// `isShutdownRequested()`.
    pub fn is_requested(&self) -> bool {
        self.lock().requested
    }

    /// Cancelled when shutdown is requested (dispatchers stop claiming).
    pub fn cancel_token(&self) -> CancelToken {
        self.token.clone()
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
        let taken = {
            let mut state = self.lock();
            if state.requested {
                None
            } else {
                state.requested = true;
                Some((
                    std::mem::take(&mut state.closers),
                    std::mem::take(&mut state.finalizers),
                ))
            }
        };
        let Some((closers, finalizers)) = taken else {
            return Box::pin(async {});
        };
        self.token.cancel();
        let this = Arc::clone(self);
        Box::pin(async move {
            let exit_code = exit_code.unwrap_or(0);
            let names: Vec<String> = closers.iter().map(|(n, _)| n.clone()).collect();
            let pending = Arc::new(Mutex::new(names.clone()));
            let timeout_ms = this.timeout_ms.or(if exit_code == 0 {
                None
            } else {
                Some(FATAL_SHUTDOWN_TIMEOUT_MS)
            });
            tracing::info!(reason = reason.as_str(), workers = ?names, "Shutting down");

            let drain = {
                let pending = Arc::clone(&pending);
                async move {
                    let closers_failed = close_all(closers, Some(pending)).await;
                    let finalizers_failed = close_all(finalizers, None).await;
                    closers_failed || finalizers_failed
                }
            };
            match timeout_ms {
                None => {
                    let failed = drain.await;
                    this.exit_once(if failed { 1 } else { exit_code });
                }
                Some(timeout_ms) => {
                    this.deadline_armed.store(true, Ordering::SeqCst);
                    tokio::select! {
                        failed = drain => {
                            this.deadline_armed.store(false, Ordering::SeqCst);
                            this.exit_once(if failed { 1 } else { exit_code });
                        }
                        () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                            let still: Vec<String> = pending
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .clone();
                            tracing::error!(
                                reason = reason.as_str(),
                                pending = ?still,
                                timeout_ms,
                                "Shutdown timed out with workers still draining"
                            );
                            this.exit_once(1);
                        }
                    }
                }
            }
        })
    }

    /// Test hook: whether a drain deadline timer is armed.
    pub fn deadline_armed(&self) -> bool {
        self.deadline_armed.load(Ordering::SeqCst)
    }

    async fn ensure_bootstrapped(&self) -> Result<(), BoxError> {
        let bootstrap = Arc::clone(&self.bootstrap);
        let result = self
            .bootstrapped
            .get_or_init(|| async move { bootstrap().await.map_err(|e| e.to_string()) })
            .await;
        result
            .clone()
            .map_err(|message| Box::new(BootstrapError(message)) as BoxError)
    }

    /// `runWorker(name, start)`: bootstrap, then `start` unless shutdown
    /// began ("Worker start skipped: shutting down"). A bootstrap or start
    /// error logs "Failed to start worker" (field `worker`) and requests
    /// shutdown with exit code 1; a start error after shutdown began logs
    /// "Worker start aborted by shutdown" (warn) instead.
    pub fn run_worker(self: &Arc<Self>, name: &str, start: StartFn) -> JoinHandle<()> {
        let this = Arc::clone(self);
        let name = name.to_owned();
        tokio::spawn(async move {
            let result = match this.ensure_bootstrapped().await {
                Ok(()) => {
                    if this.is_requested() {
                        tracing::warn!(worker = %name, "Worker start skipped: shutting down");
                        return;
                    }
                    start().await
                }
                Err(e) => Err(e),
            };
            let Err(err) = result else {
                return;
            };
            // A start racing a shutdown fails by design; not a startup failure.
            if this.is_requested() {
                tracing::warn!(err = %err, worker = %name, "Worker start aborted by shutdown");
                return;
            }
            tracing::error!(err = %err, worker = %name, "Failed to start worker");
            this.request(
                ShutdownReason::StartupFailure(format!("{name} failed to start")),
                Some(1),
            )
            .await;
        })
    }

    /// The handlers installed so far: `"SIGINT"`, `"SIGTERM"`,
    /// `"uncaughtException"` (task panic), `"unhandledRejection"`; each
    /// appears once however often [`Shutdown::on_shutdown`] ran.
    pub fn installed_handlers(&self) -> Vec<&'static str> {
        if self.lock().installed {
            vec![
                "SIGINT",
                "SIGTERM",
                "uncaughtException",
                "unhandledRejection",
            ]
        } else {
            Vec::new()
        }
    }

    /// What the installed crash handlers do: log fatal ("Uncaught
    /// exception" / "Unhandled rejection", field `err`) and request
    /// shutdown with exit code 1.
    pub fn report_crash(
        self: &Arc<Self>,
        kind: CrashKind,
        err: BoxError,
    ) -> BoxFuture<'static, ()> {
        // tracing has no fatal level: ERROR (DECISIONS P2.6 porter results).
        let reason = match kind {
            CrashKind::UncaughtException => {
                tracing::error!(err = %err, "Uncaught exception");
                ShutdownReason::UncaughtException
            }
            CrashKind::UnhandledRejection => {
                tracing::error!(err = %err, "Unhandled rejection");
                ShutdownReason::UnhandledRejection
            }
        };
        self.request(reason, Some(1))
    }
}

/// `closeAll`: every task concurrently (`allSettled`); a failed one is
/// logged ("Error during worker shutdown", field `worker`). Returns whether
/// any failed.
async fn close_all(tasks: Vec<Named>, pending: Option<Arc<Mutex<Vec<String>>>>) -> bool {
    let mut set = tokio::task::JoinSet::new();
    let names: Vec<String> = tasks.iter().map(|(n, _)| n.clone()).collect();
    for (index, (name, task)) in tasks.into_iter().enumerate() {
        let pending = pending.clone();
        let future = task();
        set.spawn(async move {
            let result = future.await;
            if let Some(pending) = pending {
                pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .retain(|n| *n != name);
            }
            (index, result)
        });
    }
    let mut results: Vec<(usize, Result<(), String>)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((index, result)) => results.push((index, result.map_err(|e| e.to_string()))),
            Err(e) => results.push((usize::MAX, Err(e.to_string()))),
        }
    }
    results.sort_by_key(|(i, _)| *i);
    let mut failed = false;
    for (index, result) in results {
        if let Err(err) = result {
            failed = true;
            let worker = names.get(index).cloned().unwrap_or_default();
            tracing::error!(err = %err, worker = %worker, "Error during worker shutdown");
        }
    }
    failed
}
