//! `pipeline/analytics.service.ts` (`getStats`) with the date-range helpers
//! of `ads-analytics/date-range.ts`. Values are the stored x 10,000 integers
//! (sums are exact); `change` compares major units as the TS did.

use chatbotx_db::Db;
use chatbotx_db::repo::pipeline_activity::{self as activity_repo, PipelineAnalyticsRange};
use chrono::{Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use super::access::{PipelineAccessScope, PipelineAction};
use super::query::workspace_timezone;
use super::service::require_access_in;
use crate::errors::Result;
use crate::tx;

/// `MAX_ADS_ANALYTICS_RANGE_DAYS`.
const MAX_RANGE_DAYS: i64 = 366;
const DEFAULT_RANGE_DAYS_BACK: i64 = 6;
const MAX_TIMEZONE_NAME_LENGTH: usize = 64;

/// `PipelineStatsTotals` (values x 10,000).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatsTotals {
    pub created: i64,
    pub won: i64,
    pub won_value: i64,
    pub lost: i64,
    pub lost_value: i64,
}

/// `change`: percent against the previous period; `None` = no base.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StatsChange {
    pub created: Option<f64>,
    pub won: Option<f64>,
    pub won_value: Option<f64>,
    pub lost: Option<f64>,
    pub lost_value: Option<f64>,
}

/// `PipelineStatsActor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsActor {
    /// `user` / `automation`.
    pub actor_kind: &'static str,
    pub actor_user_id: Option<i64>,
    pub totals: StatsTotals,
}

/// `PipelineStats`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineStats {
    pub from: String,
    pub to: String,
    pub timezone: String,
    pub current: StatsTotals,
    pub previous: StatsTotals,
    pub change: StatsChange,
    pub daily: Vec<(String, StatsTotals)>,
    pub by_actor: Vec<StatsActor>,
}

/// The resolved range (`since` / `until` unix ms, inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyticsRange {
    pub since: i64,
    pub until: i64,
    pub from: String,
    pub to: String,
    pub timezone: String,
}

fn add_transition(
    totals: &mut StatsTotals,
    kind: &str,
    to_status: Option<&str>,
    count: i64,
    value: i64,
) {
    if kind == "created" {
        totals.created += count;
    }
    match to_status {
        Some("won") => {
            totals.won += count;
            totals.won_value += value;
        }
        Some("lost") => {
            totals.lost += count;
            totals.lost_value += value;
        }
        _ => {}
    }
}

/// `percentChange` (ecommerce analytics).
pub fn percent_change(current: f64, previous: f64) -> Option<f64> {
    if previous == 0.0 {
        return if current == 0.0 { Some(0.0) } else { None };
    }
    Some(((current - previous) / previous * 1000.0).round() / 10.0)
}

fn major(stored: i64) -> f64 {
    stored as f64 / 10_000.0
}

/// `resolveTimezone`: an unknown or offset zone is UTC.
fn resolve_timezone(tz: &str) -> (Tz, String) {
    if tz.len() > MAX_TIMEZONE_NAME_LENGTH || tz.starts_with('+') || tz.starts_with('-') {
        return (Tz::UTC, "UTC".to_string());
    }
    match tz.parse::<Tz>() {
        Ok(zone) => (zone, zone.name().to_string()),
        Err(_) => (Tz::UTC, "UTC".to_string()),
    }
}

/// `isValidDateKey`.
fn date_key(key: &str) -> Option<NaiveDate> {
    let bytes = key.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shaped {
        return None;
    }
    let date = NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == key).then_some(date)
}

fn zoned_ms(zone: Tz, local: NaiveDateTime) -> i64 {
    let mut probe = local;
    for _ in 0..4 {
        if let Some(at) = zone.from_local_datetime(&probe).earliest() {
            return at.timestamp_millis();
        }
        // A skipped local time (DST gap) moves forward.
        probe += Duration::hours(1);
    }
    Utc.from_utc_datetime(&local).timestamp_millis()
}

fn day_start(zone: Tz, day: NaiveDate) -> i64 {
    zoned_ms(zone, day.and_hms_opt(0, 0, 0).unwrap_or_default())
}

fn day_end(zone: Tz, day: NaiveDate) -> i64 {
    zoned_ms(
        zone,
        day.and_hms_milli_opt(23, 59, 59, 999).unwrap_or_default(),
    )
}

