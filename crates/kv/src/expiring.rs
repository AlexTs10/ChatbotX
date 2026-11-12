//! A bounded map with lazy expiry, shared by `MemKv` and `Cache`
//! (spec 2.3, 2.4). Not thread-safe; the owner wraps it in one mutex.
//!
//! Bound rule on insert (spec 2.3): (1) drop expired entries from the front
//! of the expiry index; (2) evict the live entries that expire soonest;
//! (3) then entries without a TTL, oldest insertion first. A `warn` (at most
//! once per minute) when live entries are evicted.

use std::collections::{BTreeMap, HashMap};

/// Fixed per-entry estimate on top of key and value bytes (spec 2.3).
pub(crate) const ENTRY_OVERHEAD: usize = 96;
const WARN_INTERVAL_MS: i64 = 60_000;

pub(crate) struct Slot<V> {
    pub(crate) value: V,
    pub(crate) expires_at: Option<i64>,
    seq: u64,
    size: usize,
}

impl<V> Slot<V> {
    pub(crate) fn is_live(&self, now: i64) -> bool {
        self.expires_at.is_none_or(|at| at > now)
    }
}

/// Removed entries, returned so the owner can clean side indexes (tags).
pub(crate) type Removed<V> = Vec<(String, Slot<V>)>;

pub(crate) struct ExpiringMap<V> {
    name: &'static str,
    entries: HashMap<String, Slot<V>>,
    /// `(expires_at, seq) -> key` for entries with a TTL.
    expiry: BTreeMap<(i64, u64), String>,
    /// `seq -> key` for entries without a TTL (insertion order).
    no_ttl: BTreeMap<u64, String>,
    bytes: usize,
    next_seq: u64,
    max_entries: usize,
    max_bytes: usize,
    last_warn_ms: Option<i64>,
}

impl<V> ExpiringMap<V> {
    pub(crate) fn new(name: &'static str, max_entries: usize, max_bytes: usize) -> Self {
        ExpiringMap {
            name,
            entries: HashMap::new(),
            expiry: BTreeMap::new(),
            no_ttl: BTreeMap::new(),
            bytes: 0,
            next_seq: 0,
            max_entries: max_entries.max(1),
            max_bytes,
            last_warn_ms: None,
        }
    }

    /// Stored entries, expired ones not yet removed included.
    #[cfg(test)]
    pub(crate) fn stored_len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The entry under `key` when it is live.
    pub(crate) fn live(&self, key: &str, now: i64) -> Option<&Slot<V>> {
        self.entries.get(key).filter(|slot| slot.is_live(now))
    }

