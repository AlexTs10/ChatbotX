//! Volatile store with Redis-like methods (spec 2.3). One std mutex; no
//! `.await` while it is held, so every method is atomic (the Lua guarantee).
//! Expiry is lazy on every read; the shared sweeper only frees memory.

use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::cas::CasStore;
use crate::clock::Clock;
use crate::error::KvError;

/// Bounds (spec 2.3): 50 000 entries, 4 MiB (estimate = key + value + 96 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemKvConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for MemKvConfig {
    fn default() -> Self {
        todo!("P2.7 spec 2.3: 50 000 entries, 4 MiB")
    }
}

/// The volatile KV (`distributedStore` / `casStore` on the cache Redis).
pub struct MemKv {
    _state: (),
}

impl MemKv {
    /// A store with the default bounds.
    pub fn new(clock: Arc<dyn Clock>) -> MemKv {
        let _ = clock;
        todo!("P2.7 spec 2.3")
    }

    pub fn with_config(clock: Arc<dyn Clock>, cfg: MemKvConfig) -> MemKv {
        let _ = (clock, cfg);
        todo!("P2.7 spec 2.3")
    }

    /// `put(key, value, ttlInSeconds)`: ttl `None`/0 = no expiry.
    pub fn put<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// Malformed value -> `None`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let _ = key;
        todo!("P2.7 spec 2.3")
    }

    /// Get and delete in one step.
    pub fn take<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let _ = key;
        todo!("P2.7 spec 2.3")
    }

    pub fn exists(&self, key: &str) -> bool {
        let _ = key;
        todo!("P2.7 spec 2.3")
    }

    pub fn get_all<T: DeserializeOwned>(&self, keys: &[&str]) -> Vec<Option<T>> {
        let _ = keys;
        todo!("P2.7 spec 2.3")
    }

    pub fn delete(&self, keys: &[&str]) {
        let _ = keys;
        todo!("P2.7 spec 2.3")
    }

    pub fn put_many<T: Serialize>(
        &self,
        entries: &[(&str, T)],
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let _ = (entries, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// `setNumber`: plain `SET key n EX ttl` (always writes).
    pub fn set_number(&self, key: &str, value: f64, ttl_secs: u64) {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// `setNumberIfNotExists`: `SET key n EX ttl NX`; `true` when written.
    pub fn set_number_if_not_exists(&self, key: &str, value: f64, ttl_secs: u64) -> bool {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    pub fn get_number(&self, key: &str) -> Option<f64> {
        let _ = key;
        todo!("P2.7 spec 2.3")
    }

    /// `incrementCounter` (`INCR_IF_EXISTS_LUA`): delta 0 or missing key -> `None`
    /// (no create); `EXPIRE` on every call when ttl > 0.
    pub fn increment_counter(&self, key: &str, delta: i64, ttl_secs: Option<u64>) -> Option<i64> {
        let _ = (key, delta, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// `incrWithWindow` (`INCR_WITH_WINDOW_LUA`): TTL only when the result is 1.
    pub fn incr_with_window(&self, key: &str, ttl_secs: u64) -> i64 {
        let _ = (key, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// K5: INCR, PEXPIRE on the first increment; returns `(count, pttl_ms)`.
    pub fn incr_with_window_ms(&self, key: &str, window_ms: u64) -> (i64, i64) {
        let _ = (key, window_ms);
        todo!("P2.7 spec 2.3 / K5")
    }

    /// K6 min-interval slot: claims when `now >= next`; then the stored value
    /// is `now + interval_ms` (TTL `2 * interval_ms`). Returns the wait in ms
    /// (`min(next - now, interval_ms)`), 0 = claimed.
    pub fn claim_min_interval(&self, key: &str, interval_ms: u64) -> u64 {
        let _ = (key, interval_ms);
        todo!("P2.7 spec 2.3 / K6")
    }

    /// `reserveTimeWindow`: `start = max(now, until)`, `until = start + max(0, ceil(span))`,
    /// TTL `until - now + 60 000` ms. Returns `start`.
    pub fn reserve_time_window(&self, key: &str, span_ms: f64) -> i64 {
        let _ = (key, span_ms);
        todo!("P2.7 spec 2.3")
    }

    /// `EXPIRE key ttl`; `false` when the key is absent.
    pub fn expire(&self, key: &str, ttl_secs: u64) -> bool {
        let _ = (key, ttl_secs);
        todo!("P2.7 spec 2.3")
    }

    /// Removes expired entries; returns how many. Called by the sweeper.
    pub fn sweep_expired(&self) -> usize {
        todo!("P2.7 spec 2.3")
    }

    /// Estimated bytes (spec 2.3 estimate).
    pub fn used_bytes(&self) -> usize {
        todo!("P2.7 spec 2.3")
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl MemKv {
    /// Remaining TTL in whole seconds, rounded as Redis `TTL`;
    /// `None` when the key is absent or has no expiry.
    pub fn ttl_secs(&self, key: &str) -> Option<u64> {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Remaining TTL in ms; `None` when absent or no expiry.
    pub fn pttl_ms(&self, key: &str) -> Option<i64> {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Live (not expired) keys, any order.
    pub fn keys(&self) -> Vec<String> {
        todo!("P2.7 spec 3.2")
    }

    /// `true` when a live entry has this key.
    pub fn contains_key(&self, key: &str) -> bool {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Writes `raw` text as the value (for malformed-JSON cases).
    pub fn put_raw(&self, key: &str, raw: &str, ttl_ms: Option<u64>) {
        let _ = (key, raw, ttl_ms);
        todo!("P2.7 spec 3.2")
    }
}

impl CasStore for MemKv {
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, value, ttl_ms);
        todo!("P2.7 spec 2.10")
    }

    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, expected, next, ttl_ms);
        todo!("P2.7 spec 2.10")
    }

    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, expected);
        todo!("P2.7 spec 2.10")
    }
}
