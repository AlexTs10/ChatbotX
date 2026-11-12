//! Read-through cache (spec 2.4): replaces `withCache`,
//! `invalidateCacheKeys`, `invalidateCacheByTags` and
//! `BaseService.invalidateCacheTags`. Values are `serde_json` bytes of `T`;
//! a hit that does not deserialize is a miss. `None` results are not stored.
//! A tag membership goes away with its entry (expiry, eviction, invalidation).

use std::future::Future;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::clock::Clock;

/// Bounds (spec 2.4): 20 000 entries, 8 MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        todo!("P2.7 spec 2.4: 20 000 entries, 8 MiB")
    }
}

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
        todo!("P2.7 spec 2.4")
    }
}

pub struct Cache {
    _state: (),
}

impl Cache {
    pub fn new(clock: Arc<dyn Clock>) -> Cache {
        let _ = clock;
        todo!("P2.7 spec 2.4")
    }

    pub fn with_config(clock: Arc<dyn Clock>, cfg: CacheConfig) -> Cache {
        let _ = (clock, cfg);
        todo!("P2.7 spec 2.4")
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
        let _ = (key, opts, load);
        todo!("P2.7 spec 2.4")
    }

    pub fn invalidate_keys(&self, keys: &[&str]) {
        let _ = keys;
        todo!("P2.7 spec 2.4")
    }

    /// Removes exactly the live entries that carry one of `tags`.
    pub fn invalidate_tags(&self, tags: &[&str]) {
        let _ = tags;
        todo!("P2.7 spec 2.4")
    }

    /// Removes expired entries (and their tag memberships); returns how many.
    pub fn sweep_expired(&self) -> usize {
        todo!("P2.7 spec 2.4")
    }

    pub fn used_bytes(&self) -> usize {
        todo!("P2.7 spec 2.4")
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Cache {
    /// Number of live entries.
    pub fn len(&self) -> usize {
        todo!("P2.7 spec 3.2")
    }

    pub fn is_empty(&self) -> bool {
        todo!("P2.7 spec 3.2")
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Remaining TTL of a live entry in whole seconds (Redis `TTL` rounding).
    pub fn ttl_secs(&self, key: &str) -> Option<u64> {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Keys registered under `tag`, sorted.
    pub fn keys_for_tag(&self, tag: &str) -> Vec<String> {
        let _ = tag;
        todo!("P2.7 spec 3.2")
    }
}
