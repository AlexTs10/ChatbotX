//! Port of `packages/redis/__tests__/cache-utils.test.ts` (P2.7 spec 3.3,
//! T1-T7). The TS in-memory `distributedStore` mock becomes the real
//! `kv::Cache` with a `TestClock`; the assertions on `put`/`sadd`/`expire`
//! arguments become assertions on the cache state (TTL, tag membership).
//! T2 is not applicable (user decision 2026-11-12, DECISIONS P2.7).

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_kv::{Cache, CacheOpts, TestClock};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const T0: i64 = 1_000_000;

fn cache() -> (Cache, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(T0));
    (Cache::new(clock.clone()), clock)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Workspace {
    id: String,
    name: String,
    created_at: DateTime<Utc>,
    scheduled_deletion_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: String,
}

fn item(id: &str) -> Item {
    Item { id: id.to_owned() }
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:46 "withCache > preserves Date instances across the cache round-trip"
#[tokio::test]
async fn preserves_date_instances_across_the_cache_round_trip() {
    let (cache, _clock) = cache();
    let workspace = Workspace {
        id: "1".to_owned(),
        name: "Acme".to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
        scheduled_deletion_at: None,
    };

    let first = workspace.clone();
    let first_result = cache
        .get_or_load("workspaces:1", CacheOpts::default(), || async move {
            Ok::<_, Infallible>(Some(first))
        })
        .await
        .unwrap();
    assert_eq!(first_result.as_ref(), Some(&workspace));

    let source_calls = AtomicUsize::new(0);
    let cached_result: Option<Workspace> = cache
        .get_or_load("workspaces:1", CacheOpts::default(), || async {
            source_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(None)
        })
        .await
        .unwrap();
    assert_eq!(source_calls.load(Ordering::SeqCst), 0);
    let cached_result = cached_result.expect("cache hit");
    // The typed field comes back as a DateTime (TS: `toBeInstanceOf(Date)`).
    let created_at: DateTime<Utc> = cached_result.created_at;
    assert_eq!(created_at, workspace.created_at);
    assert_eq!(cached_result, workspace);
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:82 "withCache > registers tags against the prefixed cache key"
#[tokio::test]
async fn registers_tags_against_the_prefixed_cache_key() {
    let (cache, _clock) = cache();
    let opts = CacheOpts {
        dynamic_tags: Some(
            (|result: &Item| vec![format!("workspaces:{}", result.id)]) as fn(&Item) -> Vec<String>,
        ),
        ..CacheOpts::default()
    };
    cache
        .get_or_load("workspaces:1", opts, || async {
            Ok::<_, Infallible>(Some(item("1")))
        })
        .await
        .unwrap();

    // TS: SADD tags:workspaces:1 sj:workspaces:1. One namespace in Rust: no prefixes.
    assert_eq!(
        cache.keys_for_tag("workspaces:1"),
        vec!["workspaces:1".to_owned()]
    );
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:93 "withCache > invalidateCacheKeys removes entries stored by withCache"
#[tokio::test]
async fn invalidatecachekeys_removes_entries_stored_by_withcache() {
    let (cache, _clock) = cache();
    cache
        .get_or_load("workspaces:1", CacheOpts::default(), || async {
            Ok::<_, Infallible>(Some(item("1")))
        })
        .await
        .unwrap();
    // (The TS legacy unprefixed entry has no Rust equivalent: one namespace.)

    cache.invalidate_keys(&["workspaces:1"]);

    assert_eq!(cache.len(), 0);
    let source_calls = AtomicUsize::new(0);
    let result = cache
        .get_or_load("workspaces:1", CacheOpts::default(), || async {
            source_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(Some(item("2")))
        })
        .await
        .unwrap();
    assert_eq!(source_calls.load(Ordering::SeqCst), 1);
    assert_eq!(result, Some(item("2")));
}

fn gate_ttl(has_rule: &bool) -> Option<u64> {
    Some(if *has_rule { 60 } else { 10 })
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:107 "withCache > ttlFor overrides the write TTL per result (short negative, long positive)"
#[tokio::test]
async fn ttlfor_overrides_the_write_ttl_per_result_short_negative() {
    let (cache, _clock) = cache();

    let opts = CacheOpts {
        ttl_secs: 60,
        ttl_for: Some(gate_ttl as fn(&bool) -> Option<u64>),
        ..CacheOpts::default()
    };
    cache
        .get_or_load("gate:neg", opts, || async {
            Ok::<_, Infallible>(Some(false))
        })
        .await
        .unwrap();
    assert_eq!(cache.ttl_secs("gate:neg"), Some(10));

    let opts = CacheOpts {
        ttl_secs: 60,
        ttl_for: Some(gate_ttl as fn(&bool) -> Option<u64>),
        ..CacheOpts::default()
    };
    cache
        .get_or_load("gate:pos", opts, || async {
            Ok::<_, Infallible>(Some(true))
        })
        .await
        .unwrap();
    assert_eq!(cache.ttl_secs("gate:pos"), Some(60));
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:131 "withCache > a short-lived negative entry never truncates the shared tag set's expiry"
#[tokio::test]
async fn a_short_lived_negative_entry_never_truncates_the_shared_tag() {
    let (cache, clock) = cache();

    // TS: EXPIRE tags:ws-1 60 (max of the entry TTL 10 and the option TTL 60).
    // Rust has no tag TTL: a tag lives as long as its entries. Same intent:
    // the short negative entry must not take the longer sibling out of the tag.
    let neg = CacheOpts {
        ttl_secs: 60,
        ttl_for: Some((|_: &bool| Some(10)) as fn(&bool) -> Option<u64>),
        tags: vec!["ws-1".to_owned()],
        ..CacheOpts::default()
    };
    cache
        .get_or_load("gate:neg", neg, || async {
            Ok::<_, Infallible>(Some(false))
        })
        .await
        .unwrap();
    let pos = CacheOpts {
        ttl_secs: 60,
        tags: vec!["ws-1".to_owned()],
        ..CacheOpts::default()
    };
    cache
        .get_or_load("gate:pos", pos, || async {
            Ok::<_, Infallible>(Some(true))
        })
        .await
        .unwrap();

    clock.advance(11_000);
    assert!(!cache.contains_key("gate:neg"));
    assert!(cache.contains_key("gate:pos"));

    cache.invalidate_tags(&["ws-1"]);

    assert!(!cache.contains_key("gate:pos"));
    let source_calls = AtomicUsize::new(0);
    let pos = CacheOpts {
        ttl_secs: 60,
        tags: vec!["ws-1".to_owned()],
        ..CacheOpts::default()
    };
    cache
        .get_or_load("gate:pos", pos, || async {
            source_calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(Some(true))
        })
        .await
        .unwrap();
    assert_eq!(source_calls.load(Ordering::SeqCst), 1);
}

/// Port of: packages/redis/__tests__/cache-utils.test.ts:141 "withCache > does not cache null or undefined results"
#[tokio::test]
async fn does_not_cache_null_or_undefined_results() {
    let (cache, _clock) = cache();
    let result: Option<Item> = cache
        .get_or_load("workspaces:missing", CacheOpts::default(), || async {
            Ok::<_, Infallible>(None)
        })
        .await
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(cache.len(), 0);
}
