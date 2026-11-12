//! Port of `packages/database/__tests__/integration/working-hours-date-overrides.test.ts` (P3.3).
//! Three Postgres connections -> concurrent `repo::write` jobs on one SQLite file (the writer
//! serializes them; the asserted outcome is the same). The TS tests share one workspace
//! across tests; each port seeds the state that its case reads.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::working_hours::{
    AddDateOverrideResult, DateOverrideInput, MAX_DATE_OVERRIDES, add_date_override, delete,
    list_date_overrides, list_date_overrides_in_range, replace_date_overrides,
};
use common::{fresh_db, seed};
use repo_common::{int, ok, scalar_i64, setup};
use repo4::date_of;
use serde_json::json;

const WS: i64 = 2001;
const SCHEDULE: i64 = 21;

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::working_hours(tx, SCHEDULE, WS, "Office", 0)
    })
    .await;
    db
}

fn holiday(offset: i64) -> DateOverrideInput {
    DateOverrideInput {
        date: date_of(offset),
        name: None,
        ranges: json!([]),
    }
}

fn status(r: &AddDateOverrideResult) -> &'static str {
    match r {
        AddDateOverrideResult::Ok(_) => "ok",
        AddDateOverrideResult::NotFound => "notFound",
        AddDateOverrideResult::Duplicate => "duplicate",
        AddDateOverrideResult::LimitReached => "limitReached",
    }
}

async fn holiday_dates(db: &common::TestDb) -> Vec<String> {
    ok(repo::read(db, |c| list_date_overrides(c, WS, None)).await)
        .into_iter()
        .map(|r| r.date)
        .collect()
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:80 "working hours date overrides against Postgres > a closed day is stored as an explicit empty list and a date is unique per scope"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_a_closed_day_is_stored_as_an_explicit_empty_list_and_a_date_is_unique_per_scope()
 {
    let db = seeded().await;
    let first = ok(repo::write(&db, |tx| {
        add_date_override(
            tx,
            WS,
            None,
            &DateOverrideInput {
                date: date_of(0),
                name: Some("New year".to_owned()),
                ranges: json!([]),
            },
        )
    })
    .await);
    assert_eq!(status(&first), "ok");
    let rows = ok(repo::read(&db, |c| list_date_overrides(c, WS, None)).await);
    assert_eq!(
        rows.iter().map(|r| r.ranges.0.clone()).collect::<Vec<_>>(),
        vec![json!([])]
    );
    let duplicate = ok(repo::write(&db, |tx| add_date_override(tx, WS, None, &holiday(0))).await);
    assert_eq!(status(&duplicate), "duplicate");
    let own = ok(repo::write(&db, |tx| {
        add_date_override(
            tx,
            WS,
            Some(SCHEDULE),
            &DateOverrideInput {
                date: date_of(0),
                name: None,
                ranges: json!([{ "startMinute": 0, "endMinute": 60 }]),
            },
        )
    })
    .await);
    assert_eq!(status(&own), "ok");
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:117 "working hours date overrides against Postgres > an unknown schedule reports notFound"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_an_unknown_schedule_reports_not_found() {
    let db = seeded().await;
    let result = ok(repo::write(&db, |tx| {
        add_date_override(tx, WS, Some(999_999_999_999_999_999), &holiday(5))
    })
    .await);
    assert_eq!(status(&result), "notFound");
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:129 "working hours date overrides against Postgres > concurrent adds can never exceed the row limit"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_concurrent_adds_can_never_exceed_the_row_limit()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| {
        let overrides: Vec<DateOverrideInput> = (0..MAX_DATE_OVERRIDES - 1)
            .map(|i| holiday(100 + i))
            .collect();
        replace_date_overrides(tx, WS, None, &overrides)
    })
    .await);
    let (a, b, c) = tokio::join!(
        repo::write(&db, |tx| add_date_override(tx, WS, None, &holiday(1000))),
        repo::write(&db, |tx| add_date_override(tx, WS, None, &holiday(1001))),
        repo::write(&db, |tx| add_date_override(tx, WS, None, &holiday(1002))),
    );
    let results = [ok(a), ok(b), ok(c)];
    assert_eq!(results.iter().filter(|r| status(r) == "ok").count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| status(r) == "limitReached")
            .count(),
        2
    );
    assert_eq!(holiday_dates(&db).await.len() as i64, MAX_DATE_OVERRIDES);
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:166 "working hours date overrides against Postgres > concurrent full replacements never interleave"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_concurrent_full_replacements_never_interleave()
 {
    let db = seeded().await;
    let set_a: Vec<DateOverrideInput> = (0..5).map(|i| holiday(2000 + i)).collect();
    let set_b: Vec<DateOverrideInput> = (0..7).map(|i| holiday(3000 + i)).collect();
    let (first_a, first_b) = (set_a[0].date.clone(), set_b[0].date.clone());
    let (ra, rb) = tokio::join!(
        repo::write(&db, move |tx| replace_date_overrides(tx, WS, None, &set_a)),
        repo::write(&db, move |tx| replace_date_overrides(tx, WS, None, &set_b)),
    );
    ok(ra);
    ok(rb);
    let dates = holiday_dates(&db).await;
    let is_set_a = dates.len() == 5 && dates[0] == first_a;
    let is_set_b = dates.len() == 7 && dates[0] == first_b;
    assert!(is_set_a || is_set_b);
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:195 "working hours date overrides against Postgres > the range read returns every scope's rows inside the span only"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_the_range_read_returns_every_scope_s_rows_inside_the_span_only()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| {
        replace_date_overrides(tx, WS, None, &[holiday(10), holiday(20)])
    })
    .await);
    ok(repo::write(&db, |tx| {
        replace_date_overrides(tx, WS, Some(SCHEDULE), &[holiday(11)])
    })
    .await);
    let rows = ok(repo::read(&db, |c| {
        list_date_overrides_in_range(c, WS, &date_of(9), &date_of(12))
    })
    .await);
    assert_eq!(
        rows.into_iter().map(|r| r.date).collect::<Vec<_>>(),
        vec![date_of(10), date_of(11)]
    );
}

/// Port of: packages/database/__tests__/integration/working-hours-date-overrides.test.ts:217 "working hours date overrides against Postgres > deleting the schedule removes its overrides but keeps the holidays"
#[tokio::test]
async fn working_hours_date_overrides_against_postgres_deleting_the_schedule_removes_its_overrides_but_keeps_the_holidays()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| {
        replace_date_overrides(tx, WS, None, &[holiday(10), holiday(20)])
    })
    .await);
    ok(repo::write(&db, |tx| {
        replace_date_overrides(tx, WS, Some(SCHEDULE), &[holiday(11)])
    })
    .await);
    assert!(ok(repo::write(&db, |tx| delete(tx, WS, SCHEDULE)).await));
    let own = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "WorkingHoursDateOverride" WHERE "workingHoursId" = ?1"#,
        vec![int(SCHEDULE)],
    )
    .await;
    assert_eq!(own, Some(0));
    assert!(!holiday_dates(&db).await.is_empty());
}
