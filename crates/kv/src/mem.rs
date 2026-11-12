//! Volatile store with Redis-like methods (spec 2.3). One std mutex; no
//! `.await` while it is held, so every method is atomic (the Lua guarantee).
//! Expiry is lazy on every read; the shared sweeper only frees memory.
//!
//! A TTL of 0 means "no expiry" everywhere (the TS `if (ttlInSeconds)` rule);
//! Redis would reject `PX 0` / `EX 0`, which no caller sends.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::cas::{CasStore, cjson_matches};
use crate::clock::Clock;
use crate::error::KvError;
use crate::expiring::ExpiringMap;
use crate::num::{js_number, js_number_string, lua_tonumber, redis_int};
use crate::sweeper::SweepSignal;

/// Bounds (spec 2.3): 50 000 entries, 4 MiB (estimate = key + value + 96 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemKvConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for MemKvConfig {
    fn default() -> Self {
        MemKvConfig {
            max_entries: 50_000,
            max_bytes: 4 * 1024 * 1024,
        }
    }
}

/// A stored value: text (JSON or raw) or an integer counter.
enum MemValue {
    Str(Box<str>),
    Int(i64),
}

impl MemValue {
    fn text(&self) -> Cow<'_, str> {
        match self {
            MemValue::Str(s) => Cow::Borrowed(&**s),
            MemValue::Int(n) => Cow::Owned(n.to_string()),
        }
    }

    fn size(&self) -> usize {
        match self {
            MemValue::Str(s) => s.len(),
            MemValue::Int(_) => 8,
        }
    }

    /// Redis `INCR`/`INCRBY` view: the integer, or `None` when the text is not one.
    fn as_int(&self) -> Option<i64> {
        match self {
            MemValue::Int(n) => Some(*n),
            MemValue::Str(s) => redis_int(s),
        }
    }

    /// Lua `tonumber(GET key)`.
    fn as_lua_number(&self) -> Option<f64> {
        match self {
            MemValue::Int(n) => Some(*n as f64),
            MemValue::Str(s) => lua_tonumber(s),
        }
    }

    /// `String(n)` as `SET` writes it.
    fn from_number(n: f64) -> MemValue {
        if n == n.trunc() && n.abs() < 9.0e15 {
            MemValue::Int(n as i64)
        } else {
            MemValue::Str(js_number_string(n).into_boxed_str())
        }
    }
}

/// `now + secs` (ms); `None` for no TTL or 0.
pub(crate) fn expiry_secs(now: i64, ttl_secs: Option<u64>) -> Option<i64> {
    match ttl_secs {
        Some(secs) if secs > 0 => Some(now.saturating_add(ms(secs.saturating_mul(1000)))),
        _ => None,
    }
}

/// `now + ms`; `None` for 0.
pub(crate) fn expiry_ms(now: i64, ttl_ms: u64) -> Option<i64> {
    (ttl_ms > 0).then(|| now.saturating_add(ms(ttl_ms)))
}

