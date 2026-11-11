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
    todo!()
}

/// `resolveTickIntervalMs(configValue, envValue)`: a non-zero config value
/// wins as is (even below the env minimum); else a valid env value (integer
/// in 100..=10 000, number or numeric string); else the default 5000.
pub fn resolve_tick_interval_ms(config: Option<u64>, env: Option<EnvValue<'_>>) -> u64 {
    todo!()
}
