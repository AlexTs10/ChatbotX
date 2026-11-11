//! Port of `packages/database/__tests__/zoned-date-key.test.ts`
//! (P2.3 spec section 5.2, rows 16.1-16.5).
//!
//! Adapted: the original checks the Postgres SQL shape (`pg_timezone_names` and its bind
//! values). In SQLite the candidate list lives in `timezone_candidates` (used by the SQL
//! function `tz_resolve`), and `zoned_date_key` passes the zone name only as a bind value.

mod common;

use chatbotx_db::functions::timezone_candidates;
use chatbotx_db::queries::zoned_date_key;
use chatbotx_db::rusqlite::types::Value;
use common::{fresh_db, query_text};

const COLUMN: &str = r#""ContactInbox"."firstInteractionAt""#;

/// Port of: packages/database/__tests__/zoned-date-key.test.ts:16 "zonedDateKey > offers the canonical spelling alongside a legacy timezone name"
#[test]
fn offers_the_canonical_spelling_alongside_a_legacy_timezone_name() {
    assert_eq!(timezone_candidates("Asia/Saigon"), ["Asia/Saigon", "Asia/Ho_Chi_Minh"]);
}

/// Port of: packages/database/__tests__/zoned-date-key.test.ts:23 "zonedDateKey > offers the legacy spelling alongside a canonical timezone name"
#[test]
fn offers_the_legacy_spelling_alongside_a_canonical_timezone_name() {
    assert_eq!(timezone_candidates("Asia/Ho_Chi_Minh"), ["Asia/Ho_Chi_Minh", "Asia/Saigon"]);
}

/// Port of: packages/database/__tests__/zoned-date-key.test.ts:30 "zonedDateKey > prefers the caller's own spelling when the database knows both"
#[tokio::test]
async fn prefers_the_callers_own_spelling_when_the_database_knows_both() {
    let db = fresh_db().await;
    let resolve = |name: &str| vec![Value::Text(name.to_owned())];
    assert_eq!(query_text(&db, "SELECT tz_resolve(?1)", resolve("Asia/Saigon")).await.as_deref(), Some("Asia/Saigon"));
    assert_eq!(
        query_text(&db, "SELECT tz_resolve(?1)", resolve("Asia/Ho_Chi_Minh")).await.as_deref(),
        Some("Asia/Ho_Chi_Minh")
    );
}

/// Port of: packages/database/__tests__/zoned-date-key.test.ts:38 "zonedDateKey > never interpolates the timezone name into the statement text"
#[test]
fn never_interpolates_the_timezone_name_into_the_statement_text() {
    let query = zoned_date_key(COLUMN, "Asia/Saigon");
    assert!(!query.sql.contains("Asia/Saigon"), "zone name in SQL text: {}", query.sql);
    assert!(query.sql.contains("tz_resolve(?"), "no tz_resolve(?) in: {}", query.sql);
    assert!(query.params.iter().any(|p| p == "Asia/Saigon"), "zone name not bound: {:?}", query.params);
}

/// Port of: packages/database/__tests__/zoned-date-key.test.ts:45 "zonedDateKey > falls back to UTC for a timezone the database does not recognize"
#[tokio::test]
async fn falls_back_to_utc_for_a_timezone_the_database_does_not_recognize() {
    let query = zoned_date_key(COLUMN, "Not/AZone");
    assert!(query.params.iter().any(|p| p == "Not/AZone"), "zone name not bound: {:?}", query.params);

    let db = fresh_db().await;
    assert_eq!(
        query_text(&db, "SELECT tz_resolve(?1)", vec![Value::Text("Not/AZone".to_owned())]).await.as_deref(),
        Some("UTC")
    );
}
