//! Port of `packages/redis/__tests__/distributed-store.test.ts` (P2.7 spec
//! 3.3, T21-T35). The ioredis mocks and the JS copies of the Lua scripts
//! become the real stores: `MemKv` (volatile) and `DurableKv` (hash fields,
//! live counters, K7 pacing) on a migrated in-memory DB, with a `TestClock`.
//! Assertions on command arguments become assertions on the stored state.
//! T26, T27 and T36 are not applicable (user decision 2026-11-12,
//! DECISIONS P2.7).

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_kv::{Admit, AdmitStatus, DurableKv, MemKv, TestClock};
use serde_json::{Value, json};

const T0: i64 = 1_000_000;

fn mem() -> (MemKv, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(T0));
    (MemKv::new(clock.clone()), clock)
}

async fn durable(clock: &Arc<TestClock>) -> DurableKv {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.migrate().await.unwrap();
    DurableKv::new(db, clock.clone())
}

fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:6 "distributedStoreFactory.exists > returns true only when Redis reports the key exists"
#[test]
fn returns_true_only_when_redis_reports_the_key_exists() {
    let (store, _clock) = mem();
    store.put("present", &json!(1), None).unwrap();

    assert!(store.exists("present"));
    assert!(!store.exists("missing"));
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:18 "distributedStoreFactory.merge > writes an explicit null field instead of silently skipping it"
#[tokio::test]
async fn writes_an_explicit_null_field_instead_of_silently_skipping() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;

    store
        .merge(
            "ctx:conv-1",
            &[
                ("summarizing", Some(Value::Bool(false))),
                ("startedAt", Some(Value::Null)),
            ],
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        store.get_all_fields("ctx:conv-1").await.unwrap(),
        fields(&[("summarizing", "false"), ("startedAt", "null")])
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:33 "distributedStoreFactory.merge > skips undefined fields — the 'don't touch this field' signal"
#[tokio::test]
async fn skips_undefined_fields_the_don_t_touch_this_field_signal() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;
    // TS-only `undefined` -> `None`. An existing `b` shows it is not touched.
    store.hset("ctx:conv-1", &[("b", "keep")]).await.unwrap();

    store
        .merge("ctx:conv-1", &[("a", Some(json!(1))), ("b", None)], None)
        .await
        .unwrap();

    assert_eq!(
        store.get_all_fields("ctx:conv-1").await.unwrap(),
        fields(&[("a", "1"), ("b", "keep")])
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:73 "distributedStoreFactory.incrWithWindow > increments a fresh key to 1 and sets its expiry exactly once"
#[test]
fn increments_a_fresh_key_to_1_and_sets_its_expiry_exactly_once() {
    let (store, _clock) = mem();

    assert_eq!(store.incr_with_window("rl:ws-1:0", 10), 1);
    assert_eq!(store.ttl_secs("rl:ws-1:0"), Some(10));
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:81 "distributedStoreFactory.incrWithWindow > subsequent increments in the same window do not re-set the expiry"
#[test]
fn subsequent_increments_in_the_same_window_do_not_re_set_the() {
    let (store, clock) = mem();

    store.incr_with_window("rl:ws-1:0", 10);
    clock.advance(3_000);
    assert_eq!(store.incr_with_window("rl:ws-1:0", 10), 2);
    clock.advance(3_000);
    assert_eq!(store.incr_with_window("rl:ws-1:0", 10), 3);

    // One expiry only: still the one set at t0 (t0 + 10 s).
    assert_eq!(store.ttl_secs("rl:ws-1:0"), Some(4));
    clock.set(T0 + 10_000);
    assert!(!store.exists("rl:ws-1:0"));
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:142 "distributedStoreFactory live counter scripts > maps arguments to strings and null limit to unlimited"
#[tokio::test]
async fn maps_arguments_to_strings_and_null_limit_to_unlimited() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;
    store.hset("quota:user-1", &[("mac", "3")]).await.unwrap();

    // String mapping has no Rust meaning; the behaviour: `None` = unlimited (-1).
    assert_eq!(
        store
            .admit_within_limit("quota:user-1", "mac", None)
            .await
            .unwrap(),
        Admit {
            status: AdmitStatus::Admitted,
            value: 4
        }
    );
    assert_eq!(
        store
            .decrement_floor("quota:user-1", "mac", 2.0)
            .await
            .unwrap(),
        Some(2)
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:156 "distributedStoreFactory live counter scripts > maps result code 1 to admitted"
#[tokio::test]
async fn maps_result_code_1_to_admitted() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;
    store.hset("quota:user-1", &[("mac", "6")]).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit("quota:user-1", "mac", Some(10.0))
            .await
            .unwrap(),
        Admit {
            status: AdmitStatus::Admitted,
            value: 7
        }
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:156 "distributedStoreFactory live counter scripts > maps result code 0 to refused"
#[tokio::test]
async fn maps_result_code_0_to_refused() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;
    store.hset("quota:user-1", &[("mac", "7")]).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit("quota:user-1", "mac", Some(7.0))
            .await
            .unwrap(),
        Admit {
            status: AdmitStatus::Refused,
            value: 7
        }
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:156 "distributedStoreFactory live counter scripts > maps result code -1 to missing"
#[tokio::test]
async fn maps_result_code_minus_1_to_missing() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;

    // TS mapped a mocked `[-1, 7]`; the real script returns value 0 for a
    // missing field (spec 3.3 T31).
    assert_eq!(
        store
            .admit_within_limit("quota:user-1", "mac", Some(10.0))
            .await
            .unwrap(),
        Admit {
            status: AdmitStatus::Missing,
            value: 0
        }
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:170 "distributedStoreFactory live counter scripts > maps a missing decrement result to null"
#[tokio::test]
async fn maps_a_missing_decrement_result_to_null() {
    let clock = Arc::new(TestClock::new(T0));
    let store = durable(&clock).await;

    assert_eq!(
        store
            .decrement_floor("quota:user-1", "mac", 1.0)
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:182 "distributedStoreFactory.setNumber > always writes via plain SET key val EX ttl (no NX)"
#[test]
fn always_writes_via_plain_set_key_val_ex_ttl_no_nx() {
    let (store, _clock) = mem();

    store.set_number("throttle:key", 1.0, 300);

    assert_eq!(store.get_number("throttle:key"), Some(1.0));
    assert_eq!(store.ttl_secs("throttle:key"), Some(300));
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:193 "distributedStoreFactory.setNumber > overwrites an existing value, unlike setNumberIfNotExists"
#[test]
fn overwrites_an_existing_value_unlike_setnumberifnotexists() {
    let (store, _clock) = mem();

    store.set_number("throttle:key", 1.0, 300);
    store.set_number("throttle:key", 2.0, 300);

    // Both writes landed (TS: `set` called twice) ...
    assert_eq!(store.get_number("throttle:key"), Some(2.0));
    // ... unlike the NX form.
    assert!(!store.set_number_if_not_exists("throttle:key", 3.0, 300));
    assert_eq!(store.get_number("throttle:key"), Some(2.0));
}

/// Port of: packages/redis/__tests__/distributed-store.test.ts:224 "distributedStoreFactory.reserveTimeWindow > a free timeline starts now; the next caller starts where it ends"
#[tokio::test]
async fn a_free_timeline_starts_now_the_next_caller_starts_where_it() {
    let clock = Arc::new(TestClock::new(1_000_000));

    // K7 uses the durable store.
    let store = durable(&clock).await;
    assert_eq!(
        store
            .reserve_time_window("pace:page-1", 3000.0)
            .await
            .unwrap(),
        1_000_000
    );
    assert_eq!(
        store
            .reserve_time_window("pace:page-1", 2000.0)
            .await
            .unwrap(),
        1_003_000
    );
    // Another key has its own timeline.
    assert_eq!(
        store
            .reserve_time_window("pace:page-2", 2000.0)
            .await
            .unwrap(),
        1_000_000
    );

    // Same rule on the volatile store.
    let mem = MemKv::new(clock.clone());
    assert_eq!(mem.reserve_time_window("pace:page-1", 3000.0), 1_000_000);
    assert_eq!(mem.reserve_time_window("pace:page-1", 2000.0), 1_003_000);
    assert_eq!(mem.reserve_time_window("pace:page-2", 2000.0), 1_000_000);
}
