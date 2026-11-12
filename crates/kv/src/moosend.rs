//! Moosend subscribe quota (K5, spec 2.5): port target of
//! `apps/worker/src/integration/handlers/moosend-rate-limit.ts`.
//! 10 calls per 10 000 ms from the first call, keyed by a SHA-256 API-key
//! fingerprint; `retryAfter = max(1, ceil(pttl / 1000))`.
//! (Spec 3.1 places the final module in `crates/core/src/limits/moosend.rs`.)

use std::fmt;

use crate::mem::MemKv;

/// Calls per window.
pub const MOOSEND_SUBSCRIBE_LIMIT: i64 = 10;
/// Window length in ms (from the first call).
pub const MOOSEND_SUBSCRIBE_WINDOW_MS: u64 = 10_000;

/// `MoosendRateLimitError`: a safe local error (no secret in the message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoosendRateLimitError {
    pub retry_after_seconds: u64,
}

impl MoosendRateLimitError {
    pub fn new(retry_after_seconds: u64) -> MoosendRateLimitError {
        let _ = retry_after_seconds;
        todo!("P2.7 K5")
    }

    /// `"local_rate_limited"`.
    pub fn kind(&self) -> &'static str {
        todo!("P2.7 K5")
    }
}

impl fmt::Display for MoosendRateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("P2.7 K5")
    }
}

impl std::error::Error for MoosendRateLimitError {}

/// `buildMoosendRateLimitKey`: `moosend:subscribe-quota:<sha256 hex of the key>`.
pub fn build_moosend_rate_limit_key(api_key: &str) -> String {
    let _ = api_key;
    todo!("P2.7 K5")
}

/// `acquireMoosendSubscribePermit`.
pub fn acquire_moosend_subscribe_permit(
    store: &MemKv,
    api_key: &str,
) -> Result<(), MoosendRateLimitError> {
    let _ = (store, api_key);
    todo!("P2.7 K5")
}
