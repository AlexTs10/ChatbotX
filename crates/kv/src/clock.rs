//! Time source (moved from `crates/jobs/src/clock.rs`, spec 2.1).
//! Production uses [`SystemClock`]; tests use [`TestClock`] (manual ms)
//! and `tokio::time::pause()` for sleeps.

use std::sync::atomic::AtomicI64;

/// Unix time in milliseconds (`Date.now()`).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        todo!("P2.7 spec 2.1: same body as jobs::clock::SystemClock")
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
        let _ = now_ms;
        todo!("P2.7 spec 2.1")
    }

    /// Sets the time.
    pub fn set(&self, now_ms: i64) {
        let _ = (now_ms, &self.now_ms);
        todo!("P2.7 spec 2.1")
    }

    /// Moves the time forward by `ms` (backward when negative).
    pub fn advance(&self, ms: i64) {
        let _ = ms;
        todo!("P2.7 spec 2.1")
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        todo!("P2.7 spec 2.1")
    }
}
