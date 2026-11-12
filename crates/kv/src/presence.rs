//! Presence leases (spec 2.6, K42): replaces `presenceStore` (a ZSET scored
//! by expiry plus `PRESENCE_HEARTBEAT_MANY_LUA`).

use std::sync::Arc;

use crate::clock::Clock;

pub struct Presence {
    _state: (),
}

impl Presence {
    pub fn new(clock: Arc<dyn Clock>) -> Presence {
        let _ = clock;
        todo!("P2.7 spec 2.6")
    }

    /// Empty `members` -> `[]`, no work. Prunes members with `expires <= now`;
    /// then sets every member to `now + ttl_ms`. Returns the members that
    /// were not live before (input order).
    pub fn heartbeat_many(&self, key: &str, members: &[&str], ttl_ms: u64) -> Vec<String> {
        let _ = (key, members, ttl_ms);
        todo!("P2.7 spec 2.6")
    }

    /// Prunes `<= now`, then the live members by expiry descending (ties by
    /// member descending), at most `limit`.
    pub fn live_members(&self, key: &str, limit: usize) -> Vec<String> {
        let _ = (key, limit);
        todo!("P2.7 spec 2.6")
    }

    /// Removes expired members and empty keys; returns how many members.
    pub fn sweep_expired(&self) -> usize {
        todo!("P2.7 spec 2.6")
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Presence {
    /// The stored expiry (ms) of `member`, without pruning; `None` when not stored.
    pub fn expiry_of(&self, key: &str, member: &str) -> Option<i64> {
        let _ = (key, member);
        todo!("P2.7 spec 3.2")
    }

    /// Stored members of `key` (live or expired, no pruning).
    pub fn raw_len(&self, key: &str) -> usize {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// `true` when `key` has a stored member set.
    pub fn has_key(&self, key: &str) -> bool {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }
}
