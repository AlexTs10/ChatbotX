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
        todo!()
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
        todo!()
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
