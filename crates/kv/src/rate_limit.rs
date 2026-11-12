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
        MemKv::incr_with_window(self, key, ttl_secs)
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
        write!(f, "Too many requests. Retry after {}s.", self.retry_after)
    }
}

/// `WINDOW_SECONDS`.
const WINDOW_SECS: u64 = 10;
/// HTTP status of `tooManyRequests`.
const TOO_MANY_REQUESTS_STATUS: u16 = 429;

impl std::error::Error for RateLimited {}

/// `checkApiRateLimit`.
pub fn check_api_rate_limit(input: ApiRateLimitInput<'_>) -> ApiRateLimitResult {
    let window = input.now_ms.div_euclid(WINDOW_MS);
    let elapsed = input.now_ms.rem_euclid(WINDOW_MS);
    let retry_after = u64::try_from((WINDOW_MS - elapsed + 999) / 1000).unwrap_or(0);
    let key = format!("{}:{}:{window}", input.scope, input.key);
    let count = input.store.incr_with_window(&key, WINDOW_SECS);
    let limit = input.limit.unwrap_or(REQUEST_LIMIT);
    ApiRateLimitResult {
        limited: u64::try_from(count).is_ok_and(|count| count > limit),
        retry_after,
    }
}

/// `assertApiNotRateLimited`: `Err(RateLimited { "tooManyRequests", 429, .. })`
/// when limited; then also sets `Retry-After` in `res_headers`.
pub fn assert_api_not_rate_limited(
    input: ApiRateLimitInput<'_>,
    res_headers: Option<&mut HashMap<String, String>>,
) -> Result<(), RateLimited> {
    let result = check_api_rate_limit(input);
    if !result.limited {
        return Ok(());
    }
    if let Some(headers) = res_headers {
        headers.insert("Retry-After".to_owned(), result.retry_after.to_string());
    }
    Err(RateLimited {
        code: "tooManyRequests",
        http_status: TOO_MANY_REQUESTS_STATUS,
        retry_after: result.retry_after,
    })
}
