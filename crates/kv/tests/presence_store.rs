//! Port of `packages/redis/__tests__/presence-store.test.ts` (P2.7 spec 3.3,
//! T37-T41). The JS copy of `PRESENCE_HEARTBEAT_MANY_LUA` and the ZSET mocks
//! become the real `kv::Presence`; `vi.spyOn(Date, "now")` becomes a
//! `TestClock`. "One Lua call" is the atomicity of one `heartbeat_many` call
//! under one lock; the assertions check the stored expiries instead.

use std::sync::Arc;

use chatbotx_kv::{Presence, TestClock};

const NOW: i64 = 1_000_000;
const KEY: &str = "workspace:presence:w1";

fn presence() -> (Presence, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(NOW));
    (Presence::new(clock.clone()), clock)
}

/// Port of: packages/redis/__tests__/presence-store.test.ts:63 "presenceStoreFactory.heartbeatMany > marks every member live via ONE presenceHeartbeatMany Lua call"
#[test]
fn marks_every_member_live_via_one_presenceheartbeatmany_lua() {
    let (store, _clock) = presence();

    let result = store.heartbeat_many(KEY, &["user-1", "user-2"], 20_000);

    assert_eq!(store.expiry_of(KEY, "user-1"), Some(NOW + 20_000));
    assert_eq!(store.expiry_of(KEY, "user-2"), Some(NOW + 20_000));
    assert_eq!(result, vec!["user-1".to_owned(), "user-2".to_owned()]);
}

/// Port of: packages/redis/__tests__/presence-store.test.ts:91 "presenceStoreFactory.heartbeatMany > returns only the members that were NOT already live (renewals excluded)"
#[test]
fn returns_only_the_members_that_were_not_already_live() {
    let (store, _clock) = presence();

    store.heartbeat_many(KEY, &["user-1"], 20_000);
    let result = store.heartbeat_many(KEY, &["user-1", "user-2"], 20_000);

    assert_eq!(result, vec!["user-2".to_owned()]);
}

/// Port of: packages/redis/__tests__/presence-store.test.ts:106 "presenceStoreFactory.heartbeatMany > a member whose previous lease expired counts as newly live again"
#[test]
fn a_member_whose_previous_lease_expired_counts_as_newly_live() {
    let (store, clock) = presence();

    clock.set(NOW);
    store.heartbeat_many(KEY, &["user-1"], 20_000);

    clock.set(NOW + 20_001);
    let result = store.heartbeat_many(KEY, &["user-1"], 20_000);

    assert_eq!(result, vec!["user-1".to_owned()]);
}

/// Port of: packages/redis/__tests__/presence-store.test.ts:123 "presenceStoreFactory.heartbeatMany > is a no-op (no Redis call) for an empty member list"
#[test]
fn is_a_no_op_no_redis_call_for_an_empty_member_list() {
    let (store, _clock) = presence();

    let result = store.heartbeat_many(KEY, &[], 20_000);

    assert!(result.is_empty());
    assert!(!store.has_key(KEY));
}

/// Port of: packages/redis/__tests__/presence-store.test.ts:139 "presenceStoreFactory.liveMembers > prunes expired members first, then returns the live set (recent first, capped)"
#[test]
fn prunes_expired_members_first_then_returns_the_live_set() {
    let (store, clock) = presence();
    // Two members whose leases ended before NOW (expiry NOW - 5 000 and NOW).
    clock.set(NOW - 15_000);
    store.heartbeat_many(KEY, &["gone-1"], 10_000);
    clock.set(NOW - 10_000);
    store.heartbeat_many(KEY, &["gone-2"], 10_000);
    // Two live members.
    clock.set(NOW);
    store.heartbeat_many(KEY, &["agent-1"], 5_000);
    store.heartbeat_many(KEY, &["agent-2"], 10_000);
    assert_eq!(store.raw_len(KEY), 4);

    assert_eq!(
        store.live_members(KEY, 10),
        vec!["agent-2".to_owned(), "agent-1".to_owned()]
    );

    // TS: ZREMRANGEBYSCORE key 0 now ran first.
    assert_eq!(store.raw_len(KEY), 2);
    assert_eq!(store.expiry_of(KEY, "gone-1"), None);
    assert_eq!(store.expiry_of(KEY, "gone-2"), None);
    // TS: LIMIT 0 <limit>.
    assert_eq!(store.live_members(KEY, 1), vec!["agent-2".to_owned()]);
}