fn key_of(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// `parseAnalyticsDateRange` at `now_ms`: an invalid key falls back to the
/// last seven days, an inverted range to the default, a range over 366 days
/// keeps its most recent 366 days.
pub fn parse_analytics_date_range(from: &str, to: &str, tz: &str, now_ms: i64) -> AnalyticsRange {
    let (zone, timezone) = resolve_timezone(tz);
    let today = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_default()
        .with_timezone(&zone)
        .date_naive();
    let fallback_from = today - Duration::days(DEFAULT_RANGE_DAYS_BACK);
    let from_day = date_key(from).unwrap_or(fallback_from);
    let to_day = date_key(to).unwrap_or(today);
    let since = day_start(zone, from_day);
    let until = day_end(zone, to_day);
    if since > until {
        return AnalyticsRange {
            since: day_start(zone, fallback_from),
            until: day_end(zone, today),
            from: key_of(fallback_from),
            to: key_of(today),
            timezone,
        };
    }
    if (to_day - from_day).num_days() < MAX_RANGE_DAYS {
        return AnalyticsRange {
            since,
            until,
            from: key_of(from_day),
            to: key_of(to_day),
            timezone,
        };
    }
    let clamped = to_day - Duration::days(MAX_RANGE_DAYS - 1);
    AnalyticsRange {
        since: day_start(zone, clamped),
        until,
        from: key_of(clamped),
        to: key_of(to_day),
        timezone,
    }
}

/// `enumerateDateKeys`.
fn day_keys(from: &str, to: &str) -> Vec<String> {
    let (Some(mut day), Some(end)) = (date_key(from), date_key(to)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while day <= end {
        out.push(key_of(day));
        day += Duration::days(1);
    }
    out
}

/// `getStats`: created / won / lost transitions in the range, by day and
/// by actor, against the previous period of the same length.
pub async fn get_stats(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    from: String,
    to: String,
    timezone: Option<String>,
    scope: PipelineAccessScope,
) -> Result<PipelineStats> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::View,
        )?;
        let tz = match timezone.filter(|zone| !zone.is_empty()) {
            Some(zone) => zone,
            None => workspace_timezone(conn, workspace_id)?.unwrap_or_default(),
        };
        let range = parse_analytics_date_range(&from, &to, &tz, chatbotx_db::repo::now_ms());
        let span = range.until - range.since;
        let current_range = PipelineAnalyticsRange {
            workspace_id,
            pipeline_id,
            from: range.since,
            to: range.until,
        };
        let previous_range = PipelineAnalyticsRange {
            from: range.since - span - 1,
            to: range.since - 1,
            ..current_range
        };
        let daily_rows = activity_repo::daily_series(conn, &current_range, &range.timezone)?;
        let actor_rows = activity_repo::totals_by_actor(conn, &current_range)?;
        let previous_rows = activity_repo::totals_by_actor(conn, &previous_range)?;
        let mut current = StatsTotals::default();
        let mut by_actor: Vec<StatsActor> = Vec::new();
        for row in &actor_rows {
            let to_status = row.to_status.as_deref();
            add_transition(&mut current, &row.kind, to_status, row.count, row.value);
            let (actor_kind, actor_user_id) = if row.is_automation {
                ("automation", None)
            } else {
                ("user", row.actor_user_id)
            };
            let index = match by_actor
                .iter()
                .position(|a| a.actor_kind == actor_kind && a.actor_user_id == actor_user_id)
            {
                Some(index) => index,
                None => {
                    by_actor.push(StatsActor {
                        actor_kind,
                        actor_user_id,
                        totals: StatsTotals::default(),
                    });
                    by_actor.len() - 1
                }
            };
            add_transition(
                &mut by_actor[index].totals,
                &row.kind,
                to_status,
                row.count,
                row.value,
            );
        }
        let mut previous = StatsTotals::default();
        for row in &previous_rows {
            add_transition(
                &mut previous,
                &row.kind,
                row.to_status.as_deref(),
                row.count,
                row.value,
            );
        }
        let mut daily: Vec<(String, StatsTotals)> = day_keys(&range.from, &range.to)
            .into_iter()
            .map(|day| (day, StatsTotals::default()))
            .collect();
        for row in &daily_rows {
            if let Some((_, bucket)) = daily.iter_mut().find(|(day, _)| *day == row.day) {
                add_transition(
                    bucket,
                    &row.kind,
                    row.to_status.as_deref(),
                    row.count,
                    row.value,
                );
            }
        }
        let count = |c: i64| c as f64;
        let change = StatsChange {
            created: percent_change(count(current.created), count(previous.created)),
            won: percent_change(count(current.won), count(previous.won)),
            won_value: percent_change(major(current.won_value), major(previous.won_value)),
            lost: percent_change(count(current.lost), count(previous.lost)),
            lost_value: percent_change(major(current.lost_value), major(previous.lost_value)),
        };
        Ok(PipelineStats {
            from: range.from,
            to: range.to,
            timezone: range.timezone,
            current,
            previous,
            change,
            daily,
            by_actor,
        })
    })
    .await
}
