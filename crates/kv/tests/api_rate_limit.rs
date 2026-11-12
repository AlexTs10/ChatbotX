//! Port of `apps/builder/__tests__/api-rate-limit.test.ts` (P2.7 spec 3.3,
//! T67-T75; K1). `createFakeStore` becomes a recording `RateCounter` fake
//! (it counts calls, as the TS `vi.fn` did). T73 is adapted to the primary
//! `MemKv` counter with a `TestClock` (the TS fallback map exists only for a
//! Redis outage). T71 and T72 are not applicable (user decision 2026-11-12,
//! DECISIONS P2.7).
//!
//! The TS calls ran under `Promise.all`; the Rust limiter is synchronous on
//! an in-memory counter, so the calls run one after another.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chatbotx_kv::MemKv;
use chatbotx_kv::TestClock;
use chatbotx_kv::rate_limit::{
    ApiRateLimitInput, RateCounter, RateLimited, assert_api_not_rate_limited, check_api_rate_limit,
};

const WINDOW_MS: i64 = 10_000;
const REQUEST_LIMIT: usize = 120;

/// `createFakeStore()`: mirrors `INCR_WITH_WINDOW_LUA`'s contract and
/// records the calls.
#[derive(Default)]
struct FakeStore {
    counters: Mutex<HashMap<String, i64>>,
    calls: AtomicUsize,
}

impl RateCounter for FakeStore {
    fn incr_with_window(&self, key: &str, _ttl_secs: u64) -> i64 {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut counters = self.counters.lock().unwrap();
        let next = counters.get(key).copied().unwrap_or(0) + 1;
        counters.insert(key.to_owned(), next);
        next
    }
}

fn input<'a>(
    scope: &'a str,
    key: &'a str,
    store: &'a dyn RateCounter,
    now_ms: i64,
) -> ApiRateLimitInput<'a> {
    ApiRateLimitInput {
        scope,
        key,
        store,
        now_ms,
        limit: None,
    }
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:43 "checkApiRateLimit > counts requests within the fixed window and reports retryAfter"
#[test]
fn counts_requests_within_the_fixed_window_and_reports() {
    let store = FakeStore::default();
    let now = 0;

    let result = check_api_rate_limit(input("channel-api-rate-limit", "inbox-1", &store, now));

    assert!(!result.limited);
    assert!(result.retry_after > 0);
    assert_eq!(store.calls.load(Ordering::SeqCst), 1);
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:57 "checkApiRateLimit > limits once the count exceeds REQUEST_LIMIT within the same window"
#[test]
fn limits_once_the_count_exceeds_request_limit_within_the_same() {
    let store = FakeStore::default();
    let now = 0;
    for _ in 0..REQUEST_LIMIT {
        check_api_rate_limit(input("channel-api-rate-limit", "inbox-2", &store, now));
    }

    let result = check_api_rate_limit(input("channel-api-rate-limit", "inbox-2", &store, now));

    assert!(result.limited);
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:80 "checkApiRateLimit > scope isolation: the same key under different scopes gets independent counters"
#[test]
fn scope_isolation_the_same_key_under_different_scopes_gets() {
    let store = FakeStore::default();
    let now = 0;
    for _ in 0..REQUEST_LIMIT {
        check_api_rate_limit(input("channel-api-rate-limit", "shared-key", &store, now));
    }

    let channel_result =
        check_api_rate_limit(input("channel-api-rate-limit", "shared-key", &store, now));
    let workspace_result = check_api_rate_limit(input(
        "workspace-token-rate-limit",
        "shared-key",
        &store,
        now,
    ));

    assert!(channel_result.limited);
    assert!(!workspace_result.limited);
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:110 "checkApiRateLimit > a new window resets the counter"
#[test]
fn a_new_window_resets_the_counter() {
    let store = FakeStore::default();
    for _ in 0..REQUEST_LIMIT {
        check_api_rate_limit(input("channel-api-rate-limit", "inbox-3", &store, 0));
    }

    let next_window_result = check_api_rate_limit(input(
        "channel-api-rate-limit",
        "inbox-3",
        &store,
        WINDOW_MS,
    ));

    assert!(!next_window_result.limited);
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:166 "checkApiRateLimit > expired fallback entries are evicted instead of accumulating forever"
#[test]
fn expired_fallback_entries_are_evicted_instead_of() {
    // Adapted: the requirement applies to the primary in-memory counter.
    let base: i64 = 1_760_000_000_000 + 86_400_000;
    let clock = Arc::new(TestClock::new(base));
    let mem = MemKv::new(clock.clone());

    check_api_rate_limit(input("channel-api-rate-limit", "evict-old", &mem, base));
    assert!(mem.keys().iter().any(|key| key.contains("evict-old")));

    // Two windows later the first entry is expired and gone.
    clock.set(base + 2 * WINDOW_MS);
    check_api_rate_limit(input(
        "channel-api-rate-limit",
        "evict-new",
        &mem,
        base + 2 * WINDOW_MS,
    ));
    assert!(!mem.keys().iter().any(|key| key.contains("evict-old")));
    assert!(mem.keys().iter().any(|key| key.contains("evict-new")));
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:217 "assertApiNotRateLimited > resolves without throwing when not limited"
#[test]
fn resolves_without_throwing_when_not_limited() {
    let store = FakeStore::default();

    assert_eq!(
        assert_api_not_rate_limited(input("channel-api-rate-limit", "inbox-ok", &store, 0), None),
        Ok(())
    );
}

/// Port of: apps/builder/__tests__/api-rate-limit.test.ts:228 "assertApiNotRateLimited > throws a tooManyRequests exception with a 429 status and retry header when limited"
#[test]
fn throws_a_toomanyrequests_exception_with_a_429_status_and() {
    let store = FakeStore::default();
    for _ in 0..REQUEST_LIMIT {
        check_api_rate_limit(input("channel-api-rate-limit", "inbox-limited", &store, 0));
    }

    let mut res_headers = HashMap::new();
    let err: RateLimited = assert_api_not_rate_limited(
        input("channel-api-rate-limit", "inbox-limited", &store, 0),
        Some(&mut res_headers),
    )
    .unwrap_err();

    assert_eq!(err.code, "tooManyRequests");
    assert_eq!(err.http_status, 429);
    assert_eq!(
        res_headers.get("Retry-After").map(String::as_str),
        Some("10")
    );
}
