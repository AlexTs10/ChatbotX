//! Time source (moved from `crates/jobs/src/clock.rs`, spec 2.1).
//! Production uses [`SystemClock`]; tests use [`TestClock`] (manual ms)
//! and `tokio::time::pause()` for sleeps.

use std::sync::atomic::{AtomicI64, Ordering};

/// Unix time in milliseconds (`Date.now()`).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
        }
    }
}

/// A manual clock: time moves only by [`TestClock::set`] / [`TestClock::advance`].
#[derive(Debug, Default)]
pub struct TestClock {
    now_ms: AtomicI64,
}

impl TestClock {
    /// A clock that reads `now_ms` until it is moved.
    pub fn new(now_ms: i64) -> TestClock {
        TestClock {
            now_ms: AtomicI64::new(now_ms),
        }
    }

    /// Sets the time.
    pub fn set(&self, now_ms: i64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }

    /// Moves the time forward by `ms` (backward when negative).
    pub fn advance(&self, ms: i64) {
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}
