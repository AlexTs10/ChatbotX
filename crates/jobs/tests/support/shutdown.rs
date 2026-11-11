//! Shutdown test support: an exit recorder (replaces the `process.exit`
//! spy), bootstrap fakes (`ensureBootstrapped` mock), closer helpers and a
//! bounded wait (`vi.waitFor`). Complete test code.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chatbotx_jobs::shutdown::{BootstrapFn, ExitFn, Shutdown, ShutdownTask};
use chatbotx_jobs::{BoxError, BoxFuture};
use tokio::sync::oneshot;

/// Records every `exit(code)`.
#[derive(Clone, Default)]
pub struct ExitRecorder {
    codes: Arc<Mutex<Vec<i32>>>,
}

impl ExitRecorder {
    pub fn new() -> ExitRecorder {
        ExitRecorder::default()
    }

    pub fn exit_fn(&self) -> ExitFn {
        let codes = Arc::clone(&self.codes);
        Arc::new(move |code| codes.lock().unwrap().push(code))
    }

    pub fn codes(&self) -> Vec<i32> {
        self.codes.lock().unwrap().clone()
    }
}

/// A shared, ordered event log.
#[derive(Clone, Default)]
pub struct OrderLog {
    events: Arc<Mutex<Vec<String>>>,
}

impl OrderLog {
    pub fn new() -> OrderLog {
        OrderLog::default()
    }
    pub fn push(&self, event: &str) {
        self.events.lock().unwrap().push(event.to_owned());
    }
    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
    pub fn count(&self, event: &str) -> usize {
        self.events().iter().filter(|e| *e == event).count()
    }
}

/// A plain error with a message (`new Error(message)`).
pub fn err(message: &str) -> BoxError {
    message.to_owned().into()
}

/// `ensureBootstrapped` that resolves at once.
pub fn ok_bootstrap() -> BootstrapFn {
    Arc::new(|| Box::pin(async { Ok(()) }) as BoxFuture<'static, Result<(), BoxError>>)
}

/// `ensureBootstrapped` that records `bootstrap` in `log` and resolves.
pub fn recording_bootstrap(log: OrderLog) -> BootstrapFn {
    Arc::new(move || {
        log.push("bootstrap");
        Box::pin(async { Ok(()) }) as BoxFuture<'static, Result<(), BoxError>>
    })
}

/// `ensureBootstrapped` that rejects with `message`.
pub fn failing_bootstrap(message: &'static str) -> BootstrapFn {
    Arc::new(move || {
        Box::pin(async move { Err(err(message)) }) as BoxFuture<'static, Result<(), BoxError>>
    })
}

/// `ensureBootstrapped` that resolves when the returned sender fires.
pub fn gated_bootstrap() -> (BootstrapFn, oneshot::Sender<()>) {
    let (tx, rx) = oneshot::channel::<()>();
    let rx = Arc::new(Mutex::new(Some(rx)));
    let f: BootstrapFn = Arc::new(move || {
        let rx = rx.lock().unwrap().take();
        Box::pin(async move {
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            Ok(())
        }) as BoxFuture<'static, Result<(), BoxError>>
    });
    (f, tx)
}

/// A shutdown with no timeout, the recorder's exit and an OK bootstrap.
pub fn shutdown_with(exit: &ExitRecorder) -> Arc<Shutdown> {
    Shutdown::new(None, exit.exit_fn(), ok_bootstrap())
}

/// A closer that records `name` in `log` and resolves.
pub fn ok_closer(log: &OrderLog, name: &str) -> ShutdownTask {
    let log = log.clone();
    let name = name.to_owned();
    Box::new(move || {
        log.push(&name);
        Box::pin(async { Ok(()) })
    })
}

/// A closer that rejects with `message`.
pub fn failing_closer(message: &'static str) -> ShutdownTask {
    Box::new(move || Box::pin(async move { Err(err(message)) }))
}

/// A closer that never settles (`new Promise(() => undefined)`).
pub fn hanging_closer() -> ShutdownTask {
    Box::new(|| Box::pin(std::future::pending()))
}

/// `vi.waitFor`: polls `cond` every 10 ms; fails the test after 5 s
/// (virtual time when the runtime clock is paused).
pub async fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let polled = tokio::time::timeout(Duration::from_secs(5), async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(polled.is_ok(), "timed out waiting for: {what}");
}

/// Awaits `fut`, failing the test after 5 s.
pub async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("did not finish within 5 s")
}

/// Lets spawned tasks run (a few scheduler turns).
pub async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}
