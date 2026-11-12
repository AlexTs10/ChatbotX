//! `packages/utils/src/recurrence.ts`: "every N days / weeks / months" at a
//! wall-clock time in an IANA zone, computed with direct period arithmetic.
//! Instants are epoch ms.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::datetime::is_real_calendar_date;
use super::timezone::is_valid_timezone;
use super::tz::Zone;
use crate::js;

string_enum! {
    /// `recurrenceFrequencies`.
    pub enum RecurrenceFrequency {
        Daily = "daily",
        Weekly = "weekly",
        Monthly = "monthly",
    }
}

/// `RECURRENCE_MONTH_DAY_LAST`.
pub const RECURRENCE_MONTH_DAY_LAST: &str = "last";
/// `RECURRENCE_MAX_OCCURRENCES`.
pub const RECURRENCE_MAX_OCCURRENCES: i64 = 10_000;
/// `RECURRENCE_MAX_EXCLUDED_DATES`.
pub const RECURRENCE_MAX_EXCLUDED_DATES: usize = 366;
/// `RECURRENCE_WEEK_STARTS_ON`.
pub const RECURRENCE_WEEK_STARTS_ON: &str = "monday";

/// `RECURRENCE_MAX_INTERVAL[frequency]`.
pub fn recurrence_max_interval(frequency: RecurrenceFrequency) -> i64 {
    match frequency {
        RecurrenceFrequency::Daily => 365,
        RecurrenceFrequency::Weekly => 52,
        RecurrenceFrequency::Monthly => 24,
    }
}

const MAX_PERIOD_STEPS: usize = 10_000;
const MS_PER_MINUTE: i64 = 60_000;
const MS_PER_DAY: i64 = 86_400_000;
const DAYS_PER_WEEK: i64 = 7;

/// `monthDay`: a day 1–31 or `"last"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthDay {
    Day(i64),
    Last,
}

/// `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurrenceEnd {
    Never,
    OnDate(String),
    AfterOccurrences(i64),
}

/// `RecurrenceRule`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceRule {
    pub frequency: RecurrenceFrequency,
    pub interval: i64,
    /// Weekly only: 0 = Sunday … 6 = Saturday.
    pub weekdays: Option<Vec<i64>>,
    /// Monthly only.
    pub month_day: Option<MonthDay>,
    /// `HH:mm` in `timezone`.
    pub time_of_day: String,
    pub timezone: String,
    /// First eligible local date (`YYYY-MM-DD`).
    pub starts_on: String,
    /// Local dates that must not send.
    pub exclude_dates: Option<Vec<String>>,
    pub end: RecurrenceEnd,
}

/// One schema issue: the dotted path and the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleIssue {
    pub path: String,
    pub message: String,
}

fn issue(issues: &mut Vec<RuleIssue>, path: &str, message: impl Into<String>) {
    issues.push(RuleIssue {
        path: path.to_owned(),
        message: message.into(),
    });
}

fn local_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[0-9]{4}-[0-9]{2}-[0-9]{2}$").expect("valid regex"))
}

fn int_of(value: &Value) -> Option<i64> {
    let n = value.as_f64()?;
    (n.fract() == 0.0 && n.is_finite()).then_some(n as i64)
}

