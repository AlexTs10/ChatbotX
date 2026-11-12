//! Presence leases (spec 2.6, K42): replaces `presenceStore` (a ZSET scored
//! by expiry plus `PRESENCE_HEARTBEAT_MANY_LUA`).
//!
//! The Lua script pruned `score <= now` before it checked the members. Here a
//! heartbeat treats such members as absent without removing them (same
//! result, no O(n) scan per heartbeat); `live_members` and the sweeper
//! remove them.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::clock::Clock;
use crate::sweeper::SweepSignal;

type Members = HashMap<String, i64>;

pub struct Presence {
    clock: Arc<dyn Clock>,
    state: Mutex<HashMap<String, Members>>,
    signal: Arc<SweepSignal>,
}

impl Presence {
    pub fn new(clock: Arc<dyn Clock>) -> Presence {
        Presence::build(clock, SweepSignal::new())
    }

    pub(crate) fn build(clock: Arc<dyn Clock>, signal: Arc<SweepSignal>) -> Presence {
        Presence {
            clock,
            state: Mutex::new(HashMap::new()),
            signal,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Members>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Empty `members` -> `[]`, no work. Prunes members with `expires <= now`;
    /// then sets every member to `now + ttl_ms`. Returns the members that
    /// were not live before (input order).
    pub fn heartbeat_many(&self, key: &str, members: &[&str], ttl_ms: u64) -> Vec<String> {
        if members.is_empty() {
            return Vec::new();
        }
        let now = self.clock.now_ms();
        let expires_at = now.saturating_add(i64::try_from(ttl_ms).unwrap_or(i64::MAX));
        let mut newly_live = Vec::new();
        {
            let mut state = self.lock();
            let set = state.entry(key.to_owned()).or_default();
            let mut written: Vec<&str> = Vec::with_capacity(members.len());
            for member in members {
                // After the prune, the script's `ZSCORE` saw a live member or one
                // this call already wrote.
                let present =
                    written.contains(member) || set.get(*member).is_some_and(|at| *at > now);
                if !present {
                    newly_live.push((*member).to_owned());
                }
                set.insert((*member).to_owned(), expires_at);
                written.push(*member);
            }
        }
        self.signal.note_expiry(expires_at);
        newly_live
    }

    /// Prunes `<= now`, then the live members by expiry descending (ties by
    /// member descending), at most `limit`.
    pub fn live_members(&self, key: &str, limit: usize) -> Vec<String> {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let Some(set) = state.get_mut(key) else {
            return Vec::new();
        };
        set.retain(|_, at| *at > now);
        if set.is_empty() {
            state.remove(key);
            return Vec::new();
        }
        let mut live: Vec<(&String, i64)> = set.iter().map(|(m, at)| (m, *at)).collect();
        live.sort_by_key(|(member, at)| (Reverse(*at), Reverse(*member)));
        live.into_iter()
            .take(limit)
            .map(|(member, _)| member.clone())
            .collect()
    }

    /// Removes expired members and empty keys; returns how many members.
    pub fn sweep_expired(&self) -> usize {
        let now = self.clock.now_ms();
        let mut state = self.lock();
        let mut removed = 0;
        state.retain(|_, set| {
            let before = set.len();
            set.retain(|_, at| *at > now);
            removed += before - set.len();
            !set.is_empty()
        });
        removed
    }

    /// The earliest member expiry (for the sweeper).
    pub(crate) fn next_expiry(&self) -> Option<i64> {
        self.lock()
            .values()
            .flat_map(|set| set.values().copied())
            .min()
    }

    /// Estimated bytes (≈ member + 48 B per member).
    pub(crate) fn used_bytes(&self) -> usize {
        self.lock()
            .iter()
            .map(|(key, set)| key.len() + 64 + set.keys().map(|m| m.len() + 48).sum::<usize>())
            .sum()
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Presence {
    /// The stored expiry (ms) of `member`, without pruning; `None` when not stored.
    pub fn expiry_of(&self, key: &str, member: &str) -> Option<i64> {
        self.lock().get(key)?.get(member).copied()
    }

    /// Stored members of `key` (live or expired, no pruning).
    pub fn raw_len(&self, key: &str) -> usize {
        self.lock().get(key).map_or(0, HashMap::len)
    }

    /// `true` when `key` has a stored member set.
    pub fn has_key(&self, key: &str) -> bool {
        self.lock().contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    #[test]
    fn ties_order_by_member_descending_and_duplicates_count_once() {
        let clock = Arc::new(TestClock::new(1_000));
        let presence = Presence::new(clock.clone());
        let newly = presence.heartbeat_many("k", &["b", "a", "b", "c"], 10_000);
        assert_eq!(newly, ["b", "a", "c"]);
        // Equal expiries: Redis ZREVRANGEBYSCORE returns members descending.
        assert_eq!(presence.live_members("k", 10), ["c", "b", "a"]);
        clock.advance(10_000);
        assert!(presence.live_members("k", 10).is_empty());
        assert!(!presence.has_key("k"), "an empty set is removed");
    }

    #[test]
    fn the_sweeper_removes_expired_members_and_empty_keys() {
        let clock = Arc::new(TestClock::new(0));
        let presence = Presence::new(clock.clone());
        presence.heartbeat_many("k1", &["a"], 1_000);
        presence.heartbeat_many("k2", &["b"], 5_000);
        assert_eq!(presence.next_expiry(), Some(1_000));
        clock.set(1_000);
        assert_eq!(presence.sweep_expired(), 1);
        assert!(!presence.has_key("k1"));
        assert_eq!(presence.next_expiry(), Some(5_000));
    }
}
