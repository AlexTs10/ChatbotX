//! Port of `apps/worker/__tests__/moosend-rate-limit.test.ts` (P2.7 spec
//! 3.3, T76-T79; K5). The mocked Redis `eval` (INCR + PEXPIRE + PTTL) becomes
//! `MemKv::incr_with_window_ms` behind `acquire_moosend_subscribe_permit`,
//! with a `TestClock`. T78 is not applicable (user decision 2026-11-12,
//! DECISIONS P2.7).

use std::sync::Arc;

use chatbotx_kv::moosend::{
    MoosendRateLimitError, acquire_moosend_subscribe_permit, build_moosend_rate_limit_key,
};
use chatbotx_kv::{MemKv, TestClock};

const NOW: i64 = 1_760_000_000_000;

fn mem() -> MemKv {
    MemKv::new(Arc::new(TestClock::new(NOW)))
}

/// `^moosend:subscribe-quota:[a-f0-9]{64}$`
fn matches_key_pattern(key: &str) -> bool {
    key.strip_prefix("moosend:subscribe-quota:")
        .is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// Port of: apps/worker/__tests__/moosend-rate-limit.test.ts:32 "Moosend subscribe quota > permits ten parallel calls and blocks the eleventh"
#[test]
fn permits_ten_parallel_calls_and_blocks_the_eleventh() {
    let store = mem();

    let results: Vec<Result<(), MoosendRateLimitError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..10)
            .map(|_| scope.spawn(|| acquire_moosend_subscribe_permit(&store, "secret-api-key")))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(results.len(), 10);
    assert!(results.iter().all(Result::is_ok), "{results:?}");

    let err = acquire_moosend_subscribe_permit(&store, "secret-api-key").unwrap_err();
    assert_eq!(err.retry_after_seconds, 10);
}

/// Port of: apps/worker/__tests__/moosend-rate-limit.test.ts:47 "Moosend subscribe quota > uses only a SHA-256 API-key fingerprint in Redis"
#[test]
fn uses_only_a_sha_256_api_key_fingerprint_in_redis() {
    let key = build_moosend_rate_limit_key("secret-api-key");
    assert!(matches_key_pattern(&key), "{key}");
    assert!(!key.contains("secret-api-key"));

    let store = mem();
    acquire_moosend_subscribe_permit(&store, "secret-api-key").unwrap();

    // TS: the script arguments did not hold the secret; here: the stored keys.
    let keys = store.keys();
    assert_eq!(keys, vec![key]);
    assert!(keys.iter().all(|k| !k.contains("secret-api-key")));
}

/// Port of: apps/worker/__tests__/moosend-rate-limit.test.ts:62 "Moosend subscribe quota > returns a safe local rate-limit error"
#[test]
fn returns_a_safe_local_rate_limit_error() {
    let error = MoosendRateLimitError::new(3);

    assert!(!error.to_string().contains("secret"));
    assert_eq!(error.kind(), "local_rate_limited");
    assert_eq!(error.retry_after_seconds, 3);
}
