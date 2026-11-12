//! `packages/utils/src/datetime.ts`: temporal custom-field values, filter
//! windows and time zone resolution.

use std::sync::OnceLock;

use regex::Regex;

use super::tz::{self, DateArg};
use crate::js;

/// `DEFAULT_FILTER_TIMEZONE`.
pub const DEFAULT_FILTER_TIMEZONE: &str = "UTC";
/// Length of the `yyyy-MM-dd` prefix.
pub const DATE_PART_LENGTH: usize = 10;
/// `DATE_FORMAT`.
pub const DATE_FORMAT: &str = "yyyy-MM-dd";
/// `DATE_TIME_FORMAT`.
pub const DATE_TIME_FORMAT: &str = "yyyy-MM-dd HH:mm:ss";
const ZONED_ISO_FORMAT: &str = "yyyy-MM-dd'T'HH:mm:ssXXX";
const NAIVE_DATETIME_FORMAT: &str = "yyyy-MM-dd'T'HH:mm:ss";

string_enum! {
    /// `temporalCustomFieldTypes`.
    pub enum TemporalCustomFieldType {
        Date = "date",
        Datetime = "datetime",
    }
}

string_enum! {
    /// `TemporalInputParsing`.
    pub enum TemporalInputParsing {
        /// Canonical ISO only (UI forms and APIs).
        Strict = "strict",
        /// The loose multi-format parser first (imports).
        Lenient = "lenient",
    }
}

string_enum! {
    /// `SourceTimezoneStrategy`.
    pub enum SourceTimezoneStrategy {
        ContactThenWorkspace = "contactThenWorkspace",
        Workspace = "workspace",
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn offset_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    re(&RE, "(?:Z|[+-][0-9]{2}:?[0-9]{2})$")
}

fn time_component_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    re(&RE, "[0-9]{2}:[0-9]{2}")
}

/// `hasExplicitOffset`: ends with `Z` or `±hh[:]mm`.
pub fn has_explicit_offset(value: &str) -> bool {
    offset_suffix_re().is_match(value)
}

fn offset_suffix_of(value: &str) -> &str {
    offset_suffix_re().find(value).map_or("", |m| m.as_str())
}

fn to_local_iso(value: &str) -> String {
    value.replacen(' ', "T", 1)
}

/// `datePartOf`: the first 10 characters.
pub fn date_part_of(value: &str) -> &str {
    js::slice_utf16(value, 0, DATE_PART_LENGTH)
}

fn rest_after_date_part(value: &str) -> &str {
    let date = js::slice_utf16(value, 0, DATE_PART_LENGTH);
    &value[date.len()..]
}

/// `hasTimeComponent`: an `HH:mm` after the date part.
pub fn has_time_component(value: &str) -> bool {
    time_component_re().is_match(rest_after_date_part(value))
}

/// `toNaiveWallClockLiteral`: the wall clock with any offset stripped.
pub fn to_naive_wall_clock_literal(value: &str) -> String {
    let local = to_local_iso(&offset_suffix_re().replace(value, ""));
    if time_component_re().is_match(rest_after_date_part(&local)) {
        local
    } else {
        format!("{}T00:00:00", js::slice_utf16(&local, 0, DATE_PART_LENGTH))
    }
}

/// `isRealCalendarDate`: `YYYY-MM-DD` naming a real day (`Date.UTC` maps
/// years 0–99 to 19xx, so those never round-trip).
pub fn is_real_calendar_date(date_part: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let Some(caps) = re(&RE, "^([0-9]{4})-([0-9]{2})-([0-9]{2})$").captures(date_part) else {
        return false;
    };
    let year: i64 = caps[1].parse().unwrap_or(0);
    let month: i64 = caps[2].parse().unwrap_or(0);
    let day: i64 = caps[3].parse().unwrap_or(0);
    if year < 100 {
        return false;
    }
    let probe = js::utc_fields(js::date_utc(year, month - 1, day, 0, 0, 0, 0));
    probe.year == year && i64::from(probe.month) == month && i64::from(probe.day) == day
}

fn is_valid_datetime_value(value: &str) -> bool {
    (value.contains('T') || value.contains(' '))
        && is_real_calendar_date(js::slice_utf16(value, 0, DATE_PART_LENGTH))
        && js::parse_date(&to_local_iso(value)).is_some()
}

/// `resolveFilterTimezone`: the zone when `Intl` accepts it, else `UTC`.
pub fn resolve_filter_timezone(timezone: Option<&str>) -> String {
    match timezone {
        Some(tz) if !tz.is_empty() && tz::is_intl_time_zone(tz) => tz.to_owned(),
        _ => DEFAULT_FILTER_TIMEZONE.to_owned(),
    }
}

