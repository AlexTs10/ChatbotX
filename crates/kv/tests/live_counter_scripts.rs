//! Port of `packages/redis/__tests__/integration/live-counter-scripts.test.ts`
//! (P2.7 spec 3.3, T53-T66). The TS file ran only against a real Redis
//! (`describe.skipIf(!redisUrl)`, baseline-skip); here it runs always on
//! `kv::DurableKv` (spec 2.8) over a migrated in-memory DB. `client.hset` /
//! `hget` / `hgetall` / `exists` / `hincrby` become the `DurableKv` hash
//! methods. Concurrency cases use `tokio::spawn` on a multi-thread runtime;
//! the single writer serializes the scripts.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chatbotx_db::Db;
use chatbotx_kv::{Admit, AdmitStatus, DurableKv, TestClock};

const NOW: i64 = 1_760_000_000_000;
const FIELD_ERROR: &str = "ERR live counter field is not an integer";
const ARGUMENT_ERROR: &str = "ERR live counter argument is not an integer";

async fn store() -> Arc<DurableKv> {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.migrate().await.unwrap();
    Arc::new(DurableKv::new(db, Arc::new(TestClock::new(NOW))))
}

fn unique_key() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!("test:live-counter:{}", N.fetch_add(1, Ordering::Relaxed))
}

fn admitted(value: i64) -> Admit {
    Admit {
        status: AdmitStatus::Admitted,
        value,
    }
}

