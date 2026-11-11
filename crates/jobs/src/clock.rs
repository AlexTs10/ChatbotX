//! Time source. Production uses [`SystemClock`]; tests inject a manual clock
//! (and `tokio::time::pause()` for sleeps).

/// Unix time in milliseconds (`Date.now()`).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        todo!()
    }
}
