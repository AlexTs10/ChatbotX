//! Port of `packages/database/__tests__/date-bucket.test.ts` (P3.2b). The TS renders the
//! Postgres SQL of `zonedBucketKey`; here the SQLite expression is evaluated on a migrated
//! temp file (adapted): the unit is fixed per granularity, the zone is a bind value.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::queries::date_bucket::{
    Granularity, occurrence_bucket_granularities, zoned_bucket_key,
};
use chatbotx_db::repo::{self};
use common::fresh_db;
use repo_common::{ms, ok};

/// 2026-11-12T20:00Z is Friday 2026-11-13 03:00 in Asia/Saigon (UTC+7, legacy alias).
async fn bucket(granularity: Granularity) -> String {
    let db = fresh_db().await;
    let at = ms("2026-11-12T20:00:00.000Z");
    let expr = zoned_bucket_key(&at.to_string(), granularity, "Asia/Saigon");
    ok(repo::read(&db, move |conn| {
        let sql = chatbotx_db::repo::sql::Sql::new(format!("SELECT {}", expr.text), expr.params);
        Ok(sql
            .query_map(conn, |row| row.get::<_, String>(0))?
            .remove(0))
    })
    .await)
}

#[tokio::test]
async fn zoned_bucket_key_day_truncates_with_a_fixed_literal_unit() {
    assert_eq!(bucket(Granularity::Day).await, "2026-11-13");
}

#[tokio::test]
async fn zoned_bucket_key_week_truncates_with_a_fixed_literal_unit() {
    assert_eq!(bucket(Granularity::Week).await, "2026-11-09");
}

#[tokio::test]
async fn zoned_bucket_key_month_truncates_with_a_fixed_literal_unit() {
    assert_eq!(bucket(Granularity::Month).await, "2026-11-01");
}

#[test]
fn zoned_bucket_key_the_timezone_travels_as_bound_parameters_never_inlined() {
    let expr = zoned_bucket_key("\"occurrenceAt\"", Granularity::Day, "Asia/Saigon");
    assert!(!expr.text.contains("Saigon"), "{}", expr.text);
    assert!(
        expr.params
            .contains(&chatbotx_db::rusqlite::types::Value::Text(
                "Asia/Saigon".into()
            ))
    );
    // Resolved by the app zone resolver so a legacy alias cannot abort the statement.
    assert!(expr.text.contains("tz_resolve(?)"), "{}", expr.text);
}

#[test]
fn zoned_bucket_key_only_the_three_supported_units_exist() {
    assert_eq!(occurrence_bucket_granularities(), ["day", "week", "month"]);
}
