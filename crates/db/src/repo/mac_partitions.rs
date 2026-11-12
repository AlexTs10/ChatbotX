//! Port of `repositories/mac-partitions.ts` and `repositories/leaderboard-partitions.ts`
//! date/name helpers (P3.3). SQLite has no partitions (DECISIONS P2.2/P2.5): the
//! partition DDL helpers are no-ops that report "nothing created".
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use chrono::{Datelike, TimeZone, Utc};

/// `addUtcMonths`: the first day (00:00 UTC) of the month `months` after `date_ms`'s month.
pub fn add_utc_months(date_ms: i64, months: i32) -> i64 {
    todo!("P3.3")
}

/// `formatMonthlyPartitionName`: `ContactActiveHourly_<YYYY>_<MM>`.
pub fn format_monthly_partition_name(date_ms: i64) -> String {
    todo!("P3.3")
}

/// `formatUtcDate`: `YYYY-MM-01`.
pub fn format_utc_date(date_ms: i64) -> String {
    todo!("P3.3")
}

fn year_month(date_ms: i64) -> (i32, u32) {
    Utc.timestamp_millis_opt(date_ms)
        .single()
        .map_or((1970, 1), |d| (d.year(), d.month()))
}

/// `partitionExists`: no partitions in SQLite.
pub fn partition_exists(_name: &str) -> bool {
    todo!("P3.3")
}

/// `createContactActiveMonthlyPartition`: a no-op (one plain table); never creates.
pub fn create_contact_active_monthly_partition(_year: i32) -> bool {
    todo!("P3.3")
}

/// `createContactActiveHourlyPartition`: a no-op (one plain table); never creates.
pub fn create_contact_active_hourly_partition(_month_start_ms: i64) -> bool {
    todo!("P3.3")
}

/// `formatLeaderboardPartitionName`: `LeaderboardScoreEvent_<YYYY>_<MM>`.
pub fn format_leaderboard_partition_name(month_start_ms: i64) -> String {
    todo!("P3.3")
}

/// `createLeaderboardScoreEventPartition`: a no-op (one plain table); never creates.
pub fn create_leaderboard_score_event_partition(_month_start_ms: i64) -> bool {
    todo!("P3.3")
}

/// `listLeaderboardScoreEventPartitions`: no partitions in SQLite.
pub fn list_leaderboard_score_event_partitions() -> Vec<(String, i64)> {
    todo!("P3.3")
}

/// `countLeaderboardScoreEventDefaultRows`: there is no DEFAULT partition; always 0.
pub fn count_leaderboard_score_event_default_rows() -> i64 {
    todo!("P3.3")
}

/// Replacement of `dropLeaderboardScoreEventPartition` (DECISIONS P2.5: the cron deletes
/// in chunks): deletes up to `limit` score events of the UTC month that starts at
/// `month_start_ms`; returns the deleted count (0 = the month is empty).
pub fn delete_leaderboard_score_event_month_batch(
    conn: &rusqlite::Connection,
    month_start_ms: i64,
    limit: i64,
) -> super::Result<usize> {
    todo!("P3.3")
}