    /// Live keys, any order.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn live_keys(&self, now: i64) -> impl Iterator<Item = &String> {
        self.entries
            .iter()
            .filter(move |(_, slot)| slot.is_live(now))
            .map(|(key, _)| key)
    }

    /// Number of live entries.
    #[cfg(any(test, feature = "test-util"))]
    pub(crate) fn live_count(&self, now: i64) -> usize {
        self.entries
            .values()
            .filter(|slot| slot.is_live(now))
            .count()
    }

    /// Removes `key` when it is stored but expired (lazy expiry).
    pub(crate) fn remove_if_expired(&mut self, key: &str, now: i64) -> Option<Slot<V>> {
        if self.entries.get(key).is_some_and(|slot| !slot.is_live(now)) {
            self.remove(key)
        } else {
            None
        }
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<Slot<V>> {
        let slot = self.entries.remove(key)?;
        self.unindex(&slot);
        self.bytes = self.bytes.saturating_sub(slot.size);
        Some(slot)
    }

    fn unindex(&mut self, slot: &Slot<V>) {
        match slot.expires_at {
            Some(at) => {
                self.expiry.remove(&(at, slot.seq));
            }
            None => {
                self.no_ttl.remove(&slot.seq);
            }
        }
    }

    fn index(&mut self, key: &str, expires_at: Option<i64>, seq: u64) {
        match expires_at {
            Some(at) => {
                self.expiry.insert((at, seq), key.to_owned());
            }
            None => {
                self.no_ttl.insert(seq, key.to_owned());
            }
        }
    }

    /// Stores `value` under `key` (replacing any old entry) after making room
    /// by the bound rule. `value_size` is the value's byte estimate. Returns
    /// the replaced entry and every evicted entry.
    pub(crate) fn insert(
        &mut self,
        key: &str,
        value: V,
        value_size: usize,
        expires_at: Option<i64>,
        now: i64,
    ) -> Removed<V> {
        let mut removed = Vec::new();
        if let Some(old) = self.remove(key) {
            removed.push((key.to_owned(), old));
        }
        let size = key.len() + value_size + ENTRY_OVERHEAD;
        let mut evicted_live = 0usize;
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries || self.bytes + size > self.max_bytes)
        {
            let victim = if let Some(((at, _), victim)) = self.expiry.iter().next() {
                if *at > now {
                    evicted_live += 1;
                }
                victim.clone()
            } else if let Some((_, victim)) = self.no_ttl.iter().next() {
                evicted_live += 1;
                victim.clone()
            } else {
                break;
            };
            if let Some(slot) = self.remove(&victim) {
                removed.push((victim, slot));
            }
        }
        if evicted_live > 0
            && self
                .last_warn_ms
                .is_none_or(|last| now.saturating_sub(last) >= WARN_INTERVAL_MS)
        {
            self.last_warn_ms = Some(now);
            tracing::warn!(
                store = self.name,
                evicted = evicted_live,
                max_entries = self.max_entries,
                max_bytes = self.max_bytes,
                "kv store is over its bound: evicted live entries"
            );
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.index(key, expires_at, seq);
        self.entries.insert(
            key.to_owned(),
            Slot {
                value,
                expires_at,
                seq,
                size,
            },
        );
        self.bytes += size;
        removed
    }

    /// Changes the expiry of a stored entry; `false` when absent.
    pub(crate) fn set_expiry(&mut self, key: &str, expires_at: Option<i64>) -> bool {
        let Some(slot) = self.entries.get(key) else {
            return false;
        };
        let (old_at, seq) = (slot.expires_at, slot.seq);
        match old_at {
            Some(at) => {
                self.expiry.remove(&(at, seq));
            }
            None => {
                self.no_ttl.remove(&seq);
            }
        }
        self.index(key, expires_at, seq);
        if let Some(slot) = self.entries.get_mut(key) {
            slot.expires_at = expires_at;
        }
        true
    }

    /// Removes every expired entry.
    pub(crate) fn sweep(&mut self, now: i64) -> Removed<V> {
        let mut removed = Vec::new();
        while let Some(((at, _), key)) = self.expiry.iter().next() {
            if *at > now {
                break;
            }
            let key = key.clone();
            if let Some(slot) = self.remove(&key) {
                removed.push((key, slot));
            }
        }
        removed
    }

    /// The earliest stored expiry (expired entries not yet removed included).
    pub(crate) fn next_expiry(&self) -> Option<i64> {
        self.expiry.keys().next().map(|(at, _)| *at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys<V>(removed: &Removed<V>) -> Vec<&str> {
        removed.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn eviction_order_is_expired_then_soonest_expiring_then_oldest_without_ttl() {
        let now = 1_000;
        let mut map = ExpiringMap::new("test", 4, usize::MAX);
        map.insert("forever-old", (), 0, None, now);
        map.insert("forever-new", (), 0, None, now);
        map.insert("late", (), 0, Some(now + 5_000), now);
        map.insert("expired", (), 0, Some(now - 1), now - 10);

        // Full (4): the expired entry goes first.
        assert_eq!(keys(&map.insert("a", (), 0, None, now)), ["expired"]);
        // Then the live entry that expires soonest.
        assert_eq!(keys(&map.insert("b", (), 0, None, now)), ["late"]);
        // Then entries without a TTL, oldest insertion first.
        assert_eq!(keys(&map.insert("c", (), 0, None, now)), ["forever-old"]);
        assert_eq!(keys(&map.insert("d", (), 0, None, now)), ["forever-new"]);
        assert_eq!(map.stored_len(), 4);
        // Replacing an existing key evicts nothing else.
        assert_eq!(keys(&map.insert("d", (), 0, None, now)), ["d"]);
        assert_eq!(map.stored_len(), 4);
    }

    #[test]
    fn the_byte_bound_evicts_until_the_new_entry_fits() {
        let now = 0;
        let per_entry = 1 + 10 + ENTRY_OVERHEAD;
        let mut map = ExpiringMap::new("test", usize::MAX, per_entry * 3);
        for key in ["a", "b", "c"] {
            map.insert(key, (), 10, Some(100), now);
        }
        assert_eq!(map.bytes(), per_entry * 3);
        let removed = map.insert("d", (), 10 + per_entry, Some(100), now);
        assert_eq!(keys(&removed), ["a", "b"]);
        assert!(map.bytes() <= per_entry * 3);
        // An entry larger than the whole bound is still stored (alone).
        let removed = map.insert("huge", (), per_entry * 10, None, now);
        assert_eq!(removed.len(), 2);
        assert_eq!(map.stored_len(), 1);
    }

    #[test]
    fn sweep_removes_only_expired_entries_and_keeps_the_indexes_in_step() {
        let mut map = ExpiringMap::new("test", 100, usize::MAX);
        map.insert("a", (), 0, Some(10), 0);
        map.insert("b", (), 0, Some(20), 0);
        map.insert("c", (), 0, None, 0);
        assert_eq!(map.next_expiry(), Some(10));
        assert!(map.set_expiry("a", Some(30)));
        assert_eq!(map.next_expiry(), Some(20));
        assert_eq!(keys(&map.sweep(20)), ["b"]);
        assert_eq!(map.next_expiry(), Some(30));
        assert!(map.set_expiry("a", None));
        assert_eq!(map.next_expiry(), None);
        assert!(map.sweep(i64::MAX).is_empty());
        assert_eq!(map.stored_len(), 2);
        assert!(map.remove("a").is_some());
        assert!(map.remove("c").is_some());
        assert_eq!(map.bytes(), 0);
        assert!(map.expiry.is_empty() && map.no_ttl.is_empty());
    }
}
