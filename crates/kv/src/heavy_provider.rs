//! Heavy provider min-interval limiter (K6, spec 2.5): port target of
//! `apps/worker/src/heavy/services/provider-rate-limiter.ts`.
//! Key `heavy-provider-rate:${workspaceId}:${provider}`; claim when
//! `now >= next`, else sleep the returned wait and retry.
//! (Spec 3.1 places the final module in `crates/core/src/limits/heavy_provider.rs`.)

use std::time::Duration;

use crate::mem::MemKv;

/// The slot store (the TS Redis `eval`).
pub trait SlotClaimer: Send + Sync {
    /// See [`MemKv::claim_min_interval`]: wait in ms, 0 = claimed.
    fn claim_min_interval(&self, key: &str, interval_ms: u64) -> u64;
}

impl SlotClaimer for MemKv {
    fn claim_min_interval(&self, key: &str, interval_ms: u64) -> u64 {
        MemKv::claim_min_interval(self, key, interval_ms)
    }
}

/// Input of [`wait_for_heavy_provider_slot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeavyProviderSlot<'a> {
    pub min_interval_ms: u64,
    pub provider: &'a str,
    pub workspace_id: &'a str,
}

/// `heavy-provider-rate:${workspaceId}:${provider}`.
pub fn heavy_provider_rate_key(workspace_id: &str, provider: &str) -> String {
    format!("heavy-provider-rate:{workspace_id}:{provider}")
}

/// `waitForHeavyProviderSlot`: claims, or sleeps (`tokio::time::sleep`) the
/// returned wait and tries again.
pub async fn wait_for_heavy_provider_slot(store: &dyn SlotClaimer, input: HeavyProviderSlot<'_>) {
    if input.min_interval_ms == 0 {
        return;
    }
    let key = heavy_provider_rate_key(input.workspace_id, input.provider);
    loop {
        let wait_ms = store.claim_min_interval(&key, input.min_interval_ms);
        if wait_ms == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(wait_ms.min(input.min_interval_ms))).await;
    }
}
