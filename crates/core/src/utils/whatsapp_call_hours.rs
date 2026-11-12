//! `packages/utils/src/whatsapp-call-hours.ts`: whether a WhatsApp call
//! arrives inside the number's business hours. The schedule is the jsonb
//! value as stored (malformed shapes never refuse a call).

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::tz::Zone;

/// Meta's day names, indexed like `getUTCDay()` (0 = Sunday).
const DAY_BY_INDEX: [&str; 7] = [
    "SUNDAY",
    "MONDAY",
    "TUESDAY",
    "WEDNESDAY",
    "THURSDAY",
    "FRIDAY",
    "SATURDAY",
];

/// `CALL_HOURS_MINUTES_PER_DAY`.
pub const CALL_HOURS_MINUTES_PER_DAY: u32 = 24 * 60;

/// `parseCallTime`: minutes since midnight of `HHMM` / `HH:MM`, or `None`.
pub fn parse_call_time(value: &str) -> Option<u32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new("^([0-9]{1,2}):?([0-9]{2})$").expect("valid regex"));
    let caps = re.captures(crate::js::trim(value))?;
    let hours: u32 = caps[1].parse().ok()?;
    let minutes: u32 = caps[2].parse().ok()?;
    (hours <= 23 && minutes <= 59).then_some(hours * 60 + minutes)
}

fn time_of(value: Option<&Value>) -> Option<u32> {
    value.and_then(Value::as_str).and_then(parse_call_time)
}

fn holiday_covers_minute(holiday: &Value, minute: u32) -> bool {
    let start = time_of(holiday.get("startTime"));
    let end = time_of(holiday.get("endTime"));
    match (start, end) {
        (Some(start), Some(end)) if end > start => minute >= start && minute <= end,
        _ => true,
    }
}

fn covers_minute(window: &Value, minute: u32) -> bool {
    let (Some(open), Some(close)) = (
        time_of(window.get("openTime")),
        time_of(window.get("closeTime")),
    ) else {
        return true;
    };
    if close <= open {
        minute >= open
    } else {
        minute >= open && minute < close
    }
}

fn spills_into_next_day(window: &Value) -> Option<u32> {
    let open = time_of(window.get("openTime"))?;
    let close = time_of(window.get("closeTime"))?;
    (close <= open).then_some(close)
}

/// `isWithinCallHours`: open by default (no / disabled / empty / unreadable
/// schedule, unknown zone); otherwise the weekly windows in the schedule's
/// zone, a holiday on its local date replacing them.
pub fn is_within_call_hours(schedule: Option<&Value>, at_ms: i64) -> bool {
    let Some(schedule) = schedule.filter(|s| s.is_object()) else {
        return true;
    };
    if schedule.get("status").and_then(Value::as_str) != Some("ENABLED") {
        return true;
    }
    let Some(windows) = schedule
        .get("weeklyOperatingHours")
        .and_then(Value::as_array)
    else {
        return true;
    };
    if windows.is_empty() {
        return true;
    }
    let Some(zone) = schedule
        .get("timezoneId")
        .and_then(Value::as_str)
        .and_then(Zone::parse)
    else {
        return true;
    };
    let local = zone.wall_clock(at_ms);
    let day = DAY_BY_INDEX[local.weekday as usize];
    let minute = local.hour * 60 + local.minute;
    let date = format!("{:04}-{:02}-{:02}", local.year, local.month, local.day);

    let holidays: Vec<&Value> = schedule
        .get("holidaySchedule")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|h| {
                    h.is_object() && h.get("date").and_then(Value::as_str) == Some(date.as_str())
                })
                .collect()
        })
        .unwrap_or_default();
    if !holidays.is_empty() {
        return holidays.iter().any(|h| holiday_covers_minute(h, minute));
    }

    let previous_day = DAY_BY_INDEX[(local.weekday as usize + 6) % 7];
    windows.iter().any(|window| {
        if !window.is_object() {
            return true;
        }
        let window_day = window
            .get("dayOfWeek")
            .and_then(Value::as_str)
            .map(str::to_uppercase)
            .unwrap_or_default();
        if !DAY_BY_INDEX.contains(&window_day.as_str()) {
            return true;
        }
        if window_day == day {
            return covers_minute(window, minute);
        }
        if window_day != previous_day {
            return false;
        }
        spills_into_next_day(window).is_some_and(|spill| minute < spill)
    })
}