/// `recurrenceRuleShape.safeParse`: the unrefined shape (for responses).
pub fn parse_recurrence_rule_shape(value: &Value) -> Result<RecurrenceRule, Vec<RuleIssue>> {
    let mut issues = Vec::new();
    let Some(map) = value.as_object() else {
        issue(&mut issues, "", "Invalid input: expected object");
        return Err(issues);
    };
    let frequency = map
        .get("frequency")
        .and_then(Value::as_str)
        .and_then(RecurrenceFrequency::parse);
    if frequency.is_none() {
        issue(&mut issues, "frequency", "Invalid option");
    }
    let interval = map.get("interval").and_then(int_of).filter(|i| *i >= 1);
    if interval.is_none() {
        issue(&mut issues, "interval", "Invalid input: expected int >= 1");
    }
    let weekdays = match map.get("weekdays") {
        None => Some(None),
        Some(Value::Array(items)) if !items.is_empty() => {
            let days: Option<Vec<i64>> = items
                .iter()
                .map(|item| int_of(item).filter(|d| (0..=6).contains(d)))
                .collect();
            if days.is_none() {
                issue(&mut issues, "weekdays", "Invalid weekday");
            }
            days.map(Some)
        }
        Some(_) => {
            issue(
                &mut issues,
                "weekdays",
                "Too small: expected array to have >=1 items",
            );
            None
        }
    };
    let month_day = match map.get("monthDay") {
        None => Some(None),
        Some(Value::String(s)) if s == RECURRENCE_MONTH_DAY_LAST => Some(Some(MonthDay::Last)),
        Some(v) => match int_of(v).filter(|d| (1..=31).contains(d)) {
            Some(d) => Some(Some(MonthDay::Day(d))),
            None => {
                issue(&mut issues, "monthDay", "Invalid input");
                None
            }
        },
    };
    static TIME_RE: OnceLock<Regex> = OnceLock::new();
    let time_re =
        TIME_RE.get_or_init(|| Regex::new("^([01][0-9]|2[0-3]):[0-5][0-9]$").expect("valid regex"));
    let time_of_day = match map.get("timeOfDay").and_then(Value::as_str) {
        Some(t) if time_re.is_match(t) => Some(t.to_owned()),
        _ => {
            issue(
                &mut issues,
                "timeOfDay",
                "Invalid string: must match pattern",
            );
            None
        }
    };
    let timezone = match map.get("timezone").and_then(Value::as_str) {
        Some(t) => Some(t.to_owned()),
        None => {
            issue(&mut issues, "timezone", "Invalid input: expected string");
            None
        }
    };
    let starts_on = match map.get("startsOn").and_then(Value::as_str) {
        Some(d) if local_date_re().is_match(d) => Some(d.to_owned()),
        _ => {
            issue(
                &mut issues,
                "startsOn",
                "Invalid string: must match pattern",
            );
            None
        }
    };
    let exclude_dates = match map.get("excludeDates") {
        None => Some(None),
        Some(Value::Array(items)) => {
            let mut dates = Vec::with_capacity(items.len());
            let mut ok = true;
            for (index, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(d) if local_date_re().is_match(d) => dates.push(d.to_owned()),
                    _ => {
                        issue(
                            &mut issues,
                            &format!("excludeDates.{index}"),
                            "Invalid string: must match pattern",
                        );
                        ok = false;
                    }
                }
            }
            ok.then_some(Some(dates))
        }
        Some(_) => {
            issue(&mut issues, "excludeDates", "Invalid input: expected array");
            None
        }
    };
    let end = match map.get("end").and_then(Value::as_object) {
        Some(end) => match end.get("kind").and_then(Value::as_str) {
            Some("never") => Some(RecurrenceEnd::Never),
            Some("onDate") => match end.get("date").and_then(Value::as_str) {
                Some(d) if local_date_re().is_match(d) => Some(RecurrenceEnd::OnDate(d.to_owned())),
                _ => {
                    issue(
                        &mut issues,
                        "end.date",
                        "Invalid string: must match pattern",
                    );
                    None
                }
            },
            Some("afterOccurrences") => {
                match end
                    .get("count")
                    .and_then(int_of)
                    .filter(|c| (1..=RECURRENCE_MAX_OCCURRENCES).contains(c))
                {
                    Some(c) => Some(RecurrenceEnd::AfterOccurrences(c)),
                    None => {
                        issue(&mut issues, "end.count", "Invalid count");
                        None
                    }
                }
            }
            _ => {
                issue(&mut issues, "end.kind", "Invalid input");
                None
            }
        },
        None => {
            issue(&mut issues, "end", "Invalid input: expected object");
            None
        }
    };
    if !issues.is_empty() {
        return Err(issues);
    }
    match (
        frequency,
        interval,
        weekdays,
        month_day,
        time_of_day,
        timezone,
        starts_on,
        exclude_dates,
        end,
    ) {
        (
            Some(frequency),
            Some(interval),
            Some(weekdays),
            Some(month_day),
            Some(time_of_day),
            Some(timezone),
            Some(starts_on),
            Some(exclude_dates),
            Some(end),
        ) => Ok(RecurrenceRule {
            frequency,
            interval,
            weekdays,
            month_day,
            time_of_day,
            timezone,
            starts_on,
            exclude_dates,
            end,
        }),
        _ => Err(issues),
    }
}

