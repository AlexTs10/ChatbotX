//! `packages/utils/src/concurrency.ts`, on tokio.

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::{oneshot, watch};

/// `mapWithConcurrency`: `f` over `items` with at most `limit` calls in
/// flight; results in input order, each `Ok`/`Err` on its own (like
/// `Promise.allSettled`).
pub async fn map_with_concurrency<T, R, E, F, Fut>(
    items: Vec<T>,
    limit: usize,
    f: F,
) -> Vec<Result<R, E>>
where
    F: Fn(T, usize) -> Fut,
    Fut: Future<Output = Result<R, E>>,
{
    if items.is_empty() {
        return Vec::new();
    }
    let bounded = limit.clamp(1, items.len());
    futures::stream::iter(
        items
            .into_iter()
            .enumerate()
            .map(|(index, item)| f(item, index)),
    )
    .buffered(bounded)
    .collect()
    .await
}

/// The error of [`with_timeout`] when the deadline passes first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedOut {
    pub timeout_ms: u64,
}

impl fmt::Display for TimedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timed out after {}ms", self.timeout_ms)
    }
}

impl std::error::Error for TimedOut {}

/// `withTimeout`: the work's own result, or [`TimedOut`] once `timeout_ms`
/// passes. The work future is dropped at the deadline (Rust futures are
/// cancelled by dropping; a late failure is never observed).
pub async fn with_timeout<T, Fut>(work: Fut, timeout_ms: u64) -> Result<T, TimedOut>
where
    Fut: Future<Output = T>,
{
    tokio::time::timeout(Duration::from_millis(timeout_ms), work)
        .await
        .map_err(|_| TimedOut { timeout_ms })
}

/// An `AbortController`: [`AbortController::abort`] with a reason.
#[derive(Debug, Clone)]
pub struct AbortController {
    sender: Arc<watch::Sender<Option<String>>>,
}

/// The `AbortSignal` of an [`AbortController`].
#[derive(Debug, Clone)]
pub struct AbortSignal {
    receiver: watch::Receiver<Option<String>>,
}

impl Default for AbortController {
    fn default() -> AbortController {
        AbortController::new()
    }
}

impl AbortController {
    pub fn new() -> AbortController {
        let (sender, _) = watch::channel(None);
        AbortController {
            sender: Arc::new(sender),
        }
    }

    /// Aborts with `reason` (the first reason wins).
    pub fn abort(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.sender.send_if_modified(|current| {
            if current.is_none() {
                *current = Some(reason);
                true
            } else {
                false
            }
        });
    }

    pub fn signal(&self) -> AbortSignal {
        AbortSignal {
            receiver: self.sender.subscribe(),
        }
    }
}

impl AbortSignal {
    /// The reason, when aborted.
    pub fn reason(&self) -> Option<String> {
        self.receiver.borrow().clone()
    }

    /// Resolves with the reason once aborted.
    pub async fn aborted(&mut self) -> String {
        loop {
            if let Some(reason) = self.receiver.borrow_and_update().clone() {
                return reason;
            }
            if self.receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// A limiter failure: the caller gave up waiting, or the work failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError<E> {
    /// The signal aborted while waiting for a slot (its reason).
    Aborted(String),
    Work(E),
}

impl<E: fmt::Display> fmt::Display for LimitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LimitError::Aborted(reason) => f.write_str(reason),
            LimitError::Work(err) => err.fmt(f),
        }
    }
}

struct LimiterState {
    active: usize,
    next_id: u64,
    waiting: VecDeque<(u64, oneshot::Sender<()>)>,
}

/// `createConcurrencyLimiter`: a cap shared by every caller; callers past
/// the cap wait in order, a free slot passes straight to the next waiter.
#[derive(Clone)]
pub struct ConcurrencyLimiter {
    max_active: usize,
    state: Arc<Mutex<LimiterState>>,
}

impl fmt::Debug for ConcurrencyLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConcurrencyLimiter")
            .field("max_active", &self.max_active)
            .finish_non_exhaustive()
    }
}

/// Releases the slot when dropped (also when the work panics or the run is
/// cancelled).
struct SlotGuard {
    state: Arc<Mutex<LimiterState>>,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match state.waiting.pop_front() {
                Some((_, sender)) => {
                    if sender.send(()).is_ok() {
                        return;
                    }
                }
                None => {
                    state.active = state.active.saturating_sub(1);
                    return;
                }
            }
        }
    }
}

impl ConcurrencyLimiter {
    /// At most `max(1, limit)` runs at once.
    pub fn new(limit: usize) -> ConcurrencyLimiter {
        ConcurrencyLimiter {
            max_active: limit.max(1),
            state: Arc::new(Mutex::new(LimiterState {
                active: 0,
                next_id: 0,
                waiting: VecDeque::new(),
            })),
        }
    }

    /// Runs `work` once a slot is free. A free slot is taken at once, and
    /// `work` is called in the caller's turn (before this returns the
    /// future), like the original. With `signal`, a caller that aborts while
    /// it waits leaves the queue with the reason, without taking a slot.
    pub fn run<'a, T, E, W, Fut>(
        &'a self,
        work: W,
        signal: Option<AbortSignal>,
    ) -> impl Future<Output = Result<T, LimitError<E>>> + 'a
    where
        W: FnOnce() -> Fut + 'a,
        Fut: Future<Output = Result<T, E>> + 'a,
        T: 'a,
        E: 'a,
    {
        enum Start<W, Fut> {
            Now(Fut),
            Wait(W, oneshot::Receiver<()>, u64),
            Aborted(String),
        }
        let start = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.active < self.max_active {
                state.active += 1;
                drop(state);
                Start::Now(work())
            } else if let Some(reason) = signal.as_ref().and_then(AbortSignal::reason) {
                Start::Aborted(reason)
            } else {
                let (sender, receiver) = oneshot::channel();
                let id = state.next_id;
                state.next_id += 1;
                state.waiting.push_back((id, sender));
                Start::Wait(work, receiver, id)
            }
        };
        let state = Arc::clone(&self.state);
        async move {
            match start {
                Start::Aborted(reason) => Err(LimitError::Aborted(reason)),
                Start::Now(fut) => {
                    let _guard = SlotGuard { state };
                    fut.await.map_err(LimitError::Work)
                }
                Start::Wait(work, receiver, id) => {
                    let granted = match signal {
                        Some(mut signal) => {
                            tokio::select! {
                                biased;
                                granted = receiver => granted.is_ok(),
                                reason = signal.aborted() => {
                                    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                                    let before = st.waiting.len();
                                    st.waiting.retain(|(waiting_id, _)| *waiting_id != id);
                                    if st.waiting.len() < before {
                                        return Err(LimitError::Aborted(reason));
                                    }
                                    // The slot was handed over in the same turn.
                                    true
                                }
                            }
                        }
                        None => receiver.await.is_ok(),
                    };
                    if !granted {
                        return Err(LimitError::Aborted("limiter dropped".to_owned()));
                    }
                    let _guard = SlotGuard { state };
                    work().await.map_err(LimitError::Work)
                }
            }
        }
    }
}