fn ms(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The volatile KV (`distributedStore` / `casStore` on the cache Redis).
pub struct MemKv {
    clock: Arc<dyn Clock>,
    state: Mutex<ExpiringMap<MemValue>>,
    signal: Arc<SweepSignal>,
}

impl MemKv {
    /// A store with the default bounds.
    pub fn new(clock: Arc<dyn Clock>) -> MemKv {
        MemKv::with_config(clock, MemKvConfig::default())
    }

    pub fn with_config(clock: Arc<dyn Clock>, cfg: MemKvConfig) -> MemKv {
        MemKv::build(clock, cfg, SweepSignal::new())
    }

    pub(crate) fn build(
        clock: Arc<dyn Clock>,
        cfg: MemKvConfig,
        signal: Arc<SweepSignal>,
    ) -> MemKv {
        MemKv {
            clock,
            state: Mutex::new(ExpiringMap::new("mem", cfg.max_entries, cfg.max_bytes)),
            signal,
        }
    }

    pub(crate) fn signal(&self) -> &Arc<SweepSignal> {
        &self.signal
    }

    fn lock(&self) -> MutexGuard<'_, ExpiringMap<MemValue>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Locks, drops `key` when it is expired, and returns the guard and `now`.
    fn lock_key(&self, key: &str) -> (MutexGuard<'_, ExpiringMap<MemValue>>, i64) {
        let now = self.clock.now_ms();
        let mut map = self.lock();
        map.remove_if_expired(key, now);
        (map, now)
    }

    fn store(
        &self,
        map: &mut ExpiringMap<MemValue>,
        key: &str,
        value: MemValue,
        expires_at: Option<i64>,
        now: i64,
    ) {
        let size = value.size();
        map.insert(key, value, size, expires_at, now);
        if let Some(at) = expires_at {
            self.signal.note_expiry(at);
        }
    }

    fn live_text(&self, key: &str) -> Option<String> {
        let (map, now) = self.lock_key(key);
        map.live(key, now)
            .map(|slot| slot.value.text().into_owned())
    }

    /// `put(key, value, ttlInSeconds)`: ttl `None`/0 = no expiry.
    pub fn put<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let text = serde_json::to_string(value)?;
        let (mut map, now) = self.lock_key(key);
        let expires_at = expiry_secs(now, ttl_secs);
        self.store(
            &mut map,
            key,
            MemValue::Str(text.into_boxed_str()),
            expires_at,
            now,
        );
        Ok(())
    }

    /// Malformed value -> `None`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let text = self.live_text(key)?;
        serde_json::from_str(&text).ok()
    }

    /// Get and delete in one step.
    pub fn take<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let text = {
            let (mut map, _) = self.lock_key(key);
            map.remove(key)?.value.text().into_owned()
        };
        serde_json::from_str(&text).ok()
    }

    pub fn exists(&self, key: &str) -> bool {
        let (map, now) = self.lock_key(key);
        map.live(key, now).is_some()
    }

    pub fn get_all<T: DeserializeOwned>(&self, keys: &[&str]) -> Vec<Option<T>> {
        keys.iter().map(|key| self.get(key)).collect()
    }

    pub fn delete(&self, keys: &[&str]) {
        let mut map = self.lock();
        for key in keys {
            map.remove(key);
        }
    }

    pub fn put_many<T: Serialize>(
        &self,
        entries: &[(&str, T)],
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        // Serialize everything first: a failure writes nothing (the TS
        // pipeline ran only after the loop).
        let texts = entries
            .iter()
            .map(|(key, value)| Ok((*key, serde_json::to_string(value)?)))
            .collect::<Result<Vec<_>, KvError>>()?;
        let now = self.clock.now_ms();
        let expires_at = expiry_secs(now, ttl_secs);
        let mut map = self.lock();
        for (key, text) in texts {
            self.store(
                &mut map,
                key,
                MemValue::Str(text.into_boxed_str()),
                expires_at,
                now,
            );
        }
        Ok(())
    }

    /// `setNumber`: plain `SET key n EX ttl` (always writes).
    pub fn set_number(&self, key: &str, value: f64, ttl_secs: u64) {
        let (mut map, now) = self.lock_key(key);
        let expires_at = expiry_secs(now, Some(ttl_secs));
        self.store(&mut map, key, MemValue::from_number(value), expires_at, now);
    }

    /// `setNumberIfNotExists`: `SET key n EX ttl NX`; `true` when written.
    pub fn set_number_if_not_exists(&self, key: &str, value: f64, ttl_secs: u64) -> bool {
        let (mut map, now) = self.lock_key(key);
        if map.live(key, now).is_some() {
            return false;
        }
        let expires_at = expiry_secs(now, Some(ttl_secs));
        self.store(&mut map, key, MemValue::from_number(value), expires_at, now);
        true
    }

    pub fn get_number(&self, key: &str) -> Option<f64> {
        let (map, now) = self.lock_key(key);
        match &map.live(key, now)?.value {
            MemValue::Int(n) => Some(*n as f64),
            MemValue::Str(s) => js_number(s),
        }
    }

    /// `incrementCounter` (`INCR_IF_EXISTS_LUA`): delta 0 or missing key -> `None`
    /// (no create); `EXPIRE` on every call when ttl > 0. A stored value that is
    /// not an integer (a Redis script error) or an overflow -> `None`, no write.
    pub fn increment_counter(&self, key: &str, delta: i64, ttl_secs: Option<u64>) -> Option<i64> {
        if delta == 0 {
            return None;
        }
        let (mut map, now) = self.lock_key(key);
        let slot = map.live(key, now)?;
        let next = slot.value.as_int()?.checked_add(delta)?;
        let expires_at = expiry_secs(now, ttl_secs).or(slot.expires_at);
        self.store(&mut map, key, MemValue::Int(next), expires_at, now);
        Some(next)
    }

    /// One `INCR`; the TTL is set (from `set_ttl`) only when the result is 1.
    /// A stored value that is not an integer counts as 0.
    fn incr_first_ttl(&self, key: &str, first_ttl_ms: u64) -> (i64, Option<i64>, i64) {
        let (mut map, now) = self.lock_key(key);
        let (current, kept) = match map.live(key, now) {
            Some(slot) => (slot.value.as_int().unwrap_or(0), slot.expires_at),
            None => (0, None),
        };
        let next = current.saturating_add(1);
        let expires_at = if next == 1 {
            expiry_ms(now, first_ttl_ms)
        } else {
            kept
        };
        self.store(&mut map, key, MemValue::Int(next), expires_at, now);
        (next, expires_at, now)
    }

    /// `incrWithWindow` (`INCR_WITH_WINDOW_LUA`): TTL only when the result is 1.
    pub fn incr_with_window(&self, key: &str, ttl_secs: u64) -> i64 {
        self.incr_first_ttl(key, ttl_secs.saturating_mul(1000)).0
    }

    /// K5: INCR, PEXPIRE on the first increment; returns `(count, pttl_ms)`
    /// (`pttl` -1 when the key has no expiry, as Redis `PTTL`).
    pub fn incr_with_window_ms(&self, key: &str, window_ms: u64) -> (i64, i64) {
        let (count, expires_at, now) = self.incr_first_ttl(key, window_ms);
        (count, expires_at.map_or(-1, |at| at - now))
    }

    /// K6 min-interval slot: claims when `now >= next`; then the stored value
    /// is `now + interval_ms` (TTL `2 * interval_ms`). Returns the wait in ms
    /// (`min(next - now, interval_ms)`), 0 = claimed.
    pub fn claim_min_interval(&self, key: &str, interval_ms: u64) -> u64 {
        if interval_ms == 0 {
            return 0;
        }
        let (mut map, now) = self.lock_key(key);
        let next = map
            .live(key, now)
            .and_then(|slot| slot.value.as_lua_number());
        match next {
            Some(next) if (now as f64) < next => {
                let wait = (next - now as f64).ceil();
                (wait as u64).min(interval_ms)
            }
            _ => {
                let interval = ms(interval_ms);
                let expires_at = expiry_ms(now, interval_ms.saturating_mul(2));
                self.store(
                    &mut map,
                    key,
                    MemValue::Int(now.saturating_add(interval)),
                    expires_at,
                    now,
                );
                0
            }
        }
    }

    /// `reserveTimeWindow`: `start = max(now, until)`, `until = start + max(0, ceil(span))`,
    /// TTL `until - now + 60 000` ms. Returns `start`.
    pub fn reserve_time_window(&self, key: &str, span_ms: f64) -> i64 {
        let span = reserve_span(span_ms);
        let (mut map, now) = self.lock_key(key);
        let reserved = map
            .live(key, now)
            .and_then(|slot| slot.value.as_lua_number())
            .unwrap_or(0.0);
        let (start, finish) = reserve(now, reserved, span);
        self.store(
            &mut map,
            key,
            MemValue::Int(finish),
            Some(finish.saturating_add(60_000)),
            now,
        );
        start
    }

    /// `EXPIRE key ttl`; `false` when the key is absent. `ttl` 0 deletes the
    /// key (Redis: a non-positive TTL expires it at once).
    pub fn expire(&self, key: &str, ttl_secs: u64) -> bool {
        let (mut map, now) = self.lock_key(key);
        if map.live(key, now).is_none() {
            return false;
        }
        match expiry_secs(now, Some(ttl_secs)) {
            None => {
                map.remove(key);
            }
            Some(at) => {
                map.set_expiry(key, Some(at));
                self.signal.note_expiry(at);
            }
        }
        true
    }

    /// Removes expired entries; returns how many. Called by the sweeper.
    pub fn sweep_expired(&self) -> usize {
        let now = self.clock.now_ms();
        self.lock().sweep(now).len()
    }

    /// Estimated bytes (spec 2.3 estimate).
    pub fn used_bytes(&self) -> usize {
        self.lock().bytes()
    }

    /// The earliest stored expiry (for the sweeper).
    pub(crate) fn next_expiry(&self) -> Option<i64> {
        self.lock().next_expiry()
    }

    /// Stored entries, expired ones not yet swept included.
    #[cfg(test)]
    pub(crate) fn stored_len(&self) -> usize {
        self.lock().stored_len()
    }
}