/// The refinements of `recurrenceRuleSchema` on a shaped rule.
pub fn refine_recurrence_rule(rule: &RecurrenceRule) -> Vec<RuleIssue> {
    let mut issues = Vec::new();
    let max = recurrence_max_interval(rule.frequency);
    if rule.interval > max {
        issue(
            &mut issues,
            "interval",
            format!(
                "Interval must be at most {max} for {} schedules.",
                rule.frequency.as_str()
            ),
        );
    }
    if rule.frequency == RecurrenceFrequency::Weekly
        && rule.weekdays.as_ref().is_none_or(Vec::is_empty)
    {
        issue(
            &mut issues,
            "weekdays",
            "Pick at least one weekday for a weekly schedule.",
        );
    }
    if rule.frequency == RecurrenceFrequency::Monthly && rule.month_day.is_none() {
        issue(
            &mut issues,
            "monthDay",
            "Pick the day of the month for a monthly schedule.",
        );
    }
    if !is_valid_timezone(&rule.timezone) {
        issue(&mut issues, "timezone", "Unknown IANA timezone.");
    }
    if !is_real_calendar_date(&rule.starts_on) {
        issue(&mut issues, "startsOn", "Not a real calendar date.");
    }
    let excluded = rule.exclude_dates.as_deref().unwrap_or(&[]);
    if excluded.len() > RECURRENCE_MAX_EXCLUDED_DATES {
        issue(
            &mut issues,
            "excludeDates",
            format!("Skip at most {RECURRENCE_MAX_EXCLUDED_DATES} dates."),
        );
    }
    for (index, date) in excluded.iter().enumerate() {
        if !is_real_calendar_date(date) {
            issue(
                &mut issues,
                &format!("excludeDates.{index}"),
                "Not a real calendar date.",
            );
        }
    }
    if let RecurrenceEnd::OnDate(date) = &rule.end {
        if !is_real_calendar_date(date) {
            issue(&mut issues, "end.date", "Not a real calendar date.");
        } else if date.as_str() < rule.starts_on.as_str() {
            issue(
                &mut issues,
                "end.date",
                "The end date cannot be before the start date.",
            );
        }
    }
    issues
}

/// `recurrenceRuleSchema.safeParse`: shape, then refinements.
pub fn parse_recurrence_rule(value: &Value) -> Result<RecurrenceRule, Vec<RuleIssue>> {
    let rule = parse_recurrence_rule_shape(value)?;
    let issues = refine_recurrence_rule(&rule);
    if issues.is_empty() {
        Ok(rule)
    } else {
        Err(issues)
    }
}

// ---------------------------------------------------------------------------
// Local-date arithmetic
// ---------------------------------------------------------------------------

fn parse_local_date(local_date: &str) -> (i64, i64, i64) {
    let mut parts = local_date.split('-').map(|p| p.parse::<i64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(1),
        parts.next().unwrap_or(1),
    )
}

fn to_day_number(local_date: &str) -> i64 {
    let (year, month, day) = parse_local_date(local_date);
    js::days_from_civil(year, month - 1, day)
}

fn from_day_number(day_number: i64) -> String {
    let (y, m, d) = js::civil_from_days(day_number);
    format!("{y:04}-{m:02}-{d:02}")
}

fn weekday_of_day_number(day_number: i64) -> i64 {
    (day_number + 4).rem_euclid(7)
}

