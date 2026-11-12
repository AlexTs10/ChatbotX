//! Read-through cache (spec 2.4): replaces `withCache`,
//! `invalidateCacheKeys`, `invalidateCacheByTags` and
//! `BaseService.invalidateCacheTags`. Values are `serde_json` bytes of `T`;
//! a hit that does not deserialize is a miss. `None` results are not stored.
//! A tag membership goes away with its entry (expiry, eviction, invalidation).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::clock::Clock;
use crate::expiring::{ExpiringMap, Removed};
use crate::sweeper::SweepSignal;

/// Bounds (spec 2.4): 20 000 entries, 8 MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            max_entries: 20_000,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Default entry TTL of `withCache` (24 h).
pub const DEFAULT_CACHE_TTL_SECS: u64 = 86_400;

/// Options of one `get_or_load` call.
pub struct CacheOpts<T> {
    /// Default 86 400.
    pub ttl_secs: u64,
    /// Per-result TTL; `None` (or a `None` result) = `ttl_secs`.
    pub ttl_for: Option<fn(&T) -> Option<u64>>,
    pub tags: Vec<String>,
    pub dynamic_tags: Option<fn(&T) -> Vec<String>>,
}

impl<T> Default for CacheOpts<T> {
    /// `ttl_secs` 86 400, no `ttl_for`, no tags.
    fn default() -> Self {
        CacheOpts {
            ttl_secs: DEFAULT_CACHE_TTL_SECS,
            ttl_for: None,
            tags: Vec::new(),
            dynamic_tags: None,
        }
    }
}

struct CacheEntry {
    json: Box<str>,
    tags: Vec<String>,
}

struct CacheState {
    map: ExpiringMap<CacheEntry>,
    /// tag -> keys of stored entries.
    tags: HashMap<String, HashSet<String>>,
}

impl CacheState {
    /// Removes the tag memberships of removed entries.
    fn forget(&mut self, removed: Removed<CacheEntry>) {
        for (key, slot) in removed {
            for tag in slot.value.tags {
                if let Some(keys) = self.tags.get_mut(&tag) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        self.tags.remove(&tag);
                    }
                }
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(slot) = self.map.remove(key) {
            self.forget(vec![(key.to_owned(), slot)]);
        }
    }

    fn drop_if_expired(&mut self, key: &str, now: i64) {
        if let Some(slot) = self.map.remove_if_expired(key, now) {
            self.forget(vec![(key.to_owned(), slot)]);
        }
    }
}

pub struct Cache {
    clock: Arc<dyn Clock>,
    state: Mutex<CacheState>,
    signal: Arc<SweepSignal>,
}

impl Cache {
    pub fn new(clock: Arc<dyn Clock>) -> Cache {
        Cache::with_config(clock, CacheConfig::default())
    }

    pub fn with_config(clock: Arc<dyn Clock>, cfg: CacheConfig) -> Cache {
        Cache::build(clock, cfg, SweepSignal::new())
    }

    pub(crate) fn build(
        clock: Arc<dyn Clock>,
        cfg: CacheConfig,
        signal: Arc<SweepSignal>,
    ) -> Cache {
        Cache {
            clock,
            state: Mutex::new(CacheState {
                map: ExpiringMap::new("cache", cfg.max_entries, cfg.max_bytes),
                tags: HashMap::new(),
            }),
            signal,
        }
    }

