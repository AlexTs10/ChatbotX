//! In-process keyed lock (spec 2.2): replaces `distributedLock.runExclusive`
//! (Redlock with auto-extension), the P2.6 interim lock map and the 30 s NX
//! locks of K20/K21.
//!
//! A key's entry lives only while someone holds or waits for it, so an idle
//! map is empty (no sweeper, no idle memory). A held key never expires (the
//! Redlock auto-extension); `ttl_secs` is metadata. Not re-entrant. Never
//! call `acquire` inside a `db.write` closure (it runs on the single writer
//! thread).

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::OwnedMutexGuard;

/// Redlock `retryDelay`.
const RETRY_STEP_MS: u64 = 200;

/// `ttl_secs` is metadata only (no expiry while held). `retry_secs: None` = `ttl_secs`;
/// `Some(0)` = one try (fail fast). Window = `ceil(retry_secs * 1000 / 200) * 200` ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockOpts {
    pub ttl_secs: u64,
    pub retry_secs: Option<u64>,
}

impl LockOpts {
    /// `ceil(retry * 1000 / 200)`: the Redlock `retryAttempts`.
    fn retry_steps(self) -> u64 {
        self.retry_secs
            .unwrap_or(self.ttl_secs)
            .saturating_mul(1000)
            .div_ceil(RETRY_STEP_MS)
    }
}

type Slot = Arc<tokio::sync::Mutex<()>>;

struct Shared {
    map: Mutex<HashMap<String, Slot>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        self.map.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The slot of `key` (created on first use).
    fn slot(&self, key: &str) -> Slot {
        Arc::clone(self.lock().entry(key.to_owned()).or_default())
    }

    /// Drops the entry of `key` when nobody holds or waits for it (only the
    /// map has a reference left).
    fn release(&self, key: &str) {
        let mut map = self.lock();
        if map
            .get(key)
            .is_some_and(|slot| Arc::strong_count(slot) == 1)
        {
            map.remove(key);
        }
    }
}

/// Map of held keys. An idle map is empty.
pub struct LockMap {
    shared: Arc<Shared>,
}

impl Default for LockMap {
    fn default() -> Self {
        LockMap::new()
    }
}

/// A held key; released on drop.
pub struct LockGuard {
    key: String,
    guard: Option<OwnedMutexGuard<()>>,
    shared: Arc<Shared>,
}

impl LockGuard {
    /// The held key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockGuard").field("key", &self.key).finish()
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        drop(self.guard.take());
        self.shared.release(&self.key);
    }
}

/// Cleans the entry of a key when an acquire ends without a guard (failure
/// or a cancelled wait).
struct PendingAcquire<'a> {
    shared: &'a Shared,
    key: &'a str,
}

impl Drop for PendingAcquire<'_> {
    fn drop(&mut self) {
        self.shared.release(self.key);
    }
}

/// `LockAcquisitionError` of redlock-universal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisitionError {
    pub key: String,
    pub attempts: u32,
}

impl fmt::Display for LockAcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Failed to acquire lock \"{}\" after {} attempts",
            self.key, self.attempts
        )
    }
}

impl Error for LockAcquisitionError {}

/// Error of [`LockMap::run_exclusive`]: the lock was not acquired, or `f` failed.
#[derive(Debug)]
pub enum RunExclusiveError<E> {
    Lock(LockAcquisitionError),
    Inner(E),
}

impl<E: fmt::Display> fmt::Display for RunExclusiveError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunExclusiveError::Lock(err) => fmt::Display::fmt(err, f),
            RunExclusiveError::Inner(err) => fmt::Display::fmt(err, f),
        }
    }
}

impl<E: Error + 'static> Error for RunExclusiveError<E> {
    /// Both variants expose the wrapped error, so
    /// [`is_lock_acquisition_error`] finds a lock error of `f` too (the TS
    /// `fn` errors propagated unchanged; pass the key to tell them apart).
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            RunExclusiveError::Lock(err) => Some(err),
            RunExclusiveError::Inner(err) => Some(err),
        }
    }
}