/// `formatWithFallback`: never fails — an unusable zone falls back to UTC,
/// a corrupt value to the raw string (or `""` for an instant).
pub fn format_with_fallback(date: DateArg<'_>, timezone: Option<&str>, pattern: &str) -> String {
    tz::format_in_time_zone(date, timezone.unwrap_or(DEFAULT_FILTER_TIMEZONE), pattern)
        .or_else(|_| tz::format_in_time_zone(date, DEFAULT_FILTER_TIMEZONE, pattern))
        .unwrap_or_else(|_| match date {
            DateArg::Text(text) => text.to_owned(),
            DateArg::Ms(_) => String::new(),
        })
}

fn iso(ms: i64) -> Result<String, &'static str> {
    js::to_iso_string(ms).ok_or(tz::INVALID_TIME_VALUE)
}

fn explicit_instant_iso(value: &str) -> Result<String, &'static str> {
    iso(js::parse_date(value).ok_or(tz::INVALID_TIME_VALUE)?)
}

/// `filterValueToUtcIso`: an explicit offset is kept; a naive value is
/// read in `timezone`. `Err("Invalid time value")` where the original throws.
pub fn filter_value_to_utc_iso(value: &str, timezone: &str) -> Result<String, &'static str> {
    if has_explicit_offset(value) {
        return explicit_instant_iso(value);
    }
    iso(tz::from_zoned_time(&to_local_iso(value), timezone)?)
}

/// `filterValueToUtcDayStartIso`: local midnight of the date part.
pub fn filter_value_to_utc_day_start_iso(
    value: &str,
    timezone: &str,
) -> Result<String, &'static str> {
    iso(tz::from_zoned_time(
        &format!("{}T00:00:00", date_part_of(value)),
        timezone,
    )?)
}

/// `filterValueToUtcDayEndIso`: local midnight of the next day.
pub fn filter_value_to_utc_day_end_iso(
    value: &str,
    timezone: &str,
) -> Result<String, &'static str> {
    let day = tz::tz_to_date(date_part_of(value), None).ok_or(tz::INVALID_TIME_VALUE)?;
    let next_day = tz::format_utc(day + 86_400_000, DATE_FORMAT);
    iso(tz::from_zoned_time(
        &format!("{next_day}T00:00:00"),
        timezone,
    )?)
}

/// `toZonedDayStartIso`: local midnight as an offset-preserved ISO string
/// (`2026-07-22T00:00:00+07:00`).
pub fn to_zoned_day_start_iso(value: &str, timezone: &str) -> Result<String, &'static str> {
    let safe = resolve_filter_timezone(Some(timezone));
    let day_start = tz::from_zoned_time(&format!("{}T00:00:00", date_part_of(value)), &safe)?;
    tz::format_in_time_zone(DateArg::Ms(day_start), &safe, ZONED_ISO_FORMAT)
}

/// `toZonedWallClock`: the wall-clock fields of an instant in `timezone`
/// (UTC when unusable).
pub fn to_zoned_wall_clock(instant_ms: i64, timezone: Option<&str>) -> js::UtcFields {
    let zone = tz::Zone::parse(&resolve_filter_timezone(timezone)).unwrap_or_else(tz::Zone::utc);
    zone.wall_clock(instant_ms)
}

string_enum! {
    /// `TemporalPrecision`: the granularity a value was typed at.
    pub enum TemporalPrecision {
        Day = "day",
        Minute = "minute",
        Second = "second",
    }
}

/// `detectTemporalPrecision`.
pub fn detect_temporal_precision(value: &str) -> TemporalPrecision {
    static SECOND: OnceLock<Regex> = OnceLock::new();
    let stripped = offset_suffix_re().replace(value, "");
    let time_part = rest_after_date_part(&stripped);
    if re(&SECOND, "[0-9]{2}:[0-9]{2}:[0-9]{2}").is_match(time_part) {
        TemporalPrecision::Second
    } else if time_component_re().is_match(time_part) {
        TemporalPrecision::Minute
    } else {
        TemporalPrecision::Day
    }
}

fn floored_naive_literal(value: &str) -> String {
    static FRACTION: OnceLock<Regex> = OnceLock::new();
    static FULL_TIME: OnceLock<Regex> = OnceLock::new();
    let naive = re(&FRACTION, r"\.[0-9]+$")
        .replace(&to_naive_wall_clock_literal(value), "")
        .into_owned();
    if re(&FULL_TIME, "T[0-9]{2}:[0-9]{2}:[0-9]{2}$").is_match(&naive) {
        naive
    } else {
        format!("{naive}:00")
    }
}

/// A naive wall-clock window `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallClockWindow {
    pub start: String,
    pub end: String,
}