    fn lock(&self) -> MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The cached value, when live and of type `T`.
    fn lookup<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        state.drop_if_expired(key, now);
        let slot = state.map.live(key, now)?;
        serde_json::from_str(&slot.value.json).ok()
    }

    /// Stores `value` (best effort: a value that does not serialize is not cached).
    fn store<T: Serialize>(&self, key: &str, value: &T, opts: &CacheOpts<T>) {
        let Ok(json) = serde_json::to_string(value) else {
            return;
        };
        let ttl_secs = opts.ttl_for.and_then(|f| f(value)).unwrap_or(opts.ttl_secs);
        let mut tags = opts.tags.clone();
        if let Some(dynamic) = opts.dynamic_tags {
            tags.extend(dynamic(value));
        }
        let now = self.clock.now_ms();
        let expires_at = (ttl_secs > 0).then(|| {
            now.saturating_add(i64::try_from(ttl_secs.saturating_mul(1000)).unwrap_or(i64::MAX))
        });
        let mut state = self.lock();
        // A rewrite keeps the old memberships too (TS `SADD` only adds).
        if let Some(old) = state.map.live(key, now) {
            tags.extend(old.value.tags.iter().cloned());
        }
        tags.sort();
        tags.dedup();
        let size = json.len() + tags.iter().map(String::len).sum::<usize>();
        let removed = state.map.insert(
            key,
            CacheEntry {
                json: json.into_boxed_str(),
                tags: tags.clone(),
            },
            size,
            expires_at,
            now,
        );
        state.forget(removed);
        for tag in tags {
            state.tags.entry(tag).or_default().insert(key.to_owned());
        }
        drop(state);
        if let Some(at) = expires_at {
            self.signal.note_expiry(at);
        }
    }

    /// `withCache(key, fn, opts)`. `load` errors propagate.
    pub async fn get_or_load<T, E, Fut>(
        &self,
        key: &str,
        opts: CacheOpts<T>,
        load: impl FnOnce() -> Fut,
    ) -> Result<Option<T>, E>
    where
        T: Serialize + DeserializeOwned,
        Fut: Future<Output = Result<Option<T>, E>>,
    {
        if let Some(hit) = self.lookup::<T>(key) {
            return Ok(Some(hit));
        }
        let Some(value) = load().await? else {
            return Ok(None);
        };
        self.store(key, &value, &opts);
        Ok(Some(value))
    }

    pub fn invalidate_keys(&self, keys: &[&str]) {
        let mut state = self.lock();
        for key in keys {
            state.remove(key);
        }
    }

    /// Removes exactly the live entries that carry one of `tags`.
    pub fn invalidate_tags(&self, tags: &[&str]) {
        let mut state = self.lock();
        for tag in tags {
            let Some(keys) = state.tags.remove(*tag) else {
                continue;
            };
            for key in keys {
                state.remove(&key);
            }
        }
    }

    /// Removes expired entries (and their tag memberships); returns how many.
    pub fn sweep_expired(&self) -> usize {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let removed = state.map.sweep(now);
        let count = removed.len();
        state.forget(removed);
        count
    }

    pub fn used_bytes(&self) -> usize {
        self.lock().map.bytes()
    }

    /// The earliest stored expiry (for the sweeper).
    pub(crate) fn next_expiry(&self) -> Option<i64> {
        self.lock().map.next_expiry()
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Cache {
    /// Number of live entries.
    pub fn len(&self) -> usize {
        let now = self.clock.now_ms();
        self.lock().map.live_count(now)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let now = self.clock.now_ms();
        self.lock().map.live(key, now).is_some()
    }

    /// Remaining TTL of a live entry in whole seconds (Redis `TTL` rounding).
    pub fn ttl_secs(&self, key: &str) -> Option<u64> {
        let now = self.clock.now_ms();
        let at = self.lock().map.live(key, now)?.expires_at?;
        u64::try_from((at - now + 500) / 1000).ok()
    }

    /// Keys registered under `tag`, sorted.
    pub fn keys_for_tag(&self, tag: &str) -> Vec<String> {
        let now = self.clock.now_ms();
        let state = self.lock();
        let mut keys: Vec<String> = state
            .tags
            .get(tag)
            .map(|keys| {
                keys.iter()
                    .filter(|key| state.map.live(key, now).is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::clock::TestClock;

    fn opts(ttl_secs: u64, tags: &[&str]) -> CacheOpts<u32> {
        CacheOpts {
            ttl_secs,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            ..CacheOpts::default()
        }
    }

    async fn load(cache: &Cache, key: &str, value: u32, opts: CacheOpts<u32>) {
        cache
            .get_or_load(
                key,
                opts,
                || async move { Ok::<_, Infallible>(Some(value)) },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn expiry_eviction_and_invalidation_also_drop_tag_memberships() {
        let clock = Arc::new(TestClock::new(0));
        let cache = Cache::with_config(
            clock.clone(),
            CacheConfig {
                max_entries: 2,
                max_bytes: usize::MAX,
            },
        );
        load(&cache, "a", 1, opts(10, &["t"])).await;
        load(&cache, "b", 2, opts(60, &["t"])).await;
        // Eviction of "a" (soonest expiry) removes it from tag "t".
        load(&cache, "c", 3, opts(60, &["u"])).await;
        assert_eq!(cache.keys_for_tag("t"), ["b"]);
        // Expiry + sweep removes "b" and the empty tag.
        clock.advance(60_000);
        assert_eq!(cache.sweep_expired(), 2);
        assert!(cache.lock().tags.is_empty());
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn a_rewrite_keeps_old_tags_and_a_wrong_typed_hit_is_a_miss() {
        let clock = Arc::new(TestClock::new(0));
        let cache = Cache::new(clock.clone());
        load(&cache, "k", 1, opts(60, &["old"])).await;
        cache.invalidate_keys(&["k"]);
        assert!(cache.lock().tags.is_empty());

        load(&cache, "k", 1, opts(60, &["old"])).await;
        // A hit that does not deserialize into the asked type runs the source
        // and overwrites the entry.
        let fresh: Option<String> = cache
            .get_or_load(
                "k",
                CacheOpts::<String> {
                    tags: vec!["new".to_owned()],
                    ..CacheOpts::default()
                },
                || async { Ok::<_, Infallible>(Some("text".to_owned())) },
            )
            .await
            .unwrap();
        assert_eq!(fresh.as_deref(), Some("text"));
        assert_eq!(cache.keys_for_tag("old"), ["k"]);
        assert_eq!(cache.keys_for_tag("new"), ["k"]);
        // TTL 0 = no expiry (the TS `if (ttlInSeconds)` rule).
        load(&cache, "forever", 1, opts(0, &[])).await;
        assert_eq!(cache.ttl_secs("forever"), None);
        assert!(cache.contains_key("forever"));
    }
}