/// `Math.max(0, Math.ceil(spanMs))` as the TS caller computed it.
pub(crate) fn reserve_span(span_ms: f64) -> i64 {
    if span_ms.is_nan() {
        return 0;
    }
    span_ms.ceil().max(0.0).min(i64::MAX as f64) as i64
}

/// `RESERVE_TIME_WINDOW_LUA`: `(start, finish)`.
pub(crate) fn reserve(now: i64, reserved_until: f64, span: i64) -> (i64, i64) {
    let reserved = if reserved_until.is_finite() {
        reserved_until.ceil() as i64
    } else {
        0
    };
    let start = now.max(reserved);
    (start, start.saturating_add(span))
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl MemKv {
    /// Remaining TTL in whole seconds, rounded as Redis `TTL`;
    /// `None` when the key is absent or has no expiry.
    pub fn ttl_secs(&self, key: &str) -> Option<u64> {
        let pttl = self.pttl_ms(key)?;
        u64::try_from((pttl + 500) / 1000).ok()
    }

    /// Remaining TTL in ms; `None` when absent or no expiry.
    pub fn pttl_ms(&self, key: &str) -> Option<i64> {
        let (map, now) = self.lock_key(key);
        let at = map.live(key, now)?.expires_at?;
        Some(at - now)
    }

    /// Live (not expired) keys, any order.
    pub fn keys(&self) -> Vec<String> {
        let now = self.clock.now_ms();
        self.lock().live_keys(now).cloned().collect()
    }

    /// `true` when a live entry has this key.
    pub fn contains_key(&self, key: &str) -> bool {
        self.exists(key)
    }

    /// Writes `raw` text as the value (for malformed-JSON cases).
    pub fn put_raw(&self, key: &str, raw: &str, ttl_ms: Option<u64>) {
        let (mut map, now) = self.lock_key(key);
        let expires_at = ttl_ms.and_then(|ttl| expiry_ms(now, ttl));
        self.store(&mut map, key, MemValue::Str(raw.into()), expires_at, now);
    }
}

impl MemKv {
    fn cas_set_if_absent(&self, key: &str, value: &Value, ttl_ms: u64) -> bool {
        let text = value.to_string();
        let (mut map, now) = self.lock_key(key);
        if map.live(key, now).is_some() {
            return false;
        }
        let expires_at = expiry_ms(now, ttl_ms);
        self.store(
            &mut map,
            key,
            MemValue::Str(text.into_boxed_str()),
            expires_at,
            now,
        );
        true
    }

    fn cas_swap(
        &self,
        key: &str,
        expected: Option<&Map<String, Value>>,
        next: &Value,
        ttl_ms: u64,
    ) -> Result<bool, KvError> {
        let text = next.to_string();
        let (mut map, now) = self.lock_key(key);
        let current = map
            .live(key, now)
            .map(|slot| slot.value.text().into_owned());
        let matches = match (expected, current) {
            (None, current) => current.is_none(),
            (Some(_), None) => false,
            (Some(expected), Some(current)) => cjson_matches(key, &current, expected)?,
        };
        if matches {
            let expires_at = expiry_ms(now, ttl_ms);
            self.store(
                &mut map,
                key,
                MemValue::Str(text.into_boxed_str()),
                expires_at,
                now,
            );
        }
        Ok(matches)
    }

    fn cas_delete(&self, key: &str, expected: &Map<String, Value>) -> Result<bool, KvError> {
        let (mut map, now) = self.lock_key(key);
        let Some(current) = map
            .live(key, now)
            .map(|slot| slot.value.text().into_owned())
        else {
            return Ok(false);
        };
        let matches = cjson_matches(key, &current, expected)?;
        if matches {
            map.remove(key);
        }
        Ok(matches)
    }
}

impl CasStore for MemKv {
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async move { Ok(self.cas_set_if_absent(key, value, ttl_ms)) })
    }

    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>> {
        Box::pin(async move { Ok(self.live_text(key)) })
    }

    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>> {
        Box::pin(async move {
            Ok(self
                .live_text(key)
                .and_then(|text| serde_json::from_str(&text).ok()))
        })
    }

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>> {
        Box::pin(async move {
            self.delete(&[key]);
            Ok(())
        })
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async move { self.cas_swap(key, expected, next, ttl_ms) })
    }

    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async move { self.cas_delete(key, expected) })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::clock::TestClock;

    fn mem(cfg: MemKvConfig) -> (MemKv, Arc<TestClock>) {
        let clock = Arc::new(TestClock::new(1_000_000));
        (MemKv::with_config(clock.clone(), cfg), clock)
    }

    #[test]
    fn eviction_keeps_the_entry_bound_and_drops_soonest_expiring_first() {
        let (store, _clock) = mem(MemKvConfig {
            max_entries: 3,
            max_bytes: usize::MAX,
        });
        store.put("forever", &1, None).unwrap();
        store.put("long", &1, Some(600)).unwrap();
        store.put("short", &1, Some(10)).unwrap();
        store.put("new", &1, Some(300)).unwrap();
        let mut keys = store.keys();
        keys.sort();
        assert_eq!(keys, ["forever", "long", "new"]);
        store.put("newer", &1, None).unwrap();
        let mut keys = store.keys();
        keys.sort();
        // "new" (300 s) expires before "long" (600 s); entries without TTL go last.
        assert_eq!(keys, ["forever", "long", "newer"]);
        assert_eq!(store.stored_len(), 3);
    }

    #[test]
    fn the_byte_bound_holds_under_a_flood_of_window_counters() {
        let (store, clock) = mem(MemKvConfig {
            max_entries: usize::MAX,
            max_bytes: 64 * 1024,
        });
        for i in 0..10_000 {
            clock.advance(1);
            store.incr_with_window(&format!("rl:ip-{i}:0"), 10);
            assert!(store.used_bytes() <= 64 * 1024);
        }
        // The newest counters survive; the oldest were evicted.
        assert!(store.exists("rl:ip-9999:0"));
        assert!(!store.exists("rl:ip-0:0"));
    }

    #[test]
    fn expired_entries_read_as_absent_before_the_sweeper_runs() {
        let (store, clock) = mem(MemKvConfig::default());
        store.put("a", &json!({"x": 1}), Some(1)).unwrap();
        store.set_number("n", 5.0, 1);
        clock.advance(1_000);
        assert!(!store.exists("a"));
        assert_eq!(store.get::<Value>("a"), None);
        assert_eq!(store.get_number("n"), None);
        assert_eq!(store.increment_counter("n", 1, None), None);
        assert_eq!(store.sweep_expired(), 0, "lazy reads already removed them");
        assert_eq!(store.stored_len(), 0);
    }

    #[test]
    fn counters_follow_the_redis_scripts() {
        let (store, clock) = mem(MemKvConfig::default());
        // INCR_IF_EXISTS: no create, EXPIRE on every call.
        assert_eq!(store.increment_counter("c", 1, Some(10)), None);
        store.set_number("c", 1.0, 100);
        assert_eq!(store.increment_counter("c", 0, Some(10)), None);
        assert_eq!(store.increment_counter("c", 2, Some(10)), Some(3));
        assert_eq!(store.ttl_secs("c"), Some(10));
        assert_eq!(store.increment_counter("c", -1, None), Some(2));
        assert_eq!(store.ttl_secs("c"), Some(10), "no ttl keeps the expiry");
        store.put_raw("text", "abc", None);
        assert_eq!(store.increment_counter("text", 1, None), None);
        // incr_with_window_ms returns the remaining window.
        assert_eq!(store.incr_with_window_ms("m", 10_000), (1, 10_000));
        clock.advance(2_500);
        assert_eq!(store.incr_with_window_ms("m", 10_000), (2, 7_500));
        // claim_min_interval: claim, then wait the rest of the interval.
        assert_eq!(store.claim_min_interval("slot", 250), 0);
        clock.advance(100);
        assert_eq!(store.claim_min_interval("slot", 250), 150);
        clock.advance(150);
        assert_eq!(store.claim_min_interval("slot", 250), 0);
        // EXPIRE 0 deletes.
        assert!(store.expire("slot", 0));
        assert!(!store.exists("slot"));
        assert!(!store.expire("slot", 10));
    }
}
