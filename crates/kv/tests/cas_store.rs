//! Port of `packages/redis/__tests__/cas-store.test.ts` (P2.7 spec 3.3,
//! T8-T17). The ioredis fakes (`set`/`get`/`del` mocks and the JS copies of
//! `COMPARE_AND_SWAP_JSON_LUA` / `COMPARE_AND_DELETE_JSON_LUA`) become the
//! real stores. Every case runs on both `MemKv` and `DurableKv` (spec 3.2)
//! through one `CasStore` trait object, with one shared `TestClock`.
//! T16 is not applicable (user decision 2026-11-12, DECISIONS P2.7).

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_kv::{CasStore, DurableKv, MemKv, TestClock};
use serde_json::{Map, Value, json};

const T0: i64 = 1_000_000;

/// One store under test plus its test-only helpers.
enum Store {
    Mem(MemKv),
    Durable(DurableKv),
}

impl Store {
    fn name(&self) -> &'static str {
        match self {
            Store::Mem(_) => "MemKv",
            Store::Durable(_) => "DurableKv",
        }
    }

    fn cas(&self) -> &dyn CasStore {
        match self {
            Store::Mem(s) => s,
            Store::Durable(s) => s,
        }
    }

    async fn put_raw(&self, key: &str, raw: &str) {
        match self {
            Store::Mem(s) => s.put_raw(key, raw, None),
            Store::Durable(s) => s.put_raw(key, raw, None).await.unwrap(),
        }
    }

    async fn pttl_ms(&self, key: &str) -> Option<i64> {
        match self {
            Store::Mem(s) => s.pttl_ms(key),
            Store::Durable(s) => s.pttl_ms(key).await.unwrap(),
        }
    }
}

/// `MemKv` and a migrated in-memory `DurableKv`, both on `clock`.
async fn stores(clock: &Arc<TestClock>) -> Vec<Store> {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.migrate().await.unwrap();
    vec![
        Store::Mem(MemKv::new(clock.clone())),
        Store::Durable(DurableKv::new(db, clock.clone())),
    ]
}

