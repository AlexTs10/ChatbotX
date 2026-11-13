//! Port of the interval schemas of `partials/working-hours.ts`.

use serde::{Deserialize, Serialize};

use super::shared::ZodError;

/// `WORKING_HOURS_MINUTES_PER_DAY`.
pub const WORKING_HOURS_MINUTES_PER_DAY: i64 = 1440;
/// `WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY`.
pub const WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY: usize = 10;
/// `WORKING_HOURS_MAX_INTERVALS_TOTAL`.
pub const WORKING_HOURS_MAX_INTERVALS_TOTAL: usize = 70;

/// `WorkingHoursIntervalInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingHoursInterval {
    pub weekday: i64,
    pub start_minute: i64,
    pub end_minute: i64,
}

/// `workingHoursIntervalSchema.safeParse`.
pub fn parse_working_hours_interval(
    value: &serde_json::Value,
) -> Result<WorkingHoursInterval, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `workingHoursIntervalsSchema.safeParse` (no overlap per weekday, count limits).
pub fn parse_working_hours_intervals(
    value: &serde_json::Value,
) -> Result<Vec<WorkingHoursInterval>, ZodError> {
    let _ = value;
    todo!("P3.2b")
}
