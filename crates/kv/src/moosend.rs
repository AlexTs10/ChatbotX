//! Moosend subscribe quota (K5, spec 2.5): port target of
//! `apps/worker/src/integration/handlers/moosend-rate-limit.ts`.
//! 10 calls per 10 000 ms from the first call, keyed by a SHA-256 API-key
//! fingerprint; `retryAfter = max(1, ceil(pttl / 1000))`.
//! (Spec 3.1 places the final module in `crates/core/src/limits/moosend.rs`.)

use std::fmt;

use sha2::{Digest, Sha256};

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
        MoosendRateLimitError {
            retry_after_seconds,
        }
    }

    /// `"local_rate_limited"`.
    pub fn kind(&self) -> &'static str {
        "local_rate_limited"
    }
}

impl fmt::Display for MoosendRateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Moosend local rate limit exceeded")
    }
}

impl std::error::Error for MoosendRateLimitError {}

/// `buildMoosendRateLimitKey`: `moosend:subscribe-quota:<sha256 hex of the key>`.
pub fn build_moosend_rate_limit_key(api_key: &str) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(api_key.as_bytes());
    let mut key = String::with_capacity(24 + 64);
    key.push_str("moosend:subscribe-quota:");
    for byte in digest.iter() {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// `acquireMoosendSubscribePermit`.
pub fn acquire_moosend_subscribe_permit(
    store: &MemKv,
    api_key: &str,
) -> Result<(), MoosendRateLimitError> {
    let (count, pttl_ms) = store.incr_with_window_ms(
        &build_moosend_rate_limit_key(api_key),
        MOOSEND_SUBSCRIBE_WINDOW_MS,
    );
    if count > MOOSEND_SUBSCRIBE_LIMIT {
        // `Math.max(1, Math.ceil(pttl / 1000))`.
        let retry_after = u64::try_from(pttl_ms).map_or(1, |ms| ms.div_ceil(1000).max(1));
        return Err(MoosendRateLimitError::new(retry_after));
    }
    Ok(())
}
