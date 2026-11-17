//! `packages/analytics`: the dashboard analytics read side (Q.D11 slice 8).
//!
//! TimescaleDB `time_bucket(... AT TIME ZONE tz AT TIME ZONE 'UTC')` is the
//! SQLite function `tz_wall_ms` (the local wall clock written as UTC) cut to
//! a `YYYY-MM-DD` / `YYYY-MM` key; the hourly continuous aggregates are not
//! read (`shouldUseCagg` is always false: every read takes the raw event
//! path, which is what the caggs summarize).

pub mod bot_messages;
pub mod campaigns;
pub mod comment_automation;
pub mod contacts;
pub mod conversations;
pub mod flows;
pub mod links;
pub mod mac;
pub mod messages;

use chatbotx_db::functions::{from_wall_ms, tz_resolve, wall_ms};
use chatbotx_db::rusqlite::types::Value as Sql;
use chrono::{Datelike, NaiveDate};
use chrono_tz::Tz;

const DAY_MS: i64 = 86_400_000;

/// `FROM_FLOOR` (2020-01-01T00:00:00.000Z).
pub const FROM_FLOOR_MS: i64 = 1_577_836_800_000;

/// `normalizeAnalyticsFilters`: deduplicated, sorted `channels` / `inboxIds`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub channels: Vec<String>,
    pub inbox_ids: Vec<i64>,
}

impl Filter {
    pub fn new(mut channels: Vec<String>, mut inbox_ids: Vec<i64>) -> Filter {
        channels.sort();
        channels.dedup();
        inbox_ids.sort_unstable();
        inbox_ids.dedup();
        Filter {
            channels,
            inbox_ids,
        }
    }

    /// `hasAnalyticsFilter`.
    pub fn is_set(&self) -> bool {
        !self.channels.is_empty() || !self.inbox_ids.is_empty()
    }

    /// `buildEventFilter(filter, { channel, inboxId })`: ` AND <channel> IN
    /// (...) AND <inbox> IN (...)`, binds appended.
    pub fn sql(&self, channel: &str, inbox: &str, binds: &mut Vec<Sql>) -> String {
        let mut sql = String::new();
        if !self.channels.is_empty() {
            sql.push_str(&format!(
                " AND {channel} IN ({})",
                marks(self.channels.len())
            ));
            binds.extend(self.channels.iter().cloned().map(Sql::Text));
        }
        if !self.inbox_ids.is_empty() {
            sql.push_str(&format!(
                " AND {inbox} IN ({})",
                marks(self.inbox_ids.len())
            ));
            binds.extend(self.inbox_ids.iter().copied().map(Sql::Integer));
        }
        sql
    }

    /// [`Filter::sql`] on the event tables' own `"channel"` / `"inboxId"`.
    pub fn event_sql(&self, binds: &mut Vec<Sql>) -> String {
        self.sql(r#""channel""#, r#""inboxId""#, binds)
    }
}

/// `?, ?, ...`.
pub fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// `FilteredTimeRangeQuery`. `from` / `to` are `None` for an Invalid Date
/// (`new Date("junk")`): such a range matches no row and has no buckets.
#[derive(Debug, Clone)]
pub struct TimeRange {
    pub workspace_id: i64,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// `resolvedTimezone(timezone)`.
    pub tz: Tz,
    pub filter: Filter,
}

impl TimeRange {
    /// `timeRangeQuerySchema` transforms: `from` clamped to [`FROM_FLOOR_MS`],
    /// the timezone resolved to one the tz database knows (else UTC).
    pub fn new(
        workspace_id: i64,
        from: Option<i64>,
        to: Option<i64>,
        timezone: &str,
        filter: Filter,
    ) -> TimeRange {
        TimeRange {
            workspace_id,
            from: from.map(|ms| ms.max(FROM_FLOOR_MS)),
            to,
            tz: zone(timezone),
            filter,
        }
    }

    /// The valid `(from, to)`.
    pub fn bounds(&self) -> Option<(i64, i64)> {
        Some((self.from?, self.to?))
    }

    /// `resolveRangeGranularity`: monthly past 60 full days.
    pub fn granularity(&self) -> Granularity {
        match self.bounds() {
            Some((from, to)) if (to - from) / DAY_MS > 60 => Granularity::Month,
            _ => Granularity::Day,
        }
    }
}

/// `resolvedTimezone(name)`: a zone the tz database knows, else UTC.
pub fn zone(name: &str) -> Tz {
    tz_resolve(Some(name)).parse().unwrap_or(Tz::UTC)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Month,
}

impl Granularity {
    /// The SQL expression of the local bucket key of `column`. The zone is
    /// inlined: a tz database name (`[A-Za-z0-9/_+-]`, no quote).
    pub fn key_sql(self, column: &str, tz: Tz) -> String {
        let format = match self {
            Granularity::Day => "%Y-%m-%d",
            Granularity::Month => "%Y-%m",
        };
        format!(
            "strftime('{format}', tz_wall_ms({column}, '{}') / 1000, 'unixepoch')",
            tz.name()
        )
    }

    /// The local key of an instant.
    pub fn key(self, ms: i64, tz: Tz) -> Option<String> {
        let day = local_date(ms, tz)?;
        Some(match self {
            Granularity::Day => day.format("%Y-%m-%d").to_string(),
            Granularity::Month => day.format("%Y-%m").to_string(),
        })
    }

    /// The instant the local bucket of `ms` starts (`date_trunc(unit, ts AT
    /// TIME ZONE tz) AT TIME ZONE tz`).
    pub fn start_of(self, ms: i64, tz: Tz) -> Option<i64> {
        let day = local_date(ms, tz)?;
        let first = match self {
            Granularity::Day => day,
            Granularity::Month => day.with_day(1)?,
        };
        let wall = first.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis();
        from_wall_ms(wall, tz).ok()
    }

    /// `iterateTzDays` / `iterateTzMonths`: every local bucket key from the
    /// one of `from` to the one of `to`, with its `date` (the key's first day
    /// at 00:00 UTC, ISO).
    pub fn series(self, range: &TimeRange) -> Vec<Bucket> {
        let Some((from, to)) = range.bounds() else {
            return Vec::new();
        };
        let (Some(start), Some(end)) = (local_date(from, range.tz), local_date(to, range.tz))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut day = match self {
            Granularity::Day => start,
            Granularity::Month => start.with_day(1).unwrap_or(start),
        };
        while day <= end {
            let key = match self {
                Granularity::Day => day.format("%Y-%m-%d").to_string(),
                Granularity::Month => day.format("%Y-%m").to_string(),
            };
            out.push(Bucket {
                date: format!("{}T00:00:00.000Z", day.format("%Y-%m-%d")),
                key,
            });
            day = match self {
                Granularity::Day => match day.succ_opt() {
                    Some(next) => next,
                    None => break,
                },
                Granularity::Month => match day.checked_add_months(chrono::Months::new(1)) {
                    Some(next) => next,
                    None => break,
                },
            };
        }
        out
    }
}

/// One series bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// `YYYY-MM-DD` / `YYYY-MM`.
    pub key: String,
    /// ISO date of the bucket (`YYYY-MM-DDT00:00:00.000Z`).
    pub date: String,
}

fn local_date(ms: i64, tz: Tz) -> Option<NaiveDate> {
    let wall = wall_ms(ms, tz).ok()?;
    chrono::DateTime::from_timestamp_millis(wall).map(|d| d.date_naive())
}

/// `{ date, count }` of a contact count series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountPoint {
    pub date: String,
    pub count: i64,
}