/// `temporalWallClockWindow`: one unit of the typed precision.
pub fn temporal_wall_clock_window(value: &str) -> Result<WallClockWindow, &'static str> {
    let start = floored_naive_literal(value);
    let start_ms = tz::from_zoned_time(&start, DEFAULT_FILTER_TIMEZONE)?;
    let end_ms = match detect_temporal_precision(value) {
        TemporalPrecision::Day => start_ms + 86_400_000,
        TemporalPrecision::Minute => start_ms + 60_000,
        TemporalPrecision::Second => start_ms + 1000,
    };
    let end = tz::format_in_time_zone(
        DateArg::Ms(end_ms),
        DEFAULT_FILTER_TIMEZONE,
        NAIVE_DATETIME_FORMAT,
    )?;
    Ok(WallClockWindow { start, end })
}

/// An instant window `[start_iso, end_iso)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantWindow {
    pub start_iso: String,
    pub end_iso: String,
}

/// `filterValueToUtcInstantWindow`: the precision window anchored to
/// `timezone`, or to the value's own offset.
pub fn filter_value_to_utc_instant_window(
    value: &str,
    timezone: &str,
) -> Result<InstantWindow, &'static str> {
    let WallClockWindow { start, end } = temporal_wall_clock_window(value)?;
    let to_instant = |naive: &str| -> Result<String, &'static str> {
        if has_explicit_offset(value) {
            explicit_instant_iso(&format!("{naive}{}", offset_suffix_of(value)))
        } else {
            iso(tz::from_zoned_time(naive, timezone)?)
        }
    };
    Ok(InstantWindow {
        start_iso: to_instant(&start)?,
        end_iso: to_instant(&end)?,
    })
}

/// `isTemporalCustomFieldType`.
pub fn is_temporal_custom_field_type(kind: &str) -> bool {
    TemporalCustomFieldType::parse(kind).is_some()
}

/// `normalizeTemporalCustomFieldValue`: the stored form of a temporal
/// value — a `date` as offset-preserved local midnight, a `datetime` as a
/// UTC ISO instant — or `None` when invalid.
pub fn normalize_temporal_custom_field_value(
    kind: &str,
    value: Option<&str>,
    timezone: Option<&str>,
) -> Option<String> {
    let value = value.filter(|v| !v.is_empty())?;
    let kind = TemporalCustomFieldType::parse(kind)?;
    let timezone = resolve_filter_timezone(timezone);
    match kind {
        TemporalCustomFieldType::Date => {
            let date_part = date_part_of(value);
            if is_real_calendar_date(date_part) {
                to_zoned_day_start_iso(date_part, &timezone).ok()
            } else {
                None
            }
        }
        TemporalCustomFieldType::Datetime => {
            if has_explicit_offset(value) {
                return js::parse_date(value).and_then(js::to_iso_string);
            }
            if is_valid_datetime_value(value) {
                filter_value_to_utc_iso(value, &timezone).ok()
            } else {
                None
            }
        }
    }
}

/// `formatCustomFieldValueInTimeZone`: a stored temporal value for
/// display (`yyyy-MM-dd` / `yyyy-MM-dd HH:mm:ss` in `timezone`).
pub fn format_custom_field_value_in_time_zone(
    kind: &str,
    value: Option<&str>,
    timezone: &str,
) -> String {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return String::new();
    };
    match TemporalCustomFieldType::parse(kind) {
        None => value.to_owned(),
        Some(TemporalCustomFieldType::Date) => date_part_of(value).to_owned(),
        Some(TemporalCustomFieldType::Datetime) => {
            format_with_fallback(DateArg::Text(value), Some(timezone), DATE_TIME_FORMAT)
        }
    }
}

/// `resolveTemporalCustomFieldFormValue`.
pub fn resolve_temporal_custom_field_form_value(kind: &str, value: &str) -> String {
    if kind == "date" {
        date_part_of(value).to_owned()
    } else {
        value.to_owned()
    }
}

string_enum! {
    /// `TemporalCustomFieldSaveFormat`.
    pub enum TemporalCustomFieldSaveFormat {
        Formatted = "formatted",
        Iso = "iso",
    }
}

/// `resolveTemporalCustomFieldSaveFormat`.
pub fn resolve_temporal_custom_field_save_format(kind: &str) -> TemporalCustomFieldSaveFormat {
    match TemporalCustomFieldType::parse(kind) {
        Some(TemporalCustomFieldType::Datetime) => TemporalCustomFieldSaveFormat::Iso,
        _ => TemporalCustomFieldSaveFormat::Formatted,
    }
}

/// `currentTemporalLiteral`: "now" as the naive literal a type stores.
pub fn current_temporal_literal(
    kind: TemporalCustomFieldType,
    timezone: Option<&str>,
    now_ms: i64,
) -> String {
    let pattern = match kind {
        TemporalCustomFieldType::Date => DATE_FORMAT,
        TemporalCustomFieldType::Datetime => DATE_TIME_FORMAT,
    };
    tz::format_in_time_zone(
        DateArg::Ms(now_ms),
        &resolve_filter_timezone(timezone),
        pattern,
    )
    .unwrap_or_default()
}
