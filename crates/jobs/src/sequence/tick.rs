//! `SCHEDULER_TICK_INTERVAL_MS` (`apps/worker/src/env.ts`) and the lenient
//! `resolveTickIntervalMs` of the producer (spec 3.2: the tick is only a
//! safety net for the due-time timer; the env contract is kept).

use std::collections::BTreeMap;

use crate::enqueue::JobsError;

/// Env name.
pub const SCHEDULER_TICK_INTERVAL_ENV: &str = "SCHEDULER_TICK_INTERVAL_MS";
/// `DEFAULT_SCHEDULER_TICK_INTERVAL_MS`.
pub const DEFAULT_SCHEDULER_TICK_INTERVAL_MS: u64 = 5000;
/// `MIN_SCHEDULER_TICK_INTERVAL_MS`.
pub const MIN_SCHEDULER_TICK_INTERVAL_MS: u64 = 100;
/// `MAX_SCHEDULER_TICK_INTERVAL_MS`.
pub const MAX_SCHEDULER_TICK_INTERVAL_MS: u64 = 10_000;

/// An env value as the TypeScript `env` object can hold it: a number (the
/// validated schema value) or a raw string (`SKIP_ENV_CHECK=true`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvValue<'a> {
    Number(f64),
    Text(&'a str),
}

/// Strict env parse (the zod schema of `env.ts`): unset → 5000; an integer
/// string in 100..=10 000 → its value; anything else (`"99"`, `"0"`,
/// `"-500"`, `"10001"`, `"abc"`, `"1000.5"`) → `JobsError::Config`.
pub fn scheduler_tick_interval_from_env(env: &BTreeMap<String, String>) -> Result<u64, JobsError> {
    let Some(raw) = env.get(SCHEDULER_TICK_INTERVAL_ENV) else {
        return Ok(DEFAULT_SCHEDULER_TICK_INTERVAL_MS);
    };
    valid_interval(number_of(raw)).ok_or_else(|| {
        JobsError::Config(format!(
            "{SCHEDULER_TICK_INTERVAL_ENV} must be an integer between \
             {MIN_SCHEDULER_TICK_INTERVAL_MS} and {MAX_SCHEDULER_TICK_INTERVAL_MS}, got {raw:?}"
        ))
    })
}

/// `Number(text)`: trimmed decimal, empty = 0, anything else = NaN.
fn number_of(text: &str) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// An integer in `MIN..=MAX`.
fn valid_interval(value: f64) -> Option<u64> {
    let min = MIN_SCHEDULER_TICK_INTERVAL_MS as f64;
    let max = MAX_SCHEDULER_TICK_INTERVAL_MS as f64;
    (value.is_finite() && value.fract() == 0.0 && (min..=max).contains(&value))
        .then_some(value as u64)
}

/// `resolveTickIntervalMs(configValue, envValue)`: a non-zero config value
/// wins as is (even below the env minimum); else a valid env value (integer
/// in 100..=10 000, number or numeric string); else the default 5000.
pub fn resolve_tick_interval_ms(config: Option<u64>, env: Option<EnvValue<'_>>) -> u64 {
    if let Some(value) = config.filter(|v| *v != 0) {
        return value;
    }
    let candidate = match env {
        Some(EnvValue::Number(n)) => n,
        Some(EnvValue::Text(t)) => number_of(t),
        None => f64::NAN,
    };
    valid_interval(candidate).unwrap_or(DEFAULT_SCHEDULER_TICK_INTERVAL_MS)
}
