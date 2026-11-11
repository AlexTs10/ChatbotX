//! `calculate-next-run-at.ts`: when the next step of an enrollment runs.

use chatbotx_db::rusqlite::Connection;

use crate::clock::Clock;
use crate::sequence::repo::{SequenceCtx, SequenceError};

/// The delay fields of a `SequenceStep` row. Times are unix ms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepTiming {
    pub delay_days: i64,
    pub delay_minutes: i64,
    /// `null`, `"days"`, `"specificTime"`, ...
    pub delay_unit: Option<String>,
    pub specific_date_time_ms: Option<i64>,
}

/// `calculateNextRunAtFromStep(step, baseTime = new Date())`: the specific
/// time for `specificTime`; `base` unchanged when the delay is zero; else
/// `base + days + minutes`. `base_ms = None` reads `clock`.
pub fn calculate_next_run_at_from_step(
    step: &StepTiming,
    base_ms: Option<i64>,
    clock: &dyn Clock,
) -> i64 {
    todo!()
}

/// Result of [`calculate_next_run_at`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextRun {
    pub next_run_at_ms: i64,
    pub next_step_id: Option<String>,
}

/// `calculateNextRunAt(sequenceId, enrolledAt, client?)`: the first active
/// step (lowest `order`) and its run time; no step = `(enrolled_at, None)`.
/// Runs on `conn` (a caller transaction).
pub fn calculate_next_run_at_in(
    conn: &Connection,
    clock: &dyn Clock,
    sequence_id: &str,
    enrolled_at_ms: i64,
) -> Result<NextRun, SequenceError> {
    todo!()
}

/// [`calculate_next_run_at_in`] on a reader connection of `ctx.db`.
pub async fn calculate_next_run_at(
    ctx: &SequenceCtx,
    sequence_id: &str,
    enrolled_at_ms: i64,
) -> Result<NextRun, SequenceError> {
    todo!()
}