fn obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:6 "casStoreFactory.setIfAbsent > issues SET key value PX ttlMs NX and reports success"
#[tokio::test]
async fn issues_set_key_value_px_ttlms_nx_and_reports_success() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        let value = json!({ "sdp": "v=0..." });
        let written = store
            .cas()
            .set_if_absent("voip:offer:w1", &value, 30_000)
            .await
            .unwrap();
        assert!(written, "{}", store.name());
        // TS asserted the command `SET key JSON PX 30000 NX`; here its effect.
        assert_eq!(
            store.cas().get_raw("voip:offer:w1").await.unwrap(),
            Some(serde_json::to_string(&value).unwrap()),
            "{}",
            store.name()
        );
        assert_eq!(
            store.pttl_ms("voip:offer:w1").await,
            Some(30_000),
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:23 "casStoreFactory.setIfAbsent > returns false when the key already exists (redelivery cannot overwrite/extend)"
#[tokio::test]
async fn returns_false_when_the_key_already_exists_redelivery_cannot() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        clock.set(T0);
        let first = json!({ "sdp": "v=0..." });
        assert!(
            store
                .cas()
                .set_if_absent("voip:offer:w1", &first, 30_000)
                .await
                .unwrap()
        );
        clock.advance(10_000);

        let redelivery = json!({ "sdp": "v=1..." });
        let written = store
            .cas()
            .set_if_absent("voip:offer:w1", &redelivery, 30_000)
            .await
            .unwrap();

        assert!(!written, "{}", store.name());
        // Cannot overwrite ...
        assert_eq!(
            store.cas().get_raw("voip:offer:w1").await.unwrap(),
            Some(serde_json::to_string(&first).unwrap()),
            "{}",
            store.name()
        );
        // ... nor extend.
        assert_eq!(
            store.pttl_ms("voip:offer:w1").await,
            Some(20_000),
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:34 "casStoreFactory.get / getJson / del > get returns the raw string, null when absent"
#[tokio::test]
async fn get_returns_the_raw_string_null_when_absent() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        store.put_raw("present", "raw-value").await;

        assert_eq!(
            store.cas().get_raw("present").await.unwrap(),
            Some("raw-value".to_owned()),
            "{}",
            store.name()
        );
        assert_eq!(
            store.cas().get_raw("missing").await.unwrap(),
            None,
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:44 "casStoreFactory.get / getJson / del > getJson parses valid JSON and returns null for absent or malformed values"
#[tokio::test]
async fn getjson_parses_valid_json_and_returns_null_for_absent_or() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        store
            .put_raw("valid", &json!({ "phase": "reserved" }).to_string())
            .await;
        store.put_raw("malformed", "{not-json").await;

        assert_eq!(
            store.cas().get_json("valid").await.unwrap(),
            Some(json!({ "phase": "reserved" })),
            "{}",
            store.name()
        );
        assert_eq!(
            store.cas().get_json("malformed").await.unwrap(),
            None,
            "{}",
            store.name()
        );
        assert_eq!(
            store.cas().get_json("missing").await.unwrap(),
            None,
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:60 "casStoreFactory.get / getJson / del > del removes the key"
#[tokio::test]
async fn del_removes_the_key() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        store
            .put_raw("voip:ctrl:w1", &json!({ "phase": "reserved" }).to_string())
            .await;

        store.cas().del("voip:ctrl:w1").await.unwrap();

        assert_eq!(
            store.cas().get_raw("voip:ctrl:w1").await.unwrap(),
            None,
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:115 "casStoreFactory.compareAndSwap > applies the swap when every expected field matches the current record"
#[tokio::test]
async fn applies_the_swap_when_every_expected_field_matches_the() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        store
            .put_raw(
                "voip:ctrl:w1",
                &json!({ "phase": "reserved", "fenceToken": "f1" }).to_string(),
            )
            .await;

        let expected = obj(json!({ "phase": "reserved", "fenceToken": "f1" }));
        let next = json!({ "phase": "answering", "fenceToken": "f1" });
        let applied = store
            .cas()
            .compare_and_swap("voip:ctrl:w1", Some(&expected), &next, 30_000)
            .await
            .unwrap();

        assert!(applied, "{}", store.name());
        assert_eq!(
            store.cas().get_json("voip:ctrl:w1").await.unwrap(),
            Some(json!({ "phase": "answering", "fenceToken": "f1" })),
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:137 "casStoreFactory.compareAndSwap > rejects the swap when any expected field is stale (lost race)"
#[tokio::test]
async fn rejects_the_swap_when_any_expected_field_is_stale_lost_race() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        // A concurrent terminate already moved the phase away from "answering".
        let terminated = json!({ "phase": "terminated", "fenceToken": "f1" });
        store
            .cas()
            .compare_and_swap("voip:ctrl:w1", None, &terminated, 30_000)
            .await
            .unwrap();

        let expected = obj(json!({ "phase": "answering", "fenceToken": "f1" }));
        let next = json!({ "phase": "accepted", "fenceToken": "f1" });
        let applied = store
            .cas()
            .compare_and_swap("voip:ctrl:w1", Some(&expected), &next, 30_000)
            .await
            .unwrap();

        assert!(!applied, "{}", store.name());
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:158 "casStoreFactory.compareAndSwap > expected: null requires the key to be absent (create-only swap)"
#[tokio::test]
async fn expected_null_requires_the_key_to_be_absent_create_only_swap() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        let next = json!({ "phase": "reserved" });
        assert!(
            store
                .cas()
                .compare_and_swap("voip:ctrl:w1", None, &next, 1000)
                .await
                .unwrap(),
            "{}",
            store.name()
        );
        assert!(
            !store
                .cas()
                .compare_and_swap("voip:ctrl:w1", None, &next, 1000)
                .await
                .unwrap(),
            "{}",
            store.name()
        );
    }
}

/// Port of: packages/redis/__tests__/cas-store.test.ts:186 "casStoreFactory.compareAndDelete > deletes only when every expected field matches the current record"
#[tokio::test]
async fn deletes_only_when_every_expected_field_matches_the_current() {
    let clock = Arc::new(TestClock::new(T0));
    for store in stores(&clock).await {
        store
            .put_raw(
                "api:idempotency:key",
                &json!({ "state": "inFlight", "id": "1" }).to_string(),
            )
            .await;

        let stale = obj(json!({ "state": "inFlight", "id": "stale" }));
        assert!(
            !store
                .cas()
                .compare_and_delete("api:idempotency:key", &stale)
                .await
                .unwrap(),
            "{}",
            store.name()
        );
        let matching = obj(json!({ "state": "inFlight", "id": "1" }));
        assert!(
            store
                .cas()
                .compare_and_delete("api:idempotency:key", &matching)
                .await
                .unwrap(),
            "{}",
            store.name()
        );
        assert_eq!(
            store.cas().get_raw("api:idempotency:key").await.unwrap(),
            None,
            "{}",
            store.name()
        );
    }
}
