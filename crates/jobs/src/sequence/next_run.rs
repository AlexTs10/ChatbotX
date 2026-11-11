//! `calculate-next-run-at.ts`: when the next step of an enrollment runs.

use std::sync::Arc;

use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::clock::Clock;
use crate::sequence::repo::{SequenceCtx, SequenceError, parse_id, read_tx};

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
    let base = base_ms.unwrap_or_else(|| clock.now_ms());
    if step.delay_unit.as_deref() == Some("specificTime")
        && let Some(specific) = step.specific_date_time_ms
    {
        return specific;
    }
    let delay = step
        .delay_days
        .saturating_mul(DAY_IN_MS)
        .saturating_add(step.delay_minutes.saturating_mul(MINUTE_IN_MS));
    if delay <= 0 {
        base
    } else {
        base.saturating_add(delay)
    }
}

const MINUTE_IN_MS: i64 = 60_000;
const DAY_IN_MS: i64 = 24 * 60 * MINUTE_IN_MS;

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
    let first: Option<(i64, StepTiming)> = conn
        .query_row(
            r#"SELECT "id", "delayDays", "delayMinutes", "delayUnit", "specificDateTime"
               FROM "SequenceStep" WHERE "sequenceId" = ?1 AND "order" = 0 AND "isActive" = 1
               LIMIT 1"#,
            params![parse_id(sequence_id)?],
            |r| {
                Ok((
                    r.get(0)?,
                    StepTiming {
                        delay_days: r.get(1)?,
                        delay_minutes: r.get(2)?,
                        delay_unit: r.get(3)?,
                        specific_date_time_ms: r.get(4)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(DbError::from)?;
    Ok(match first {
        None => NextRun {
            next_run_at_ms: enrolled_at_ms,
            next_step_id: None,
        },
        Some((id, timing)) => NextRun {
            next_run_at_ms: calculate_next_run_at_from_step(&timing, Some(enrolled_at_ms), clock),
            next_step_id: Some(id.to_string()),
        },
    })
}

/// [`calculate_next_run_at_in`] on a reader connection of `ctx.db`.
pub async fn calculate_next_run_at(
    ctx: &SequenceCtx,
    sequence_id: &str,
    enrolled_at_ms: i64,
) -> Result<NextRun, SequenceError> {
    let clock = Arc::clone(&ctx.clock);
    let sequence_id = sequence_id.to_owned();
    read_tx(&ctx.db, move |conn| {
        calculate_next_run_at_in(conn, clock.as_ref(), &sequence_id, enrolled_at_ms)
    })
    .await
}
