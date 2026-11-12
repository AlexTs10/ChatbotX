//! Port of `queries/date-bucket.ts`. The zone name is always a bind value
//! (`tz_resolve(?)`), never SQL text.

use rusqlite::types::Value;

use crate::repo::sql::Sql;

/// `zonedDateKey(column, timezone)`: `YYYY-MM-DD` of the unix-ms `column_sql` in `timezone`.
pub fn zoned_date_key(column_sql: &str, timezone: &str) -> Sql {
    let f = crate::queries::zoned_date_key(column_sql, timezone);
    Sql::new(f.sql, f.params.into_iter().map(Value::Text).collect())
}

/// Bucket granularity of `zonedBucketKey` (`OccurrenceBucketGranularity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Week,
    Month,
}

/// `zonedBucketKey(column, granularity, timezone)`: the `YYYY-MM-DD` of the start of the
/// day / ISO week (Monday) / month of `column_sql` in `timezone`.
pub fn zoned_bucket_key(column_sql: &str, granularity: Granularity, timezone: &str) -> Sql {
    let wall = format!("tz_wall_ms({column_sql}, tz_resolve(?)) / 1000.0");
    let text = match granularity {
        Granularity::Day => format!("strftime('%Y-%m-%d', {wall}, 'unixepoch')"),
        // date_trunc('week') = the Monday of the ISO week.
        Granularity::Week => format!(
            "strftime('%Y-%m-%d', {wall}, 'unixepoch', '-' || ((CAST(strftime('%w', {wall}, 'unixepoch') AS INTEGER) + 6) % 7) || ' days')"
        ),
        Granularity::Month => format!("strftime('%Y-%m-01', {wall}, 'unixepoch')"),
    };
    let binds = if granularity == Granularity::Week { 2 } else { 1 };
    Sql::new(text, vec![Value::Text(timezone.to_owned()); binds])
}

/// `zonedHourKey(column, timezone)`: `YYYY-MM-DDTHH` of `column_sql` in `timezone`.
pub fn zoned_hour_key(column_sql: &str, timezone: &str) -> Sql {
    Sql::bind(
        format!("strftime('%Y-%m-%dT%H', tz_wall_ms({column_sql}, tz_resolve("),
        timezone.to_owned(),
    )
    .closed(")) / 1000.0, 'unixepoch')")
}

