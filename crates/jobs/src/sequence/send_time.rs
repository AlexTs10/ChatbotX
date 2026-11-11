//! `send-time-validator.ts`: move a run time into the allowed send days and
//! the daily send window. Local times are in an explicit zone (spec 5.2:
//! the TypeScript code used the process zone; production passes UTC).

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// `MAX_ATTEMPTS`: days tried before giving up and returning the base time.
pub const MAX_ATTEMPTS: u32 = 14;

/// The send settings of a step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendWindow {
    pub anytime: bool,
    /// JSON text such as `["monday","tuesday"]`; `None` or invalid JSON =
    /// every day.
    pub send_days: Option<String>,
    /// `"HH:MM"`.
    pub send_time_start: Option<String>,
    /// `"HH:MM"`; a time `>=` the end rolls to the next allowed day.
    pub send_time_end: Option<String>,
}

/// `calculateNextValidSendTime(baseTime, step)` in zone `tz`; unix ms in
/// and out.
pub fn calculate_next_valid_send_time(base_ms: i64, window: &SendWindow, tz: Tz) -> i64 {
    if window.anytime {
        return base_ms;
    }
    let Some(base) = Utc.timestamp_millis_opt(base_ms).single() else {
        return base_ms;
    };
    let allowed = parse_send_days(window.send_days.as_deref());
    let range = match (&window.send_time_start, &window.send_time_end) {
        (Some(start), Some(end)) if !start.is_empty() && !end.is_empty() => {
            Some((minutes_of(start), minutes_of(end)))
        }
        _ => None,
    };
    let mut result = base.with_timezone(&tz);
    for _ in 0..MAX_ATTEMPTS {
        let day = DAY_NAMES[result.weekday().num_days_from_sunday() as usize];
        if !allowed.iter().any(|d| d == day) {
            result = start_of_next_day(result, tz);
            continue;
        }
        if let Some((start, end)) = range {
            let now = i64::from(result.hour()) * 60 + i64::from(result.minute());
            if now < start {
                let (hour, minute) = (start.div_euclid(60), start.rem_euclid(60));
                let snapped = result
                    .date_naive()
                    .and_hms_opt(hour as u32, minute as u32, 0)
                    .and_then(|t| tz.from_local_datetime(&t).earliest());
                return snapped.map_or(base_ms, |t| t.timestamp_millis());
            }
            if now >= end {
                result = start_of_next_day(result, tz);
                continue;
            }
        }
        return result.timestamp_millis();
    }
    base_ms
}

/// `getDay` names, Sunday first.
const DAY_NAMES: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// `parseSendDays`: `None` or invalid JSON = every day; a JSON non-array =
/// no day.
fn parse_send_days(send_days: Option<&str>) -> Vec<String> {
    let all = || DAY_NAMES.iter().map(|d| (*d).to_owned()).collect();
    let Some(text) = send_days.filter(|t| !t.is_empty()) else {
        return all();
    };
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Ok(_) => Vec::new(),
        Err(_) => all(),
    }
}

/// `"HH:MM"` as minutes of the day (`NaN` parts count as 0).
fn minutes_of(time: &str) -> i64 {
    let mut parts = time
        .split(':')
        .map(|p| p.trim().parse::<i64>().unwrap_or(0));
    let hour = parts.next().unwrap_or(0);
    let minute = parts.next().unwrap_or(0);
    hour * 60 + minute
}

/// `startOfDay(addDays(t, 1))` in `tz`.
fn start_of_next_day(t: DateTime<Tz>, tz: Tz) -> DateTime<Tz> {
    let next = t.date_naive().succ_opt().unwrap_or(t.date_naive());
    next.and_hms_opt(0, 0, 0)
        .and_then(|midnight| tz.from_local_datetime(&midnight).earliest())
        .unwrap_or(t)
}
