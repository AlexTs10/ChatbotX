//! Port of `apps/worker/__tests__/provider-rate-limiter.test.ts` (P2.7 spec
//! 3.3, T80-T81; K6). The mocked Redis `eval` becomes
//! `MemKv::claim_min_interval` behind `wait_for_heavy_provider_slot`; the
//! TS assertions on the `eval` arguments become assertions on the stored
//! slot. Sleeps run on paused tokio time; a counting wrapper keeps the
//! `TestClock` in step with tokio time and counts the claims (the TS
//! `toHaveBeenCalledTimes`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_kv::heavy_provider::{HeavyProviderSlot, SlotClaimer, wait_for_heavy_provider_slot};
use chatbotx_kv::{MemKv, TestClock};
use tokio::time::Instant;

const NOW: i64 = 1_760_000_000_000;
const KEY: &str = "heavy-provider-rate:workspace-1:openai";

/// Delegates to `MemKv`, counts calls, and moves the `TestClock` with the
/// (paused) tokio clock before every claim.
struct CountingClaimer {
    inner: MemKv,
    clock: Arc<TestClock>,
    start: Instant,
    calls: AtomicUsize,
}

impl CountingClaimer {
    fn new() -> CountingClaimer {
        let clock = Arc::new(TestClock::new(NOW));
        CountingClaimer {
            inner: MemKv::new(clock.clone()),
            clock,
            start: Instant::now(),
            calls: AtomicUsize::new(0),
        }
    }

    fn sync_clock(&self) {
        let elapsed = i64::try_from(self.start.elapsed().as_millis()).unwrap();
        self.clock.set(NOW + elapsed);
    }
}

impl SlotClaimer for CountingClaimer {
    fn claim_min_interval(&self, key: &str, interval_ms: u64) -> u64 {
        self.sync_clock();
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.claim_min_interval(key, interval_ms)
    }
}

/// Port of: apps/worker/__tests__/provider-rate-limiter.test.ts:20 "waitForHeavyProviderSlot > claims a workspace provider slot atomically"
#[tokio::test(start_paused = true)]
async fn claims_a_workspace_provider_slot_atomically() {
    let store = CountingClaimer::new();
    let started = Instant::now();

    wait_for_heavy_provider_slot(
        &store,
        HeavyProviderSlot {
            min_interval_ms: 250,
            provider: "openai",
            workspace_id: "workspace-1",
        },
    )
    .await;

    // Claimed at once, in one atomic step.
    assert_eq!(started.elapsed().as_millis(), 0);
    assert_eq!(store.calls.load(Ordering::SeqCst), 1);
    // TS: eval(script, 1, "heavy-provider-rate:workspace-1:openai", now, "250").
    assert_eq!(store.inner.get_number(KEY), Some((NOW + 250) as f64));
    assert_eq!(store.inner.pttl_ms(KEY), Some(500));
}

/// Port of: apps/worker/__tests__/provider-rate-limiter.test.ts:38 "waitForHeavyProviderSlot > waits and retries when another worker owns the slot"
#[tokio::test(start_paused = true)]
async fn waits_and_retries_when_another_worker_owns_the_slot() {
    let store = CountingClaimer::new();
    // Another worker owns the slot.
    assert_eq!(store.inner.claim_min_interval(KEY, 1), 0);
    let started = Instant::now();

    wait_for_heavy_provider_slot(
        &store,
        HeavyProviderSlot {
            min_interval_ms: 1,
            provider: "openai",
            workspace_id: "workspace-1",
        },
    )
    .await;

    assert_eq!(store.calls.load(Ordering::SeqCst), 2);
    assert!(started.elapsed().as_millis() >= 1);
}
