//! Live leaderboards (spec 2.7, K43): replaces `leaderboardStoreFactory`
//! (ranking ZSET + stats hash + top cache, rebuild swap, memory report).
//! A ranking that is not built (`None`) replaces the `-inf` sentinel `_`.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::clock::Clock;
use crate::sweeper::SweepSignal;

const TIE_SCALE: i64 = 100_000_000;
const TIE_MAX: i64 = TIE_SCALE - 1;
/// `KEY_TTL_SECONDS`: 30 days, refreshed on every score write.
const KEY_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// `encodeRankValue(score, tieSeconds)` = `score * 100 000 000 + (99 999 999 - tieSeconds)`.
pub fn encode_rank_value(score: i64, tie_seconds: i64) -> i64 {
    score * TIE_SCALE + (TIE_MAX - tie_seconds)
}

/// `decodeRankValue(v)` = `floor(v / 1e8)` (the score).
pub fn decode_rank_value(value: i64) -> i64 {
    value.div_euclid(TIE_SCALE)
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

/// `tieSecondsSince(startsAt, at)`: whole seconds since the start, clamped to `0..=99 999 999`.
pub fn tie_seconds_since(starts_at_ms: i64, at_ms: i64) -> i64 {
    (at_ms.saturating_sub(starts_at_ms).div_euclid(1000)).clamp(0, TIE_MAX)
}

fn new_today_field(day_key: &str) -> String {
    format!("new:{day_key}")
}

/// glibc-like allocation size of a heap string of `len` bytes.
fn alloc_size(len: usize) -> usize {
    (len + 8).next_multiple_of(16).max(32)
}

/// A ranking (the `lb:<id>:rank` ZSET without its sentinel). Order: rank
/// value descending, then member descending (Redis `ZREVRANGE` tie order).
#[derive(Default)]
struct Ranking {
    by_member: HashMap<Box<str>, i64>,
    order: BTreeSet<(Reverse<i64>, Reverse<Box<str>>)>,
}

impl Ranking {
    /// `ZADD`: returns the old value.
    fn set(&mut self, member: &str, value: i64) -> Option<i64> {
        let old = self.by_member.insert(member.into(), value);
        if let Some(old) = old {
            self.order.remove(&(Reverse(old), Reverse(member.into())));
        }
        self.order.insert((Reverse(value), Reverse(member.into())));
        old
    }

    /// `ZREM`.
    fn remove(&mut self, member: &str) {
        if let Some(old) = self.by_member.remove(member) {
            self.order.remove(&(Reverse(old), Reverse(member.into())));
        }
    }

    fn get(&self, member: &str) -> Option<i64> {
        self.by_member.get(member).copied()
    }

    /// `ZREVRANK + 1` (O(n) walk, spec 4.10).
    fn rank_of(&self, member: &str) -> Option<u64> {
        let value = self.get(member)?;
        let before = self
            .order
            .range(..(Reverse(value), Reverse(Box::<str>::from(member))))
            .count();
        u64::try_from(before + 1).ok()
    }

    /// Estimate (spec 2.7): two string copies, one hash slot, one B-tree slot.
    fn memory(&self) -> usize {
        64 + self
            .by_member
            .keys()
            .map(|member| 2 * alloc_size(member.len()) + 41 + 38)
            .sum::<usize>()
    }
}

/// All keys of one leaderboard (`rank`, `stats`, `top`, `rank:rebuild`).
#[derive(Default)]
struct Board {
    /// `None` = not built (the missing-key state of Redis).
    rank: Option<Ranking>,
    rank_expires_at: i64,
    rebuild: Option<Ranking>,
    stats: Option<HashMap<String, i64>>,
    stats_expires_at: i64,
    top: Option<(String, Option<i64>)>,
}

impl Board {
    fn purge(&mut self, now: i64) -> usize {
        let mut removed = 0;
        if self.rank.is_some() && self.rank_expires_at <= now {
            self.rank = None;
            removed += 1;
        }
        if self.stats.is_some() && self.stats_expires_at <= now {
            self.stats = None;
            removed += 1;
        }
        if self
            .top
            .as_ref()
            .is_some_and(|(_, at)| at.is_some_and(|at| at <= now))
        {
            self.top = None;
            removed += 1;
        }
        removed
    }

    fn is_empty(&self) -> bool {
        self.rank.is_none() && self.rebuild.is_none() && self.stats.is_none() && self.top.is_none()
    }

    fn next_expiry(&self) -> Option<i64> {
        [
            self.rank.as_ref().map(|_| self.rank_expires_at),
            self.stats.as_ref().map(|_| self.stats_expires_at),
            self.top.as_ref().and_then(|(_, at)| *at),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn memory(&self) -> usize {
        let rank = self.rank.as_ref().map_or(0, Ranking::memory);
        let rebuild = self.rebuild.as_ref().map_or(0, Ranking::memory);
        let stats = self
            .stats
            .as_ref()
            .map_or(0, |s| s.keys().map(|k| k.len() + 48).sum::<usize>());
        let top = self.top.as_ref().map_or(0, |(v, _)| v.len() + 48);
        rank + rebuild + stats + top
    }
}

pub struct Leaderboards {
    clock: Arc<dyn Clock>,
    boards: Mutex<HashMap<String, Board>>,
    signal: Arc<SweepSignal>,
}

impl Leaderboards {
    pub fn new(clock: Arc<dyn Clock>) -> Leaderboards {
        Leaderboards::build(clock, SweepSignal::new())
    }

    pub(crate) fn build(clock: Arc<dyn Clock>, signal: Arc<SweepSignal>) -> Leaderboards {
        Leaderboards {
            clock,
            boards: Mutex::new(HashMap::new()),
            signal,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Board>> {
        self.boards.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` on the (purged) board; drops the board when it ends empty.
    fn with_board<R>(&self, leaderboard_id: &str, f: impl FnOnce(&mut Board, i64) -> R) -> R {
        let now = self.clock.now_ms();
        let mut boards = self.lock();
        let board = boards.entry(leaderboard_id.to_owned()).or_default();
        board.purge(now);
        let out = f(board, now);
        if board.is_empty() {
            boards.remove(leaderboard_id);
        }
        out
    }

    /// `false` (no write) when the ranking is not built. Also writes the
    /// rebuild set when one exists. Updates stats; refreshes the 30-day TTLs.
    pub fn set_score(&self, input: SetScore<'_>) -> bool {
        let value = encode_rank_value(input.score, input.tie_seconds);
        let written = self.with_board(input.leaderboard_id, |board, now| {
            if let Some(rebuild) = board.rebuild.as_mut() {
                rebuild.set(input.contact_id, value);
            }
            let rank = board.rank.as_mut()?;
            let old_score = rank
                .set(input.contact_id, value)
                .map_or(0, decode_rank_value);
            let stats = board.stats.get_or_insert_with(HashMap::new);
            *stats.entry("totalPoints".to_owned()).or_insert(0) += input.score - old_score;
            if input.is_new {
                *stats.entry("participants".to_owned()).or_insert(0) += 1;
                *stats.entry(new_today_field(input.day_key)).or_insert(0) += 1;
            }
            let expires_at = now.saturating_add(KEY_TTL_MS);
            board.rank_expires_at = expires_at;
            board.stats_expires_at = expires_at;
            Some(expires_at)
        });
        if let Some(at) = written {
            self.signal.note_expiry(at);
        }
        written.is_some()
    }

    /// Only when built.
    pub fn set_members(&self, leaderboard_id: &str, members: &[MemberValue]) {
        if members.is_empty() {
            return;
        }
        self.with_board(leaderboard_id, |board, _| {
            let Some(rank) = board.rank.as_mut() else {
                return;
            };
            for member in members {
                match member.value {
                    Some(value) => {
                        rank.set(&member.contact_id, value);
                    }
                    None => rank.remove(&member.contact_id),
                }
            }
        });
    }

    pub fn set_member(&self, input: SetMember<'_>) {
        let value = encode_rank_value(input.score, input.tie_seconds);
        self.with_board(input.leaderboard_id, |board, _| {
            if let Some(rank) = board.rank.as_mut() {
                rank.set(input.contact_id, value);
            }
        });
    }

    pub fn remove_member(&self, leaderboard_id: &str, contact_id: &str) {
        self.with_board(leaderboard_id, |board, _| {
            if let Some(rank) = board.rank.as_mut() {
                rank.remove(contact_id);
            }
        });
    }

    /// `None` when not built and `offset == 0`; else the page (descending,
    /// ties by member descending), `[]` when empty.
    pub fn top(&self, leaderboard_id: &str, limit: usize, offset: usize) -> Option<Vec<TopEntry>> {
        self.with_board(leaderboard_id, |board, _| match board.rank.as_ref() {
            None => (offset != 0).then(Vec::new),
            Some(rank) => Some(
                rank.order
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|(Reverse(value), Reverse(member))| TopEntry {
                        contact_id: member.to_string(),
                        score: decode_rank_value(*value),
                    })
                    .collect(),
            ),
        })
    }

    pub fn ranks_of(&self, leaderboard_id: &str, contact_ids: &[&str]) -> RanksOf {
        self.with_board(leaderboard_id, |board, _| {
            let rank = board.rank.as_ref();
            let ranks = contact_ids
                .iter()
                .map(|id| {
                    let entry = rank.and_then(|rank| {
                        Some(RankOf {
                            rank: rank.rank_of(id)?,
                            score: decode_rank_value(rank.get(id)?),
                        })
                    });
                    ((*id).to_owned(), entry)
                })
                .collect();
            RanksOf {
                built: rank.is_some(),
                ranks,
            }
        })
    }

    /// `stats[id] = None` when never built.
    pub fn stats_many(
        &self,
        leaderboard_ids: &[&str],
        day_key: &str,
    ) -> HashMap<String, Option<LeaderboardStats>> {
        let field = new_today_field(day_key);
        leaderboard_ids
            .iter()
            .map(|id| {
                let stats = self.with_board(id, |board, _| {
                    let stats = board.stats.as_ref().filter(|s| !s.is_empty())?;
                    let get = |name: &str| stats.get(name).copied().unwrap_or(0);
                    Some(LeaderboardStats {
                        participants: get("participants"),
                        total_points: get("totalPoints"),
                        new_today: get(&field),
                    })
                });
                ((*id).to_owned(), stats)
            })
            .collect()
    }

    /// Builds a new ranking from `pages`, swaps it in, replaces the stats,
    /// drops the top cache and any rebuild set.
    ///
    /// Each page is added under the lock and the lock is released between
    /// pages, so a `set_score` during the rebuild also lands in the new set
    /// (the TS rebuild key), as in Redis.
    pub fn replace_ranking<I>(
        &self,
        leaderboard_id: &str,
        pages: I,
        stats: LeaderboardStats,
        day_key: &str,
    ) where
        I: IntoIterator<Item = Vec<RankEntry>>,
    {
        self.with_board(leaderboard_id, |board, _| {
            board.rebuild = Some(Ranking::default());
        });
        for page in pages {
            if page.is_empty() {
                continue;
            }
            self.with_board(leaderboard_id, |board, _| {
                let rebuild = board.rebuild.get_or_insert_with(Ranking::default);
                for row in &page {
                    rebuild.set(&row.contact_id, row.value);
                }
            });
        }
        let expires_at = self.with_board(leaderboard_id, |board, now| {
            board.rank = Some(board.rebuild.take().unwrap_or_default());
            let expires_at = now.saturating_add(KEY_TTL_MS);
            board.rank_expires_at = expires_at;
            board.stats = Some(HashMap::from([
                ("participants".to_owned(), stats.participants),
                ("totalPoints".to_owned(), stats.total_points),
                (new_today_field(day_key), stats.new_today),
            ]));
            board.stats_expires_at = expires_at;
            board.top = None;
            expires_at
        });
        self.signal.note_expiry(expires_at);
    }

    pub fn read_top_cache(&self, leaderboard_id: &str) -> Option<String> {
        self.with_board(leaderboard_id, |board, _| {
            board.top.as_ref().map(|(value, _)| value.clone())
        })
    }

    pub fn write_top_cache(&self, leaderboard_id: &str, value: &str, ttl_secs: u64) {
        let expires_at = self.with_board(leaderboard_id, |board, now| {
            let expires_at = (ttl_secs > 0).then(|| {
                now.saturating_add(i64::try_from(ttl_secs.saturating_mul(1000)).unwrap_or(i64::MAX))
            });
            board.top = Some((value.to_owned(), expires_at));
            expires_at
        });
        if let Some(at) = expires_at {
            self.signal.note_expiry(at);
        }
    }

    /// Removes every part of one leaderboard.
    pub fn drop(&self, leaderboard_id: &str) {
        self.lock().remove(leaderboard_id);
    }

    /// Estimated bytes of one ranking (`MEMORY USAGE`); `None` when not built.
    pub fn memory_usage(&self, leaderboard_id: &str) -> Option<usize> {
        self.with_board(leaderboard_id, |board, _| {
            board.rank.as_ref().map(Ranking::memory)
        })
    }

    /// Estimated bytes of all boards (replaces `INFO memory`).
    pub fn used_memory(&self) -> usize {
        self.lock().values().map(Board::memory).sum()
    }

    /// Removes expired parts and empty boards; returns how many parts.
    pub(crate) fn sweep_expired(&self) -> usize {
        let now = self.clock.now_ms();
        let mut removed = 0;
        self.lock().retain(|_, board| {
            removed += board.purge(now);
            !board.is_empty()
        });
        removed
    }

    /// The earliest expiry of any part (for the sweeper).
    pub(crate) fn next_expiry(&self) -> Option<i64> {
        self.lock().values().filter_map(Board::next_expiry).min()
    }
}

/// Test-only inspection helpers (spec 2.7, 3.2).
#[cfg(any(test, feature = "test-util"))]
impl Leaderboards {
    /// Starts a rebuild set (the frozen test wrote `ZADD rebuild -inf _`).
    pub fn begin_rebuild(&self, leaderboard_id: &str) {
        self.with_board(leaderboard_id, |board, _| {
            board.rebuild.get_or_insert_with(Ranking::default);
        });
    }

    /// `ZSCORE rank member`.
    pub fn raw_value(&self, leaderboard_id: &str, contact_id: &str) -> Option<i64> {
        self.with_board(leaderboard_id, |board, _| {
            board.rank.as_ref()?.get(contact_id)
        })
    }

    /// `ZSCORE rebuild member`.
    pub fn rebuild_raw_value(&self, leaderboard_id: &str, contact_id: &str) -> Option<i64> {
        self.with_board(leaderboard_id, |board, _| {
            board.rebuild.as_ref()?.get(contact_id)
        })
    }

    /// `EXISTS rank` (built).
    pub fn rank_exists(&self, leaderboard_id: &str) -> bool {
        self.with_board(leaderboard_id, |board, _| board.rank.is_some())
    }

    /// `EXISTS rebuild`.
    pub fn has_rebuild(&self, leaderboard_id: &str) -> bool {
        self.with_board(leaderboard_id, |board, _| board.rebuild.is_some())
    }

    /// `TTL rank` in seconds; `None` when not built.
    pub fn rank_ttl_secs(&self, leaderboard_id: &str) -> Option<u64> {
        self.with_board(leaderboard_id, |board, now| {
            board.rank.as_ref()?;
            u64::try_from((board.rank_expires_at - now + 500) / 1000).ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn built(store: &Leaderboards, id: &str, rows: &[(&str, i64)]) {
        let page = rows
            .iter()
            .map(|(contact_id, value)| RankEntry {
                contact_id: (*contact_id).to_owned(),
                value: *value,
            })
            .collect();
        store.replace_ranking(
            id,
            vec![page],
            LeaderboardStats {
                participants: 0,
                total_points: 0,
                new_today: 0,
            },
            "d",
        );
    }

    #[test]
    fn equal_rank_values_order_by_member_descending_as_redis() {
        let store = Leaderboards::new(Arc::new(TestClock::new(0)));
        built(&store, "lb", &[("a", 5), ("c", 5), ("b", 5), ("z", 9)]);
        let ids: Vec<String> = store
            .top("lb", 10, 0)
            .unwrap()
            .into_iter()
            .map(|e| e.contact_id)
            .collect();
        assert_eq!(ids, ["z", "c", "b", "a"]);
        let ranks = store.ranks_of("lb", &["a", "c"]);
        assert_eq!(ranks.ranks["a"].map(|r| r.rank), Some(4));
        assert_eq!(ranks.ranks["c"].map(|r| r.rank), Some(2));
    }

    #[test]
    fn rankings_expire_after_30_days_and_set_score_refreshes_the_ttl() {
        let clock = Arc::new(TestClock::new(0));
        let store = Leaderboards::new(clock.clone());
        built(&store, "lb", &[]);
        clock.set(KEY_TTL_MS - 1);
        assert!(store.set_score(SetScore {
            leaderboard_id: "lb",
            contact_id: "1",
            score: 1,
            tie_seconds: 0,
            is_new: true,
            day_key: "d",
        }));
        clock.set(KEY_TTL_MS + 1);
        assert!(store.rank_exists("lb"), "refreshed by set_score");
        clock.set(2 * KEY_TTL_MS);
        assert!(!store.rank_exists("lb"));
        assert_eq!(store.top("lb", 10, 0), None);
        assert_eq!(store.sweep_expired(), 0, "lazy reads removed the board");
        assert_eq!(store.next_expiry(), None);
    }

    #[test]
    fn decode_floors_and_tie_seconds_clamp() {
        assert_eq!(decode_rank_value(encode_rank_value(0, TIE_MAX)), 0);
        assert_eq!(decode_rank_value(encode_rank_value(-1, 0)), -1);
        assert_eq!(tie_seconds_since(10_000, 5_000), 0);
        assert_eq!(tie_seconds_since(0, 12_999), 12);
        assert_eq!(tie_seconds_since(0, i64::MAX), TIE_MAX);
    }
}
