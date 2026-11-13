//! Port of `@chatbotx.io/utils/contact-filter-value-format` and the filter-window parts of
//! `@chatbotx.io/utils/datetime` (`chatbotx-db` cannot depend on `chatbotx-core`, which
//! depends on it).

use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use chrono_tz::Tz;
use regex::Regex;

use crate::functions::{from_wall_ms, tz_resolve};

/// `NUMERIC_VALUE_PATTERN` (ASCII digits).
pub(crate) static NUMERIC_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?[0-9]+(\.[0-9]+)?$").expect("regex"));

/// `DATETIME_VALUE_PATTERN` with capture groups.
static DATETIME_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^([0-9]{4})-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])(?:[T ]([01][0-9]|2[0-3]):([0-5][0-9])(?::([0-5][0-9])(?:\.[0-9]{1,6})?)?(Z|[+-](?:[01][0-9]|2[0-3]):?[0-5][0-9])?)?$",
    )
    .expect("regex")
});

/// One parsed filter value: the floored wall clock, the typed precision and the offset.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FilterValue {
    pub wall: NaiveDateTime,
    pub unit_ms: i64,
    /// Explicit offset in seconds east of UTC.
    pub offset: Option<i32>,
}

pub(crate) fn parse_filter_value(value: &str) -> Option<FilterValue> {
    let c = DATETIME_VALUE.captures(value)?;
    let num = |i: usize| c.get(i).map(|m| m.as_str().parse::<u32>().unwrap_or(0));
    let date = NaiveDate::from_ymd_opt(num(1)? as i32, num(2)?, num(3)?)?;
    let (h, m, s) = (num(4), num(5), num(6));
    let unit_ms = match (h, s) {
        (None, _) => 86_400_000,
        (Some(_), None) => 60_000,
        (Some(_), Some(_)) => 1000,
    };
    let time = NaiveTime::from_hms_opt(h.unwrap_or(0), m.unwrap_or(0), s.unwrap_or(0))?;
    let offset = c.get(7).map(|o| {
        let o = o.as_str();
        if o == "Z" {
            return 0;
        }
        let sign = if o.starts_with('-') { -1 } else { 1 };
        let digits: String = o[1..].chars().filter(|ch| ch.is_ascii_digit()).collect();
        let hh: i32 = digits[..2].parse().unwrap_or(0);
        let mm: i32 = digits[2..].parse().unwrap_or(0);
        sign * (hh * 3600 + mm * 60)
    });
    Some(FilterValue {
        wall: date.and_time(time),
        unit_ms,
        offset,
    })
}

/// `isValidDateTimeFilterValue`: the pattern and a real calendar day.
pub fn is_valid_date_time_filter_value(value: &str) -> bool {
    parse_filter_value(value).is_some()
}

/// `hasExplicitOffset`.
pub(crate) fn has_explicit_offset(value: &str) -> bool {
    parse_filter_value(value).is_some_and(|v| v.offset.is_some())
}

/// `resolveFilterTimezone` (unknown zone -> UTC).
pub(crate) fn resolve_zone(timezone: Option<&str>) -> Tz {
    tz_resolve(timezone).parse().unwrap_or(Tz::UTC)
}

fn wall_ms(wall: NaiveDateTime) -> i64 {
    wall.and_utc().timestamp_millis()
}

/// `filterValueToUtcInstantWindow`: `[start, end)` unix ms of the typed precision unit,
/// anchored to the value's own offset, else to `zone`.
pub(crate) fn instant_window(value: &str, zone: Tz) -> Option<(i64, i64)> {
    let v = parse_filter_value(value)?;
    let start_wall = wall_ms(v.wall);
    let end_wall = start_wall + v.unit_ms;
    match v.offset {
        Some(off) => {
            let shift = i64::from(off) * 1000;
            Some((start_wall - shift, end_wall - shift))
        }
        None => Some((
            from_wall_ms(start_wall, zone).ok()?,
            from_wall_ms(end_wall, zone).ok()?,
        )),
    }
}

/// `temporalWallClockWindow`: the naive `[start, end)` wall clock (`YYYY-MM-DDTHH:MM:SS`).
pub(crate) fn wall_window(value: &str) -> Option<(String, String)> {
    let v = parse_filter_value(value)?;
    let end = v.wall + Duration::milliseconds(v.unit_ms);
    Some((naive_text(v.wall), naive_text(end)))
}

/// The naive day `[start, end)` of the value's date part (`filterValueToUtcDayStartIso` /
/// `DayEndIso` in UTC, read as wall clock).
pub(crate) fn wall_day(value: &str) -> Option<(String, String)> {
    let v = parse_filter_value(value)?;
    let start = v.wall.date().and_time(NaiveTime::MIN);
    Some((naive_text(start), naive_text(start + Duration::days(1))))
}

/// `YYYY-MM-DD` of the value.
pub(crate) fn date_part(value: &str) -> Option<String> {
    parse_filter_value(value).map(|v| naive_text(v.wall)[..10].to_owned())
}

/// `hasTimeComponent`.
pub(crate) fn has_time_component(value: &str) -> bool {
    parse_filter_value(value).is_some_and(|v| v.unit_ms < 86_400_000)
}

fn naive_text(t: NaiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}