fn refused(value: i64) -> Admit {
    Admit {
        status: AdmitStatus::Refused,
        value,
    }
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:41 "live counter scripts against Redis > admitWithinLimit admits below the limit"
#[tokio::test]
async fn admitwithinlimit_admits_below_the_limit() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "2")]).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit(&key, "mac", Some(3.0))
            .await
            .unwrap(),
        admitted(3)
    );
    assert_eq!(store.hget(&key, "mac").await.unwrap(), Some("3".to_owned()));
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:52 "live counter scripts against Redis > admitWithinLimit refuses at the limit without writing"
#[tokio::test]
async fn admitwithinlimit_refuses_at_the_limit_without_writing() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "3")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit(&key, "mac", Some(3.0))
            .await
            .unwrap(),
        refused(3)
    );
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:64 "live counter scripts against Redis > admitWithinLimit repairs a negative counter when admitting"
#[tokio::test]
async fn admitwithinlimit_repairs_a_negative_counter_when_admitting() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "-2")]).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit(&key, "mac", Some(1.0))
            .await
            .unwrap(),
        admitted(1)
    );
    assert_eq!(store.hget(&key, "mac").await.unwrap(), Some("1".to_owned()));
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:75 "live counter scripts against Redis > admitWithinLimit refuses a negative counter at zero without writing"
#[tokio::test]
async fn admitwithinlimit_refuses_a_negative_counter_at_zero_without() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "-2")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    assert_eq!(
        store
            .admit_within_limit(&key, "mac", Some(0.0))
            .await
            .unwrap(),
        refused(0)
    );
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:87 "live counter scripts against Redis > admitWithinLimit treats a null limit as unlimited"
#[tokio::test]
async fn admitwithinlimit_treats_a_null_limit_as_unlimited() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "3")]).await.unwrap();

    assert_eq!(
        store.admit_within_limit(&key, "mac", None).await.unwrap(),
        admitted(4)
    );
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:97 "live counter scripts against Redis > admitWithinLimit reports a missing field without writing"
#[tokio::test]
async fn admitwithinlimit_reports_a_missing_field_without_writing() {
    let store = store().await;
    let key = unique_key();

    assert_eq!(
        store
            .admit_within_limit(&key, "mac", Some(3.0))
            .await
            .unwrap(),
        Admit {
            status: AdmitStatus::Missing,
            value: 0
        }
    );
    assert!(!store.exists(&key).await.unwrap());
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:107 "live counter scripts against Redis > admitWithinLimit rejects a non-integer field without writing"
#[tokio::test]
async fn admitwithinlimit_rejects_a_non_integer_field_without_writing() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "1.5")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    let err = store
        .admit_within_limit(&key, "mac", Some(3.0))
        .await
        .unwrap_err();
    assert!(err.to_string().contains(FIELD_ERROR), "{err}");
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:118 "live counter scripts against Redis > admitWithinLimit rejects a fractional limit without writing"
#[tokio::test]
async fn admitwithinlimit_rejects_a_fractional_limit_without_writing() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "1")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    let err = store
        .admit_within_limit(&key, "mac", Some(1.5))
        .await
        .unwrap_err();
    assert!(err.to_string().contains(ARGUMENT_ERROR), "{err}");
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:129 "live counter scripts against Redis > admitWithinLimit admits exactly the available concurrent slots"
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitwithinlimit_admits_exactly_the_available_concurrent() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "0")]).await.unwrap();

    let tasks: Vec<_> = (0..50)
        .map(|_| {
            let store = Arc::clone(&store);
            let key = key.clone();
            tokio::spawn(async move { store.admit_within_limit(&key, "mac", Some(10.0)).await })
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap().unwrap());
    }

    assert_eq!(
        results
            .iter()
            .filter(|result| result.status == AdmitStatus::Admitted)
            .count(),
        10
    );
    assert_eq!(
        store.hget(&key, "mac").await.unwrap(),
        Some("10".to_owned())
    );
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:143 "live counter scripts against Redis > decrementFloor floors the result at zero"
#[tokio::test]
async fn decrementfloor_floors_the_result_at_zero() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "2")]).await.unwrap();

    assert_eq!(
        store.decrement_floor(&key, "mac", 5.0).await.unwrap(),
        Some(0)
    );
    assert_eq!(store.hget(&key, "mac").await.unwrap(), Some("0".to_owned()));
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:151 "live counter scripts against Redis > decrementFloor reports a missing field without writing"
#[tokio::test]
async fn decrementfloor_reports_a_missing_field_without_writing() {
    let store = store().await;
    let key = unique_key();

    assert_eq!(store.decrement_floor(&key, "mac", 1.0).await.unwrap(), None);
    // TS: HEXISTS key mac = 0.
    assert_eq!(store.hget(&key, "mac").await.unwrap(), None);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:158 "live counter scripts against Redis > decrementFloor does not lose concurrent increments"
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decrementfloor_does_not_lose_concurrent_increments() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "100")]).await.unwrap();

    // 50 x (decrement by 1 || increment by 1), all interleaved.
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let (s, k) = (Arc::clone(&store), key.clone());
        tasks.push(tokio::spawn(async move {
            s.decrement_floor(&k, "mac", 1.0).await.map(|_| ())
        }));
        let (s, k) = (Arc::clone(&store), key.clone());
        tasks.push(tokio::spawn(async move {
            s.hincrby(&k, "mac", 1).await.map(|_| ())
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    // The floor is never reached (the value stays >= 50), so a lost update
    // from a non-atomic read-then-write would show as a value other than 100.
    assert_eq!(
        store.hget(&key, "mac").await.unwrap(),
        Some("100".to_owned())
    );
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:184 "live counter scripts against Redis > decrementFloor rejects a non-integer field without writing"
#[tokio::test]
async fn decrementfloor_rejects_a_non_integer_field_without_writing() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "bad")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    let err = store.decrement_floor(&key, "mac", 1.0).await.unwrap_err();
    assert!(err.to_string().contains(FIELD_ERROR), "{err}");
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}

/// Port of: packages/redis/__tests__/integration/live-counter-scripts.test.ts:195 "live counter scripts against Redis > decrementFloor rejects a fractional count without writing"
#[tokio::test]
async fn decrementfloor_rejects_a_fractional_count_without_writing() {
    let store = store().await;
    let key = unique_key();
    store.hset(&key, &[("mac", "2")]).await.unwrap();
    let before = store.get_all_fields(&key).await.unwrap();

    let err = store.decrement_floor(&key, "mac", 1.5).await.unwrap_err();
    assert!(err.to_string().contains(ARGUMENT_ERROR), "{err}");
    assert_eq!(store.get_all_fields(&key).await.unwrap(), before);
}