fn iso_week_start(day_number: i64) -> i64 {
    day_number - (weekday_of_day_number(day_number) + 6) % DAYS_PER_WEEK
}

fn month_index_of(local_date: &str) -> i64 {
    let (year, month, _) = parse_local_date(local_date);
    year * 12 + (month - 1)
}

fn period_index_of(rule: &RecurrenceRule, local_date: &str) -> i64 {
    let steps = match rule.frequency {
        RecurrenceFrequency::Daily => to_day_number(local_date) - to_day_number(&rule.starts_on),
        RecurrenceFrequency::Weekly => {
            (iso_week_start(to_day_number(local_date))
                - iso_week_start(to_day_number(&rule.starts_on)))
                / DAYS_PER_WEEK
        }
        RecurrenceFrequency::Monthly => {
            month_index_of(local_date) - month_index_of(&rule.starts_on)
        }
    };
    steps.div_euclid(rule.interval.max(1)).max(0)
}

fn dates_of_period(rule: &RecurrenceRule, index: i64) -> Vec<String> {
    let start_day = to_day_number(&rule.starts_on);
    let offset = index * rule.interval;
    let days: Vec<i64> = match rule.frequency {
        RecurrenceFrequency::Daily => vec![start_day + offset],
        RecurrenceFrequency::Weekly => {
            let week_start = iso_week_start(start_day) + offset * DAYS_PER_WEEK;
            let unique: BTreeSet<i64> = rule.weekdays.iter().flatten().copied().collect();
            let mut days: Vec<i64> = unique
                .into_iter()
                .map(|weekday| week_start + (weekday + 6) % DAYS_PER_WEEK)
                .collect();
            days.sort_unstable();
            days
        }
        RecurrenceFrequency::Monthly => {
            let month_index = month_index_of(&rule.starts_on) + offset;
            let year = month_index.div_euclid(12);
            let month = month_index.rem_euclid(12) + 1;
            let last_day = i64::from(js::days_in_month(year, month as u32));
            let day = match rule.month_day.unwrap_or(MonthDay::Last) {
                MonthDay::Last => last_day,
                MonthDay::Day(d) => d.min(last_day),
            };
            vec![js::days_from_civil(year, month - 1, day)]
        }
    };
    days.into_iter()
        .filter(|d| *d >= start_day)
        .map(from_day_number)
        .collect()
}

fn last_eligible_date(rule: &RecurrenceRule) -> Option<&str> {
    match &rule.end {
        RecurrenceEnd::OnDate(date) => Some(date),
        _ => None,
    }
}

fn is_exhausted_by_count(rule: &RecurrenceRule, occurrence_count: i64) -> bool {
    matches!(rule.end, RecurrenceEnd::AfterOccurrences(count) if occurrence_count >= count)
}

fn zone_of(timezone: &str) -> Zone {
    Zone::parse(timezone).unwrap_or_else(Zone::utc)
}

/// `offsetAt`: the zone offset at an instant, at minute precision.
fn offset_at(instant_ms: i64, zone: &Zone) -> i64 {
    let minute_ms = instant_ms.div_euclid(MS_PER_MINUTE) * MS_PER_MINUTE;
    let wall = minute_ms + zone.offset_ms(minute_ms);
    wall.div_euclid(MS_PER_MINUTE) * MS_PER_MINUTE - minute_ms
}

/// `resolveZonedInstant`: a local date + `HH:mm` in a zone as an instant.
/// An ambiguous time takes the EARLIER instant; a nonexistent one moves
/// FORWARD by the real size of the gap (a skipped whole day included).
pub fn resolve_zoned_instant(local_date: &str, time_of_day: &str, timezone: &str) -> i64 {
    let zone = zone_of(timezone);
    let mut time = time_of_day
        .split(':')
        .map(|p| p.parse::<i64>().unwrap_or(0));
    let (hours, minutes) = (time.next().unwrap_or(0), time.next().unwrap_or(0));
    let (year, month, day) = parse_local_date(local_date);
    let wall_clock_as_utc = js::date_utc(year, month - 1, day, hours, minutes, 0, 0);
    let offset_before = offset_at(wall_clock_as_utc - MS_PER_DAY, &zone);
    let offset_after = offset_at(wall_clock_as_utc + MS_PER_DAY, &zone);
    let valid: Vec<i64> = [offset_before, offset_after]
        .into_iter()
        .map(|offset| (offset, wall_clock_as_utc - offset))
        .filter(|(offset, instant)| offset_at(*instant, &zone) == *offset)
        .map(|(_, instant)| instant)
        .collect();
    valid
        .into_iter()
        .min()
        .unwrap_or(wall_clock_as_utc - offset_before)
}

