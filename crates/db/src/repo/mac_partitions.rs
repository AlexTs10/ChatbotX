//! Port of `repositories/mac-partitions.ts` and `repositories/leaderboard-partitions.ts`
//! date/name helpers (P3.3). SQLite has no partitions (DECISIONS P2.2/P2.5): the
//! partition DDL helpers are no-ops that report "nothing created".

use chrono::{Datelike, TimeZone, Utc};

/// `addUtcMonths`: the first day (00:00 UTC) of the month `months` after `date_ms`'s month.
pub fn add_utc_months(date_ms: i64, months: i32) -> i64 {
    let Some(date) = Utc.timestamp_millis_opt(date_ms).single() else {
        return date_ms;
    };
    let total = date.year() * 12 + i32::try_from(date.month0()).unwrap_or(0) + months;
    let (year, month0) = (total.div_euclid(12), total.rem_euclid(12));
    Utc.with_ymd_and_hms(year, u32::try_from(month0).unwrap_or(0) + 1, 1, 0, 0, 0)
        .single()
        .map_or(date_ms, |d| d.timestamp_millis())
}

/// `formatMonthlyPartitionName`: `ContactActiveHourly_<YYYY>_<MM>`.
pub fn format_monthly_partition_name(date_ms: i64) -> String {
    let (year, month) = year_month(date_ms);
    format!("ContactActiveHourly_{year}_{month:02}")
}

/// `formatUtcDate`: `YYYY-MM-01`.
pub fn format_utc_date(date_ms: i64) -> String {
    let (year, month) = year_month(date_ms);
    format!("{year}-{month:02}-01")
}

fn year_month(date_ms: i64) -> (i32, u32) {
    Utc.timestamp_millis_opt(date_ms)
        .single()
        .map_or((1970, 1), |d| (d.year(), d.month()))
}

/// `partitionExists`: no partitions in SQLite.
pub fn partition_exists(_name: &str) -> bool {
    false
}

/// `createContactActiveMonthlyPartition`: a no-op (one plain table); never creates.
pub fn create_contact_active_monthly_partition(_year: i32) -> bool {
    false
}

/// `createContactActiveHourlyPartition`: a no-op (one plain table); never creates.
pub fn create_contact_active_hourly_partition(_month_start_ms: i64) -> bool {
    false
}

/// `formatLeaderboardPartitionName`: `LeaderboardScoreEvent_<YYYY>_<MM>`.
pub fn format_leaderboard_partition_name(month_start_ms: i64) -> String {
    let (year, month) = year_month(month_start_ms);
    format!("LeaderboardScoreEvent_{year}_{month:02}")
}

/// `createLeaderboardScoreEventPartition`: a no-op (one plain table); never creates.
pub fn create_leaderboard_score_event_partition(_month_start_ms: i64) -> bool {
    false
}

/// `listLeaderboardScoreEventPartitions`: no partitions in SQLite.
pub fn list_leaderboard_score_event_partitions() -> Vec<(String, i64)> {
    Vec::new()
}

/// `countLeaderboardScoreEventDefaultRows`: there is no DEFAULT partition; always 0.
pub fn count_leaderboard_score_event_default_rows() -> i64 {
    0
}

/// Replacement of `dropLeaderboardScoreEventPartition` (DECISIONS P2.5: the cron deletes
/// in chunks): deletes up to `limit` score events of the UTC month that starts at
/// `month_start_ms`; returns the deleted count (0 = the month is empty).
pub fn delete_leaderboard_score_event_month_batch(
    conn: &rusqlite::Connection,
    month_start_ms: i64,
    limit: i64,
) -> super::Result<usize> {
    let next = add_utc_months(month_start_ms, 1);
    Ok(conn.execute(
        "DELETE FROM \"LeaderboardScoreEvent\" WHERE rowid IN (SELECT rowid FROM \"LeaderboardScoreEvent\" \
         WHERE \"createdAt\" >= ?1 AND \"createdAt\" < ?2 LIMIT ?3)",
        rusqlite::params![month_start_ms, next, limit],
    )?)
}
