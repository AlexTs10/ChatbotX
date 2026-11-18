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

/// `WORKING_HOURS_MAX_DATE_OVERRIDES`.
pub const WORKING_HOURS_MAX_DATE_OVERRIDES: usize = 400;
/// `WORKING_HOURS_MAX_NAME_LENGTH`.
pub const WORKING_HOURS_MAX_NAME_LENGTH: usize = 100;

/// `workingHoursDateRangesSchema.safeParse`: the ranges as stored
/// (`[{ startMinute, endMinute }]`).
pub fn parse_working_hours_date_ranges(
    value: &serde_json::Value,
) -> Result<serde_json::Value, ZodError> {
    let items: Vec<serde_json::Value> = deserialize(value)?;
    let mut issues = Issues::default();
    let mut ranges = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        // A minute range is an interval without the weekday.
        let mut with_day = item.clone();
        if let Some(obj) = with_day.as_object_mut() {
            obj.insert("weekday".into(), 0.into());
        }
        match parse_working_hours_interval(&with_day) {
            Ok(range) => ranges.push(range),
            Err(error) => issues
                .0
                .extend(error.issues.into_iter().map(|issue| ZodIssue {
                    path: format!("{index}.{}", issue.path),
                    message: issue.message,
                })),
        }
    }
    if ranges.len() > WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY {
        issues.add(
            "",
            format!(
                "Too big: expected array to have <={WORKING_HOURS_MAX_INTERVALS_PER_WEEKDAY} items"
            ),
        );
    }
    if !issues.0.is_empty() {
        return Err(ZodError { issues: issues.0 });
    }
    ranges.sort_by_key(|r| r.start_minute);
    if ranges
        .windows(2)
        .any(|pair| pair[1].start_minute < pair[0].end_minute)
    {
        issues.add("", "Intervals overlap on this date");
    }
    let json = items
        .iter()
        .map(|item| {
            serde_json::json!({
                "startMinute": item["startMinute"],
                "endMinute": item["endMinute"],
            })
        })
        .collect();
    issues.finish(serde_json::Value::Array(json))
}

/// `workingHoursLocalDateSchema.safeParse` (`YYYY-MM-DD`, a real date).
pub fn parse_working_hours_local_date(value: &str) -> Result<(), ZodError> {
    let b = value.as_bytes();
    let shape = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if !shape {
        return Err(ZodError::one("", "Expected YYYY-MM-DD"));
    }
    let year: i32 = value[0..4].parse().unwrap_or(0);
    let month: u32 = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    if chrono::NaiveDate::from_ymd_opt(year, month, day).is_none() {
        return Err(ZodError::one("", "Not a real calendar date"));
    }
    Ok(())
}