/// `localDateOf`: `YYYY-MM-DD` of an instant in a zone.
pub fn local_date_of(instant_ms: i64, timezone: &str) -> String {
    let f = zone_of(timezone).wall_clock(instant_ms);
    format!("{:04}-{:02}-{:02}", f.year, f.month, f.day)
}

/// `Recurrence search exceeded its step limit` (a bug, never a normal end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepLimitExceeded;

impl fmt::Display for StepLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Recurrence search exceeded its step limit")
    }
}

impl std::error::Error for StepLimitExceeded {}

/// `computeNextOccurrence`: the first slot strictly after `after`, or
/// `None` once the series is over.
pub fn compute_next_occurrence(
    rule: &RecurrenceRule,
    after_ms: i64,
    occurrence_count: i64,
) -> Result<Option<i64>, StepLimitExceeded> {
    if is_exhausted_by_count(rule, occurrence_count) {
        return Ok(None);
    }
    let end_date = last_eligible_date(rule);
    let mut period = (period_index_of(rule, &local_date_of(after_ms, &rule.timezone)) - 1).max(0);
    for _ in 0..MAX_PERIOD_STEPS {
        for date in dates_of_period(rule, period) {
            if end_date.is_some_and(|end| date.as_str() > end) {
                return Ok(None);
            }
            let instant = resolve_zoned_instant(&date, &rule.time_of_day, &rule.timezone);
            if instant > after_ms {
                return Ok(Some(instant));
            }
        }
        period += 1;
    }
    Err(StepLimitExceeded)
}

/// `computeLatestOccurrence`: the latest slot at or before `at_or_before`.
pub fn compute_latest_occurrence(
    rule: &RecurrenceRule,
    at_or_before_ms: i64,
    occurrence_count: i64,
) -> Result<Option<i64>, StepLimitExceeded> {
    if is_exhausted_by_count(rule, occurrence_count) {
        return Ok(None);
    }
    let end_date = last_eligible_date(rule);
    let at_or_before_date = local_date_of(at_or_before_ms, &rule.timezone);
    let anchor = match end_date {
        Some(end) if end < at_or_before_date.as_str() => end.to_owned(),
        _ => at_or_before_date,
    };
    let mut period = period_index_of(rule, &anchor) + 1;
    let mut step = 0;
    while step < MAX_PERIOD_STEPS && period >= 0 {
        let mut dates = dates_of_period(rule, period);
        dates.reverse();
        for date in dates {
            if end_date.is_some_and(|end| date.as_str() > end) {
                continue;
            }
            let instant = resolve_zoned_instant(&date, &rule.time_of_day, &rule.timezone);
            if instant <= at_or_before_ms {
                return Ok(Some(instant));
            }
        }
        step += 1;
        period -= 1;
    }
    if period >= 0 {
        return Err(StepLimitExceeded);
    }
    Ok(None)
}

/// One scheduled slot, flagged when its local date is excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcomingSlot {
    pub at: i64,
    pub date: String,
    pub excluded: bool,
}

/// `isExcludedSlot`.
pub fn is_excluded_slot(rule: &RecurrenceRule, instant_ms: i64) -> bool {
    let date = local_date_of(instant_ms, &rule.timezone);
    rule.exclude_dates
        .as_ref()
        .is_some_and(|dates| dates.contains(&date))
}

