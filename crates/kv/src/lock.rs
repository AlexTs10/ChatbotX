//! In-process keyed lock (spec 2.2): replaces `distributedLock.runExclusive`
//! (Redlock with auto-extension), the P2.6 interim lock map and the 30 s NX
//! locks of K20/K21.

use std::error::Error;
use std::fmt;
use std::future::Future;

/// `ttl_secs` is metadata only (no expiry while held). `retry_secs: None` = `ttl_secs`;
/// `Some(0)` = one try (fail fast). Window = `ceil(retry_secs * 1000 / 200) * 200` ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockOpts {
    pub ttl_secs: u64,
    pub retry_secs: Option<u64>,
}

/// Map of held keys. An idle map is empty.
pub struct LockMap {
    _state: (),
}

/// A held key; released on drop.
pub struct LockGuard {
    _state: (),
}

/// `LockAcquisitionError` of redlock-universal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisitionError {
    pub key: String,
    pub attempts: u32,
}

impl fmt::Display for LockAcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("P2.7 spec 2.2: `Failed to acquire lock \"<key>\" after <attempts> attempts`")
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
        let _ = f;
        todo!("P2.7 spec 2.2")
    }
}

impl<E: Error + 'static> Error for RunExclusiveError<E> {}

impl LockMap {
    pub fn new() -> LockMap {
        todo!("P2.7 spec 2.2")
    }

    /// Waits up to the retry window for `key`.
    pub async fn acquire(
        &self,
        key: &str,
        opts: LockOpts,
    ) -> Result<LockGuard, LockAcquisitionError> {
        let _ = (key, opts);
        todo!("P2.7 spec 2.2")
    }

    /// One try (retry window 0).
    pub fn try_acquire(&self, key: &str) -> Result<LockGuard, LockAcquisitionError> {
        let _ = key;
        todo!("P2.7 spec 2.2")
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
        let _ = (key, opts, f);
        todo!("P2.7 spec 2.2")
    }

    /// K41 `distributedStore.exists(LOCK_KEY)`.
    pub fn is_held(&self, key: &str) -> bool {
        let _ = key;
        todo!("P2.7 spec 2.2")
    }

    /// Number of map entries (held or waited-for keys).
    pub fn len(&self) -> usize {
        todo!("P2.7 spec 2.2")
    }

    pub fn is_empty(&self) -> bool {
        todo!("P2.7 spec 2.2")
    }
}

/// `isLockAcquisitionError(error, key?)`: walks the `source()` chain for a
/// [`LockAcquisitionError`]; with `key`, only an error for that key matches.
pub fn is_lock_acquisition_error(err: &(dyn Error + 'static), key: Option<&str>) -> bool {
    let _ = (err, key);
    todo!("P2.7 spec 2.2")
}
