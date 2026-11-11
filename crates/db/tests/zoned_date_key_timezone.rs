//! Port of `packages/database/__tests__/integration/zoned-date-key-timezone.test.ts`
//! (P2.3 spec section 5.2, rows 15.1-15.5). Class C: runs the app SQL functions
//! `tz_resolve` / `tz_wall_ms` (spec 3.5) on a real connection.

mod common;

use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::Db;
use common::{fresh_db, query_text};

/// 2026-09-08T17:30:00Z: 00:30 the next morning in Vietnam.
const AFTER_MIDNIGHT_IN_VIETNAM: i64 = 1_788_888_600_000;
/// 2026-09-08T03:30:00Z.
const EARLY_UTC_MORNING: i64 = 1_788_838_200_000;

/// The day key SQLite produces for `at_ms` when asked for `timezone`
/// (port of `to_char($at AT TIME ZONE <resolvedTimezone>, 'YYYY-MM-DD')`).
async fn day_key_for(db: &Db, timezone: &str, at_ms: i64) -> String {
    query_text(
        db,
        "SELECT strftime('%Y-%m-%d', tz_wall_ms(?1, tz_resolve(?2)) / 1000, 'unixepoch')",
        vec![Value::Integer(at_ms), Value::Text(timezone.to_owned())],
    )
    .await
    .expect("a day key")
}

/// Port of: packages/database/__tests__/integration/zoned-date-key-timezone.test.ts:60 "zonedDateKey against a real PostgreSQL > accepts a legacy timezone name instead of raising 22023"
#[tokio::test]
async fn accepts_a_legacy_timezone_name_instead_of_raising_22023() {
    let db = fresh_db().await;
    assert_eq!(day_key_for(&db, "Asia/Saigon", AFTER_MIDNIGHT_IN_VIETNAM).await, "2026-09-09");
}

/// Port of: packages/database/__tests__/integration/zoned-date-key-timezone.test.ts:66 "zonedDateKey against a real PostgreSQL > accepts the canonical spelling of the same zone"
#[tokio::test]
async fn accepts_the_canonical_spelling_of_the_same_zone() {
    let db = fresh_db().await;
    assert_eq!(day_key_for(&db, "Asia/Ho_Chi_Minh", AFTER_MIDNIGHT_IN_VIETNAM).await, "2026-09-09");
}

/// Port of: packages/database/__tests__/integration/zoned-date-key-timezone.test.ts:75 "zonedDateKey against a real PostgreSQL > both spellings agree on the day boundary"
#[tokio::test]
async fn both_spellings_agree_on_the_day_boundary() {
    let db = fresh_db().await;
    let legacy = day_key_for(&db, "Asia/Saigon", AFTER_MIDNIGHT_IN_VIETNAM).await;
    let canonical = day_key_for(&db, "Asia/Ho_Chi_Minh", AFTER_MIDNIGHT_IN_VIETNAM).await;
    assert_eq!(legacy, canonical);
}

/// Port of: packages/database/__tests__/integration/zoned-date-key-timezone.test.ts:87 "zonedDateKey against a real PostgreSQL > falls back to UTC for a name the database does not know"
#[tokio::test]
async fn falls_back_to_utc_for_a_name_the_database_does_not_know() {
    let db = fresh_db().await;
    assert_eq!(day_key_for(&db, "Not/AZone", AFTER_MIDNIGHT_IN_VIETNAM).await, "2026-09-08");
}

/// Port of: packages/database/__tests__/integration/zoned-date-key-timezone.test.ts:93 "zonedDateKey against a real PostgreSQL > a zone the database does know is used, not the fallback"
#[tokio::test]
async fn a_zone_the_database_does_know_is_used_not_the_fallback() {
    let db = fresh_db().await;
    assert_eq!(day_key_for(&db, "America/New_York", EARLY_UTC_MORNING).await, "2026-09-07");
}
