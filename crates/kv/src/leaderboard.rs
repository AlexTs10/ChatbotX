//! Live leaderboards (spec 2.7, K43): replaces `leaderboardStoreFactory`
//! (ranking ZSET + stats hash + top cache, rebuild swap, memory report).
//! A ranking that is not built (`None`) replaces the `-inf` sentinel `_`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::clock::Clock;

/// `encodeRankValue(score, tieSeconds)` = `score * 100 000 000 + (99 999 999 - tieSeconds)`.
pub fn encode_rank_value(score: i64, tie_seconds: i64) -> i64 {
    let _ = (score, tie_seconds);
    todo!("P2.7 spec 2.7")
}

/// `decodeRankValue(v)` = `floor(v / 1e8)` (the score).
pub fn decode_rank_value(value: i64) -> i64 {
    let _ = value;
    todo!("P2.7 spec 2.7")
}

/// One ranking row (`{ contactId, value }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankEntry {
    pub contact_id: String,
    pub value: i64,
}

/// `setMembers` row; `value: None` removes the member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberValue {
    pub contact_id: String,
    pub value: Option<i64>,
}

/// `{ contactId, score }` of `top`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopEntry {
    pub contact_id: String,
    pub score: i64,
}

/// `{ rank, score }` of `ranksOf` (1-based rank).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankOf {
    pub rank: u64,
    pub score: i64,
}

/// Result of `ranksOf`: `ranks[id] = None` when the member is not ranked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RanksOf {
    pub built: bool,
    pub ranks: HashMap<String, Option<RankOf>>,
}

/// `{ participants, totalPoints, newToday }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderboardStats {
    pub participants: i64,
    pub total_points: i64,
    pub new_today: i64,
}

/// `setScore` input (`SET_SCORE_LUA`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetScore<'a> {
    pub leaderboard_id: &'a str,
    pub contact_id: &'a str,
    pub score: i64,
    pub tie_seconds: i64,
    pub is_new: bool,
    pub day_key: &'a str,
}

/// `setMember` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetMember<'a> {
    pub leaderboard_id: &'a str,
    pub contact_id: &'a str,
    pub score: i64,
    pub tie_seconds: i64,
}

pub struct Leaderboards {
    _state: (),
}

impl Leaderboards {
    pub fn new(clock: Arc<dyn Clock>) -> Leaderboards {
        let _ = clock;
        todo!("P2.7 spec 2.7")
    }

    /// `false` (no write) when the ranking is not built. Also writes the
    /// rebuild set when one exists. Updates stats; refreshes the 30-day TTLs.
    pub fn set_score(&self, input: SetScore<'_>) -> bool {
        let _ = input;
        todo!("P2.7 spec 2.7")
    }

    /// Only when built.
    pub fn set_members(&self, leaderboard_id: &str, members: &[MemberValue]) {
        let _ = (leaderboard_id, members);
        todo!("P2.7 spec 2.7")
    }

    pub fn set_member(&self, input: SetMember<'_>) {
        let _ = input;
        todo!("P2.7 spec 2.7")
    }

    pub fn remove_member(&self, leaderboard_id: &str, contact_id: &str) {
        let _ = (leaderboard_id, contact_id);
        todo!("P2.7 spec 2.7")
    }

    /// `None` when not built and `offset == 0`; else the page (descending,
    /// ties by member descending), `[]` when empty.
    pub fn top(&self, leaderboard_id: &str, limit: usize, offset: usize) -> Option<Vec<TopEntry>> {
        let _ = (leaderboard_id, limit, offset);
        todo!("P2.7 spec 2.7")
    }

    pub fn ranks_of(&self, leaderboard_id: &str, contact_ids: &[&str]) -> RanksOf {
        let _ = (leaderboard_id, contact_ids);
        todo!("P2.7 spec 2.7")
    }

    /// `stats[id] = None` when never built.
    pub fn stats_many(
        &self,
        leaderboard_ids: &[&str],
        day_key: &str,
    ) -> HashMap<String, Option<LeaderboardStats>> {
        let _ = (leaderboard_ids, day_key);
        todo!("P2.7 spec 2.7")
    }

    /// Builds a new ranking from `pages`, swaps it in, replaces the stats,
    /// drops the top cache and any rebuild set.
    pub fn replace_ranking<I>(
        &self,
        leaderboard_id: &str,
        pages: I,
        stats: LeaderboardStats,
        day_key: &str,
    ) where
        I: IntoIterator<Item = Vec<RankEntry>>,
    {
        let _ = (leaderboard_id, pages, stats, day_key);
        todo!("P2.7 spec 2.7")
    }

    pub fn read_top_cache(&self, leaderboard_id: &str) -> Option<String> {
        let _ = leaderboard_id;
        todo!("P2.7 spec 2.7")
    }

    pub fn write_top_cache(&self, leaderboard_id: &str, value: &str, ttl_secs: u64) {
        let _ = (leaderboard_id, value, ttl_secs);
        todo!("P2.7 spec 2.7")
    }

    /// Removes every part of one leaderboard.
    pub fn drop(&self, leaderboard_id: &str) {
        let _ = leaderboard_id;
        todo!("P2.7 spec 2.7")
    }

    /// Estimated bytes of one ranking (`MEMORY USAGE`); `None` when not built.
    pub fn memory_usage(&self, leaderboard_id: &str) -> Option<usize> {
        let _ = leaderboard_id;
        todo!("P2.7 spec 2.7")
    }

    /// Estimated bytes of all boards (replaces `INFO memory`).
    pub fn used_memory(&self) -> usize {
        todo!("P2.7 spec 2.7")
    }
}

/// Test-only inspection helpers (spec 2.7, 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Leaderboards {
    /// Starts a rebuild set (the frozen test wrote `ZADD rebuild -inf _`).
    pub fn begin_rebuild(&self, leaderboard_id: &str) {
        let _ = leaderboard_id;
        todo!("P2.7 spec 2.7")
    }

    /// `ZSCORE rank member`.
    pub fn raw_value(&self, leaderboard_id: &str, contact_id: &str) -> Option<i64> {
        let _ = (leaderboard_id, contact_id);
        todo!("P2.7 spec 3.2")
    }

    /// `ZSCORE rebuild member`.
    pub fn rebuild_raw_value(&self, leaderboard_id: &str, contact_id: &str) -> Option<i64> {
        let _ = (leaderboard_id, contact_id);
        todo!("P2.7 spec 3.2")
    }

    /// `EXISTS rank` (built).
    pub fn rank_exists(&self, leaderboard_id: &str) -> bool {
        let _ = leaderboard_id;
        todo!("P2.7 spec 3.2")
    }

    /// `EXISTS rebuild`.
    pub fn has_rebuild(&self, leaderboard_id: &str) -> bool {
        let _ = leaderboard_id;
        todo!("P2.7 spec 3.2")
    }

    /// `TTL rank` in seconds; `None` when not built.
    pub fn rank_ttl_secs(&self, leaderboard_id: &str) -> Option<u64> {
        let _ = leaderboard_id;
        todo!("P2.7 spec 3.2")
    }
}
