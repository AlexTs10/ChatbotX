//! Port of `packages/redis/__tests__/integration/leaderboard-store.test.ts`
//! (P2.7 spec 3.3, T42-T52). The TS file ran only against a real Redis
//! (`describe.skipIf(!redisUrl)`, baseline-skip); here it runs always on the
//! in-memory `kv::Leaderboards`. Raw Redis reads (`ZSCORE`, `EXISTS`, `TTL`,
//! `ZADD rebuild -inf _`) become test-only helpers (spec 2.7).
#![allow(clippy::int_plus_one)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_kv::{
    LeaderboardStats, Leaderboards, MemberValue, RankEntry, RankOf, SetMember, SetScore, TestClock,
    TopEntry, decode_rank_value, encode_rank_value,
};

const MAX_SCORE: i64 = 89_999_999;
const DAY: &str = "2026-10-12";
const NOW: i64 = 1_760_000_000_000;

struct Row {
    contact_id: String,
    score: i64,
    tie: i64,
}

fn row(contact_id: &str, score: i64, tie: i64) -> Row {
    Row {
        contact_id: contact_id.to_owned(),
        score,
        tie,
    }
}

fn store() -> Leaderboards {
    Leaderboards::new(Arc::new(TestClock::new(NOW)))
}

fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!("test-{}", N.fetch_add(1, Ordering::Relaxed))
}

/// `newLeaderboard(rows)`: a built ranking from one page.
fn new_leaderboard(store: &Leaderboards, rows: &[Row]) -> String {
    let id = new_id();
    let page: Vec<RankEntry> = rows
        .iter()
        .map(|row| RankEntry {
            contact_id: row.contact_id.clone(),
            value: encode_rank_value(row.score, row.tie),
        })
        .collect();
    store.replace_ranking(
        &id,
        vec![page],
        LeaderboardStats {
            participants: rows.len() as i64,
            total_points: rows.iter().map(|row| row.score).sum(),
            new_today: 0,
        },
        DAY,
    );
    id
}

fn set(
    store: &Leaderboards,
    id: &str,
    contact_id: &str,
    score: i64,
    tie_seconds: i64,
    is_new: bool,
) -> bool {
    store.set_score(SetScore {
        leaderboard_id: id,
        contact_id,
        score,
        tie_seconds,
        is_new,
        day_key: DAY,
    })
}

