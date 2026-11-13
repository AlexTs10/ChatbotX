//! Port of the interval schemas of `partials/working-hours.ts`.

use serde::{Deserialize, Serialize};

use super::shared::{Issues, ZodError, ZodIssue, deserialize};

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
    let interval: WorkingHoursInterval = deserialize(value)?;
    let mut issues = Issues::default();
    check_range(&mut issues, "weekday", interval.weekday, 0, 6);
    check_range(
        &mut issues,
        "startMinute",
        interval.start_minute,
        0,
        WORKING_HOURS_MINUTES_PER_DAY - 1,
    );
    check_range(
        &mut issues,
        "endMinute",
        interval.end_minute,
        1,
        WORKING_HOURS_MINUTES_PER_DAY,
    );
    // zod runs the refinement only on an input without issues.
    if issues.0.is_empty() && interval.start_minute >= interval.end_minute {
        issues.add("endMinute", "endMinute must be greater than startMinute");
    }
    issues.finish(interval)
}

/// `z.number().int().min(min).max(max)` (the `int` part is the serde `i64`).
fn check_range(issues: &mut Issues, path: &str, value: i64, min: i64, max: i64) {
    if value < min {
        issues.add(path, format!("Too small: expected number to be >={min}"));
    } else if value > max {
        issues.add(path, format!("Too big: expected number to be <={max}"));
    }
}

/// `workingHoursIntervalsSchema.safeParse` (no overlap per weekday, count limits).
pub fn parse_working_hours_intervals(
    value: &serde_json::Value,
) -> Result<Vec<WorkingHoursInterval>, ZodError> {
    let items: Vec<serde_json::Value> = deserialize(value)?;
    let mut issues = Issues::default();
    let mut intervals = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match parse_working_hours_interval(item) {
            Ok(interval) => intervals.push(interval),
            Err(error) => issues
                .0
                .extend(error.issues.into_iter().map(|issue| ZodIssue {
                    path: format!("{index}.{}", issue.path),
                    message: issue.message,
                })),
        }
    }
    if intervals.len() > WORKING_HOURS_MAX_INTERVALS_TOTAL {
        issues.add(
            "",
            format!("Too big: expected array to have <={WORKING_HOURS_MAX_INTERVALS_TOTAL} items"),
        );
    }
    if !issues.0.is_empty() {
        return issues.finish(intervals);
    }
    // Weekdays in first-seen order (the TS `Map` insertion order).
    let mut weekdays: Vec<i64> = Vec::new();
    for interval in &intervals {
        if !weekdays.contains(&interval.weekday) {
            weekdays.push(interval.weekday);
        }
    }
    for weekday in weekdays {
        let mut day: Vec<&WorkingHoursInterval> =
            intervals.iter().filter(|i| i.weekday == weekday).collect();
        if day.len() > WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY {
            issues.add(
                "",
                format!(
                    "At most {WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY} intervals per weekday (weekday {weekday})"
                ),
            );
        }
        day.sort_by_key(|i| i.start_minute);
        if day
            .windows(2)
            .any(|pair| pair[1].start_minute < pair[0].end_minute)
        {
            issues.add("", format!("Intervals overlap on weekday {weekday}"));
        }
    }
    issues.finish(intervals)
}
