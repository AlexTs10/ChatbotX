//! API rate limit (K1, spec 2.5): port target of
//! `apps/builder/src/lib/rate-limit/api-rate-limit.ts`. Fixed 10 s window
//! aligned to the epoch; key `${scope}:${identity}:${floor(now / 10 000)}`;
//! `limited = count > limit`; `retryAfter = ceil((10 000 - now mod 10 000) / 1000)`.
//! (Spec 3.1 places the final module in `crates/api/src/rate_limit.rs`.)

use std::collections::HashMap;
use std::fmt;

use crate::mem::MemKv;

/// Window length (`WINDOW_MS`).
pub const WINDOW_MS: i64 = 10_000;
/// Default limit per window (`REQUEST_LIMIT`).
pub const REQUEST_LIMIT: u64 = 120;

/// The counter behind the limiter (the TS `store` parameter).
pub trait RateCounter: Send + Sync {
    /// `incrWithWindow(key, ttlSeconds)`: increment; TTL only on the first.
    fn incr_with_window(&self, key: &str, ttl_secs: u64) -> i64;
}

impl RateCounter for MemKv {
    fn incr_with_window(&self, key: &str, ttl_secs: u64) -> i64 {
        let _ = (key, ttl_secs);
        todo!("P2.7 spec 2.5")
    }
}

/// Input of [`check_api_rate_limit`].
pub struct ApiRateLimitInput<'a> {
    pub scope: &'a str,
    pub key: &'a str,
    pub store: &'a dyn RateCounter,
    pub now_ms: i64,
    /// `None` = [`REQUEST_LIMIT`].
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiRateLimitResult {
    pub limited: bool,
    /// Seconds until the window ends.
    pub retry_after: u64,
}

/// The `tooManyRequests` exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimited {
    pub code: &'static str,
    pub http_status: u16,
    pub retry_after: u64,
}

impl fmt::Display for RateLimited {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("P2.7 spec 2.5")
    }
}

impl std::error::Error for RateLimited {}

/// `checkApiRateLimit`.
pub fn check_api_rate_limit(input: ApiRateLimitInput<'_>) -> ApiRateLimitResult {
    let _ = input;
    todo!("P2.7 spec 2.5")
}

/// `assertApiNotRateLimited`: `Err(RateLimited { "tooManyRequests", 429, .. })`
/// when limited; then also sets `Retry-After` in `res_headers`.
pub fn assert_api_not_rate_limited(
    input: ApiRateLimitInput<'_>,
    res_headers: Option<&mut HashMap<String, String>>,
) -> Result<(), RateLimited> {
    let _ = (input, res_headers);
    todo!("P2.7 spec 2.5")
}