fn top(entries: &[(&str, i64)]) -> Vec<TopEntry> {
    entries
        .iter()
        .map(|(contact_id, score)| TopEntry {
            contact_id: (*contact_id).to_owned(),
            score: *score,
        })
        .collect()
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:81 "leaderboard store against Redis > setScore is a no-op when the ranking is not built"
#[test]
fn setscore_is_a_no_op_when_the_ranking_is_not_built() {
    let store = store();
    let id = new_id();

    assert!(!set(&store, &id, "1", 10, 5, true));
    assert!(!store.rank_exists(&id));
    assert_eq!(store.top(&id, 10, 0), None);
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:89 "leaderboard store against Redis > an empty but built ranking is distinguishable from a missing one"
#[test]
fn an_empty_but_built_ranking_is_distinguishable_from_a() {
    let store = store();
    let id = new_leaderboard(&store, &[]);

    assert_eq!(store.top(&id, 10, 0), Some(vec![]));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:94 "leaderboard store against Redis > setScore writes the exact value, the last write wins"
#[test]
fn setscore_writes_the_exact_value_the_last_write_wins() {
    let store = store();
    let id = new_leaderboard(&store, &[]);

    set(&store, &id, "42", 100, 50, false);
    set(&store, &id, "42", 60, 80, false);

    assert_eq!(store.raw_value(&id, "42"), Some(encode_rank_value(60, 80)));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:102 "leaderboard store against Redis > ranks by score, then by who reached the score first"
#[test]
fn ranks_by_score_then_by_who_reached_the_score_first() {
    let store = store();
    let id = new_leaderboard(&store, &[]);
    set(&store, &id, "late", 620, 500, true);
    set(&store, &id, "early", 620, 100, true);
    set(&store, &id, "top", 900, 900, true);

    assert_eq!(
        store.top(&id, 10, 0),
        Some(top(&[("top", 900), ("early", 620), ("late", 620)]))
    );
    let ranks = store.ranks_of(&id, &["late", "missing"]);
    assert!(ranks.built);
    assert_eq!(
        ranks.ranks.get("late"),
        Some(&Some(RankOf {
            rank: 3,
            score: 620
        }))
    );
    assert_eq!(ranks.ranks.get("missing"), Some(&None));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:119 "leaderboard store against Redis > the max score stays an exact integer in the rank value"
#[test]
fn the_max_score_stays_an_exact_integer_in_the_rank_value() {
    let store = store();
    let id = new_leaderboard(&store, &[]);

    set(&store, &id, "b", MAX_SCORE, 3, false);

    let raw = store.raw_value(&id, "b").expect("ranked");
    // `Number.isSafeInteger(raw)`.
    assert!(raw <= (1_i64 << 53) - 1);
    assert_eq!(decode_rank_value(raw), MAX_SCORE);
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:126 "leaderboard store against Redis > stats track points and new participants"
#[test]
fn stats_track_points_and_new_participants() {
    let store = store();
    let id = new_leaderboard(&store, &[row("x", 5, 0)]);

    set(&store, &id, "y", 7, 1, true);
    set(&store, &id, "x", 8, 2, false);

    let stats = store.stats_many(&[id.as_str(), "never-built"], DAY);
    assert_eq!(
        stats.get(&id),
        Some(&Some(LeaderboardStats {
            participants: 2,
            total_points: 15,
            new_today: 1
        }))
    );
    assert_eq!(stats.get("never-built"), Some(&None));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:139 "leaderboard store against Redis > a write during a rebuild also lands in the rebuild key"
#[test]
fn a_write_during_a_rebuild_also_lands_in_the_rebuild_key() {
    let store = store();
    let id = new_leaderboard(&store, &[]);
    store.begin_rebuild(&id);

    set(&store, &id, "new", 40, 10, true);

    assert_eq!(
        store.rebuild_raw_value(&id, "new"),
        Some(encode_rank_value(40, 10))
    );
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:148 "leaderboard store against Redis > setMembers overwrites and removes members, only when built"
#[test]
fn setmembers_overwrites_and_removes_members_only_when_built() {
    let store = store();
    let id = new_leaderboard(&store, &[row("1", 30, 0), row("2", 20, 0)]);

    store.set_members(
        &id,
        &[
            MemberValue {
                contact_id: "1".to_owned(),
                value: Some(encode_rank_value(55, 4)),
            },
            MemberValue {
                contact_id: "2".to_owned(),
                value: None,
            },
        ],
    );
    assert_eq!(store.top(&id, 5, 0), Some(top(&[("1", 55)])));

    let missing = new_id();
    store.set_members(
        &missing,
        &[MemberValue {
            contact_id: "1".to_owned(),
            value: Some(1),
        }],
    );
    assert!(!store.rank_exists(&missing));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:167 "leaderboard store against Redis > replaceRanking swaps in the rebuilt set and pagination skips the sentinel"
#[test]
fn replaceranking_swaps_in_the_rebuilt_set_and_pagination() {
    let store = store();
    let id = new_leaderboard(&store, &[row("1", 30, 0), row("2", 20, 0), row("3", 10, 0)]);

    assert_eq!(store.top(&id, 2, 0), Some(top(&[("1", 30), ("2", 20)])));
    assert_eq!(store.top(&id, 5, 2), Some(top(&[("3", 10)])));
    assert!(!store.has_rebuild(&id));
    assert!(store.rank_ttl_secs(&id).is_some_and(|ttl| ttl > 0));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:184 "leaderboard store against Redis > ban removes a member and unban puts it back"
#[test]
fn ban_removes_a_member_and_unban_puts_it_back() {
    let store = store();
    let id = new_leaderboard(&store, &[row("1", 30, 0)]);

    store.remove_member(&id, "1");
    assert_eq!(store.top(&id, 5, 0), Some(vec![]));

    store.set_member(SetMember {
        leaderboard_id: &id,
        contact_id: "1",
        score: 30,
        tie_seconds: 0,
    });
    assert_eq!(store.top(&id, 5, 0), Some(top(&[("1", 30)])));
}

/// Port of: packages/redis/__tests__/integration/leaderboard-store.test.ts:199 "leaderboard store against Redis > memory per participant stays small"
#[test]
fn memory_per_participant_stays_small() {
    let store = store();
    let rows: Vec<Row> = (0..10_000_u64)
        .map(|index| Row {
            contact_id: (7_300_000_000_000_000_000_u64 + index).to_string(),
            score: (index % 5000) as i64,
            tie: index as i64,
        })
        .collect();
    let id = new_leaderboard(&store, &rows);

    // `MEMORY USAGE` becomes the Rust estimate of spec 2.7.
    let bytes = store.memory_usage(&id);
    assert!(bytes.is_some());
    assert!((bytes.unwrap_or(0) as f64) / (rows.len() as f64) < 200.0);
}
