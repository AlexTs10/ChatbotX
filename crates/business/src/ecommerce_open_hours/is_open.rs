//! `ecommerce-open-hours/is-open.ts` (`isWithinOpenHours`).

use chrono::{Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use super::schedule::OpenHoursRange;

/// The weekday (0 = Sunday) and `HH:mm` of `at_ms` in `time_zone`; an unknown
/// zone counts as UTC (the original's `Intl.DateTimeFormat` throws).
fn local_day_and_time(at_ms: i64, time_zone: &str) -> (u32, String) {
    let utc = Utc.timestamp_millis_opt(at_ms).single().unwrap_or_default();
    let (day, hour, minute) = match time_zone.parse::<Tz>() {
        Ok(tz) => {
            let local = utc.with_timezone(&tz);
            (
                local.weekday().num_days_from_sunday(),
                local.hour(),
                local.minute(),
            )
        }
        Err(_) => (
            utc.weekday().num_days_from_sunday(),
            utc.hour(),
            utc.minute(),
        ),
    };
    (day, format!("{hour:02}:{minute:02}"))
}

/// `isWithinOpenHours`: whether `at_ms` falls in one of the ranges (a range
/// whose `to` is not after `from` runs past midnight into the next day).
pub fn is_within_open_hours(ranges: &[OpenHoursRange], time_zone: &str, at_ms: i64) -> bool {
    todo!("P3.6-D1")
}