/// `findSlotOnDate`: the slot on a local date, or `None`.
pub fn find_slot_on_date(
    rule: &RecurrenceRule,
    date: &str,
    occurrence_count: i64,
) -> Result<Option<i64>, StepLimitExceeded> {
    let day_start = resolve_zoned_instant(date, "00:00", &rule.timezone);
    let next = compute_next_occurrence(rule, day_start - 1, occurrence_count)?;
    Ok(next.filter(|n| local_date_of(*n, &rule.timezone) == date))
}

/// `computeNextSendable`: the first slot at or after `from` that is not
/// skipped (skips never consume `afterOccurrences`).
pub fn compute_next_sendable(
    rule: &RecurrenceRule,
    from_ms: i64,
    occurrence_count: i64,
) -> Result<Option<i64>, StepLimitExceeded> {
    let max_skips = rule.exclude_dates.as_ref().map_or(0, Vec::len);
    let mut slot = Some(from_ms);
    let mut skipped = 0;
    while let Some(current) = slot {
        if skipped > max_skips {
            break;
        }
        if !is_excluded_slot(rule, current) {
            return Ok(Some(current));
        }
        slot = compute_next_occurrence(rule, current, occurrence_count)?;
        skipped += 1;
    }
    Ok(None)
}

fn collect_upcoming_slots(
    rule: &RecurrenceRule,
    after_ms: i64,
    occurrence_count: i64,
    limit: usize,
    include_excluded: bool,
) -> Result<Vec<UpcomingSlot>, StepLimitExceeded> {
    let mut slots = Vec::new();
    let mut after = after_ms;
    let mut count = occurrence_count;
    let max_steps = limit + rule.exclude_dates.as_ref().map_or(0, Vec::len);
    let mut step = 0;
    while step < max_steps && slots.len() < limit {
        let Some(at) = compute_next_occurrence(rule, after, count)? else {
            break;
        };
        after = at;
        let excluded = is_excluded_slot(rule, at);
        if !excluded {
            count += 1;
        }
        if include_excluded || !excluded {
            slots.push(UpcomingSlot {
                at,
                date: local_date_of(at, &rule.timezone),
                excluded,
            });
        }
        step += 1;
    }
    Ok(slots)
}

/// `listUpcomingSlots`: the next `limit` slots, skipped ones flagged.
pub fn list_upcoming_slots(
    rule: &RecurrenceRule,
    after_ms: i64,
    occurrence_count: i64,
    limit: usize,
) -> Result<Vec<UpcomingSlot>, StepLimitExceeded> {
    collect_upcoming_slots(rule, after_ms, occurrence_count, limit, true)
}

/// `listUpcomingOccurrences`: the next `limit` runs that will send.
pub fn list_upcoming_occurrences(
    rule: &RecurrenceRule,
    after_ms: i64,
    occurrence_count: i64,
    limit: usize,
) -> Result<Vec<i64>, StepLimitExceeded> {
    Ok(
        collect_upcoming_slots(rule, after_ms, occurrence_count, limit, false)?
            .into_iter()
            .map(|slot| slot.at)
            .collect(),
    )
}

/// `listOccurrencesBetween`: slots in `(after, through]`, oldest first.
pub fn list_occurrences_between(
    rule: &RecurrenceRule,
    after_ms: i64,
    through_ms: i64,
    occurrence_count: i64,
    limit: usize,
) -> Result<Vec<i64>, StepLimitExceeded> {
    let mut slots = Vec::new();
    let mut after = after_ms;
    while slots.len() < limit {
        match compute_next_occurrence(rule, after, occurrence_count)? {
            Some(next) if next <= through_ms => {
                slots.push(next);
                after = next;
            }
            _ => break,
        }
    }
    Ok(slots)
}

/// `toLocalSlot`: `YYYY-MM-DD HH:mm` in the zone.
pub fn to_local_slot(instant_ms: i64, timezone: &str) -> String {
    let f = zone_of(timezone).wall_clock(instant_ms);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        f.year, f.month, f.day, f.hour, f.minute
    )
}
