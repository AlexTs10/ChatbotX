//! `field-value-calculator/date-expression.ts`: `now + 2 days - 3h`.

use std::sync::OnceLock;

use regex::Regex;

use crate::js;
use crate::utils::datetime::{TemporalCustomFieldType, normalize_temporal_custom_field_value};
use crate::utils::recurrence::{local_date_of, resolve_zoned_instant};
use crate::utils::temporal_input::parse_loose_temporal_value;
use crate::utils::tz::{self, DateArg};

string_enum! {
    /// `DateUnit`.
    pub enum DateUnit {
        Minute = "minute",
        Hour = "hour",
        Day = "day",
        Week = "week",
        Month = "month",
        Year = "year",
    }
}

/// `DATE_EXPRESSION_LIMITS`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateExpressionLimits {
    pub max_terms: usize,
    pub max_magnitude: f64,
}

/// `DATE_EXPRESSION_LIMITS`.
pub const DATE_EXPRESSION_LIMITS: DateExpressionLimits = DateExpressionLimits {
    max_terms: 10,
    max_magnitude: 100_000.0,
};

const MS_PER_MINUTE: f64 = 60_000.0;
const MS_PER_HOUR: f64 = 3_600_000.0;

/// The clock an expression reads `now` and its zone from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateClock {
    pub now_ms: i64,
    pub time_zone: String,
}

fn is_representable(ms: i64) -> bool {
    if ms.abs() > js::MAX_TIME_MS {
        return false;
    }
    let year = js::utc_fields(ms).year;
    (1..=9999).contains(&year)
}

fn add_wall_clock(base: i64, time_zone: &str, days: i64, months: i64) -> Option<i64> {
    let wall = tz::format_in_time_zone(DateArg::Ms(base), time_zone, "yyyy-MM-dd'T'HH:mm").ok()?;
    let (local_date, local_time) = wall.split_once('T')?;
    let mut parts = local_date.split('-').map(|p| p.parse::<i64>().unwrap_or(0));
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let first_of_month = js::utc_fields(js::date_utc(year, month - 1 + months, 1, 0, 0, 0, 0));
    let days_in_month = i64::from(js::days_in_month(first_of_month.year, first_of_month.month));
    let shifted = js::date_utc(
        first_of_month.year,
        i64::from(first_of_month.month) - 1,
        day.min(days_in_month) + days,
        0,
        0,
        0,
        0,
    );
    if !is_representable(shifted) {
        return None;
    }
    let sub_minute = base.rem_euclid(60_000);
    let date = js::to_iso_string(shifted)?;
    Some(resolve_zoned_instant(&date[..10], local_time, time_zone) + sub_minute)
}

/// `addDateUnit`: minutes and hours move the instant; days, weeks, months
/// and years move the wall clock. `None` outside years 1–9999.
pub fn add_date_unit(base_ms: i64, unit: DateUnit, amount: f64, time_zone: &str) -> Option<i64> {
    let result = match unit {
        DateUnit::Minute => Some(base_ms + js::math_round(amount * MS_PER_MINUTE) as i64),
        DateUnit::Hour => Some(base_ms + js::math_round(amount * MS_PER_HOUR) as i64),
        DateUnit::Day => add_wall_clock(base_ms, time_zone, amount as i64, 0),
        DateUnit::Week => add_wall_clock(base_ms, time_zone, amount as i64 * 7, 0),
        DateUnit::Month => add_wall_clock(base_ms, time_zone, 0, amount as i64),
        DateUnit::Year => add_wall_clock(base_ms, time_zone, 0, amount as i64 * 12),
    }?;
    is_representable(result).then_some(result)
}

fn unit_by_alias(alias: &str) -> Option<DateUnit> {
    Some(match alias {
        "min" | "minute" | "minutes" => DateUnit::Minute,
        "h" | "hour" | "hours" => DateUnit::Hour,
        "d" | "day" | "days" => DateUnit::Day,
        "w" | "week" | "weeks" => DateUnit::Week,
        "mo" | "month" | "months" => DateUnit::Month,
        "y" | "year" | "years" => DateUnit::Year,
        _ => return None,
    })
}

fn resolve_base(token: &str, clock: &DateClock) -> Option<i64> {
    let keyword = token.to_lowercase();
    if keyword == "now" {
        return Some(clock.now_ms);
    }
    if keyword == "today" {
        return Some(resolve_zoned_instant(
            &local_date_of(clock.now_ms, &clock.time_zone),
            "00:00",
            &clock.time_zone,
        ));
    }
    let stored =
        normalize_temporal_custom_field_value("datetime", Some(token), Some(&clock.time_zone))
            .or_else(|| {
                let loose = parse_loose_temporal_value(
                    TemporalCustomFieldType::Datetime,
                    token,
                    Some(&clock.time_zone),
                    clock.now_ms,
                );
                normalize_temporal_custom_field_value(
                    "datetime",
                    loose.as_deref(),
                    Some(&clock.time_zone),
                )
            })?;
    js::parse_date(&stored)
}

/// `evaluateDateExpression`: the instant the expression names, or `None`.
pub fn evaluate_date_expression(expression: &str, clock: &DateClock) -> Option<i64> {
    static TERM: OnceLock<Regex> = OnceLock::new();
    let term = TERM
        .get_or_init(|| Regex::new(r"(?i)^\s+([+-])\s*([0-9]+)\s*([a-z]+)").expect("valid regex"));
    let source = js::trim(expression);
    let base_token = source
        .split(|c: char| c.is_whitespace())
        .next()
        .filter(|t| !t.is_empty())?;
    let mut result = resolve_base(base_token, clock);
    let mut pos = base_token.len();
    let mut terms = 0;
    while let Some(current) = result {
        if pos >= source.len() {
            break;
        }
        let caps = term.captures(&source[pos..])?;
        let unit = unit_by_alias(&caps[3].to_lowercase())?;
        let magnitude: f64 = caps[2].parse().ok()?;
        terms += 1;
        if magnitude > DATE_EXPRESSION_LIMITS.max_magnitude
            || terms > DATE_EXPRESSION_LIMITS.max_terms
        {
            return None;
        }
        pos += caps.get(0).map_or(0, |m| m.end());
        let amount = if &caps[1] == "-" {
            -magnitude
        } else {
            magnitude
        };
        result = add_date_unit(current, unit, amount, &clock.time_zone);
    }
    result
}