impl LockMap {
    pub fn new() -> LockMap {
        LockMap {
            shared: Arc::new(Shared {
                map: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn guard(&self, key: &str, guard: OwnedMutexGuard<()>) -> LockGuard {
        LockGuard {
            key: key.to_owned(),
            guard: Some(guard),
            shared: Arc::clone(&self.shared),
        }
    }

    /// Waits up to the retry window for `key`.
    pub async fn acquire(
        &self,
        key: &str,
        opts: LockOpts,
    ) -> Result<LockGuard, LockAcquisitionError> {
        let steps = opts.retry_steps();
        if steps == 0 {
            return self.try_acquire(key);
        }
        // Declared before `slot`: dropped after it, so `release` sees the
        // final reference count.
        let _pending = PendingAcquire {
            shared: &self.shared,
            key,
        };
        let slot = self.shared.slot(key);
        let window = Duration::from_millis(steps.saturating_mul(RETRY_STEP_MS));
        match tokio::time::timeout(window, slot.lock_owned()).await {
            Ok(guard) => Ok(self.guard(key, guard)),
            Err(_) => Err(LockAcquisitionError {
                key: key.to_owned(),
                attempts: u32::try_from(steps.saturating_add(1)).unwrap_or(u32::MAX),
            }),
        }
    }

    /// One try (retry window 0).
    pub fn try_acquire(&self, key: &str) -> Result<LockGuard, LockAcquisitionError> {
        let _pending = PendingAcquire {
            shared: &self.shared,
            key,
        };
        let slot = self.shared.slot(key);
        match slot.try_lock_owned() {
            Ok(guard) => Ok(self.guard(key, guard)),
            Err(_) => Err(LockAcquisitionError {
                key: key.to_owned(),
                attempts: 1,
            }),
        }
    }

    /// `distributedLock.runExclusive(key, ttl, fn, retry)`.
    pub async fn run_exclusive<T, E, Fut>(
        &self,
        key: &str,
        opts: LockOpts,
        f: impl FnOnce() -> Fut,
    ) -> Result<T, RunExclusiveError<E>>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        let _guard = self
            .acquire(key, opts)
            .await
            .map_err(RunExclusiveError::Lock)?;
        f().await.map_err(RunExclusiveError::Inner)
    }

    /// K41 `distributedStore.exists(LOCK_KEY)`.
    pub fn is_held(&self, key: &str) -> bool {
        self.shared
            .lock()
            .get(key)
            .is_some_and(|slot| slot.try_lock().is_err())
    }

    /// Number of map entries (held or waited-for keys).
    pub fn len(&self) -> usize {
        self.shared.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `isLockAcquisitionError(error, key?)`: walks the `source()` chain for a
/// [`LockAcquisitionError`]; with `key`, only an error for that key matches.
/// The first lock error in the chain decides.
pub fn is_lock_acquisition_error(err: &(dyn Error + 'static), key: Option<&str>) -> bool {
    let mut current = Some(err);
    while let Some(e) = current {
        if let Some(lock) = e.downcast_ref::<LockAcquisitionError>() {
            return key.is_none_or(|key| lock.key == key);
        }
        current = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const WAIT: LockOpts = LockOpts {
        ttl_secs: 10,
        retry_secs: Some(1),
    };
    const FAIL_FAST: LockOpts = LockOpts {
        ttl_secs: 10,
        retry_secs: Some(0),
    };

    #[test]
    fn retry_steps_follow_redlock_retry_attempts() {
        let opts = |ttl_secs, retry_secs| LockOpts {
            ttl_secs,
            retry_secs,
        };
        assert_eq!(opts(10, None).retry_steps(), 50);
        assert_eq!(opts(10, Some(0)).retry_steps(), 0);
        assert_eq!(opts(30, Some(5)).retry_steps(), 25);
    }

    #[tokio::test(start_paused = true)]
    async fn the_map_is_empty_after_release_failure_and_cancelled_waits() {
        let locks = LockMap::new();
        let held = locks.acquire("k", WAIT).await.unwrap();
        assert!(locks.is_held("k"));
        assert_eq!(locks.len(), 1);

        // A fail-fast try and a timed-out wait leave only the holder's entry.
        let err = locks.try_acquire("k").unwrap_err();
        assert_eq!(err.attempts, 1);
        let err = locks.acquire("k", WAIT).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to acquire lock \"k\" after 6 attempts"
        );
        assert_eq!(locks.len(), 1);

        // A cancelled wait cleans up too.
        let waiting = tokio::time::timeout(Duration::from_millis(10), locks.acquire("k", WAIT));
        assert!(waiting.await.is_err());
        assert_eq!(locks.len(), 1);

        drop(held);
        assert!(locks.is_empty());
        assert!(!locks.is_held("k"));

        // Failed try on a free key cannot happen; a successful one cleans up.
        drop(locks.try_acquire("k").unwrap());
        assert!(locks.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_gets_the_key_when_the_holder_releases_and_no_entry_is_left() {
        let locks = Arc::new(LockMap::new());
        let held = locks.acquire("k", WAIT).await.unwrap();
        let waiter = {
            let locks = Arc::clone(&locks);
            tokio::spawn(async move { locks.acquire("k", WAIT).await.map(drop) })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        drop(held);
        waiter.await.unwrap().unwrap();
        assert!(locks.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn run_exclusive_serializes_and_keeps_a_held_key_past_its_ttl() {
        let locks = Arc::new(LockMap::new());
        let running = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (locks, running) = (Arc::clone(&locks), Arc::clone(&running));
            tasks.push(tokio::spawn(async move {
                let opts = LockOpts {
                    ttl_secs: 1,
                    retry_secs: Some(60),
                };
                locks
                    .run_exclusive("k", opts, || async {
                        assert_eq!(running.fetch_add(1, Ordering::SeqCst), 0);
                        // Longer than the TTL: no expiry while held.
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        running.fetch_sub(1, Ordering::SeqCst);
                        Ok::<_, std::io::Error>(())
                    })
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert!(locks.is_empty());

        let busy = locks.try_acquire("k").unwrap();
        let err = locks
            .run_exclusive("k", FAIL_FAST, || async { Ok::<_, std::io::Error>(()) })
            .await
            .unwrap_err();
        assert!(matches!(err, RunExclusiveError::Lock(_)));
        assert!(is_lock_acquisition_error(&err, Some("k")));
        drop(busy);
        let inner = locks
            .run_exclusive("k", FAIL_FAST, || async {
                Err::<(), _>(std::io::Error::other("boom"))
            })
            .await
            .unwrap_err();
        assert_eq!(inner.to_string(), "boom");
        assert!(!is_lock_acquisition_error(&inner, None));
        assert!(locks.is_empty());
    }
}
