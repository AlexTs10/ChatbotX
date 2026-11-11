//! Job options (BullMQ `JobsOptions` subset in use, spec 2.1 / 2.13a).
//!
//! Every field is an `Option` so that "not set" (TypeScript `{}`) and an
//! explicit value stay apart, as the frozen tests compare whole option objects.

use serde_json::Value;

/// `backoff: { type, delay }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backoff {
    Fixed(u64),
    Exponential(u64),
}

impl Backoff {
    /// Delay before the retry after `attempts_made` failed attempts
    /// (BullMQ `classes/backoffs.js`, no jitter): fixed = delay,
    /// exponential = `round(2^(attempts_made - 1) * delay)`.
    pub fn delay_ms(self, attempts_made: u32) -> u64 {
        match self {
            Backoff::Fixed(delay) => delay,
            Backoff::Exponential(delay) => {
                let exp = i32::try_from(attempts_made)
                    .unwrap_or(i32::MAX)
                    .saturating_sub(1);
                let value = 2f64.powi(exp) * delay as f64;
                if value >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    value.round() as u64
                }
            }
        }
    }

    /// The configured delay (`backoff.delay`).
    pub fn base_ms(self) -> u64 {
        match self {
            Backoff::Fixed(delay) | Backoff::Exponential(delay) => delay,
        }
    }
}

/// `removeOnComplete` / `removeOnFail` (spec 2.8; per-job, DECISIONS P2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// `true`: delete the row when the job finishes.
    Remove,
    /// `false` / no option anywhere: keep the row.
    Keep,
    /// `number` or `{ count }`.
    KeepCount(u32),
    /// `{ age }` (seconds) or `{ age, count }`.
    KeepAge { age_secs: u64, count: Option<u32> },
}

/// `deduplication: { id, ttl, extend, replace, keepLastIfActive }` (spec S3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dedup {
    pub id: String,
    pub ttl_ms: Option<u64>,
    pub extend: bool,
    pub replace: bool,
    pub keep_last_if_active: bool,
}

/// The options of one `add`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobOptions {
    pub job_id: Option<String>,
    pub delay_ms: Option<u64>,
    pub priority: Option<u32>,
    pub attempts: Option<u32>,
    pub backoff: Option<Backoff>,
    pub remove_on_complete: Option<Retention>,
    pub remove_on_fail: Option<Retention>,
    pub dedup: Option<Dedup>,
}

impl JobOptions {
    /// `{ ...defaults, ...producer }`: every field set in `producer` wins.
    pub fn merge(defaults: &JobOptions, producer: &JobOptions) -> JobOptions {
        JobOptions {
            job_id: producer.job_id.clone().or_else(|| defaults.job_id.clone()),
            delay_ms: producer.delay_ms.or(defaults.delay_ms),
            priority: producer.priority.or(defaults.priority),
            attempts: producer.attempts.or(defaults.attempts),
            backoff: producer.backoff.or(defaults.backoff),
            remove_on_complete: producer.remove_on_complete.or(defaults.remove_on_complete),
            remove_on_fail: producer.remove_on_fail.or(defaults.remove_on_fail),
            dedup: producer.dedup.clone().or_else(|| defaults.dedup.clone()),
        }
    }
}

/// One job of an `add_bulk`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub name: String,
    pub data: Value,
    pub opts: JobOptions,
}

/// Result of an `add`: the job id and whether a new row was written
/// (`false` = deduplicated by job id or `deduplication`, spec S2/S3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddResult {
    pub job_id: String,
    pub created: bool,
}
