//! Port of `packages/business/__tests__/contact-custom-field-normalize.test.ts`
//! (P3.6-D2 slice contact custom-field values). The mocked resolver is a
//! counting closure; the mocked `contactModel` / `workspaceModel` lookups of
//! `createSourceTimezoneResolver` become real `Contact` / `Workspace` rows
//! ("called once" = a second `resolve` does not see a later row change;
//! "no lookup" = rows with another zone do not win). Fake timers = `now_ms`.

mod common;

use std::cell::Cell;

use chatbotx_business::Result;
use chatbotx_business::contact_custom_field::{
    NormalizeInput, SourceTimezoneResolver, SourceTimezoneStrategy, TemporalInputParsing,
    normalize_custom_field_value_for_storage,
};
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, setup, two_workspaces};

/// 2026-07-22T18:00:00Z.
const FAKE_NOW: i64 = 1_784_743_200_000;

/// The result and how often the resolver (answering `zone`) was called.
fn run(input: NormalizeInput<'_>, zone: &str) -> (Option<String>, usize) {
    run_at(input, zone, FAKE_NOW)
}

fn run_at(input: NormalizeInput<'_>, zone: &str, now_ms: i64) -> (Option<String>, usize) {
    let calls = Cell::new(0);
    let mut resolver = || -> Result<String> {
        calls.set(calls.get() + 1);
        Ok(zone.to_string())
    };
    let out =
        normalize_custom_field_value_for_storage(&input, &mut resolver, now_ms).expect("normalize");
    (out, calls.get())
}

fn lenient<'a>(kind: &'a str, value: &'a str) -> NormalizeInput<'a> {
    NormalizeInput {
        parsing: TemporalInputParsing::Lenient,
        ..NormalizeInput::new(kind, value)
    }
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

/// Contact 2 of `WS` with `contact_zone`; `WS` with `workspace_zone` (`None` = blank: the column is NOT NULL).
async fn zones(contact_zone: Option<&'static str>, workspace_zone: Option<&'static str>) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = ?1 WHERE "id" = ?2"#,
            params![workspace_zone.unwrap_or(""), WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId", "timezone") VALUES (2, 'Ada', ?1, ?2)"#,
            params![WS, contact_zone],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn resolve_twice(db: &TestDb, mut resolver: SourceTimezoneResolver) -> (String, String) {
    db.write(move |tx| {
        let first = resolver.resolve(tx).expect("first resolve");
        // A later row change is not seen: the zone is memoized.
        tx.execute(
            r#"UPDATE "Contact" SET "timezone" = 'Europe/Paris' WHERE "id" = 2"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = 'Europe/Paris' WHERE "id" = ?1"#,
            params![WS],
        )?;
        let second = resolver.resolve(tx).expect("second resolve");
        Ok((first, second))
    })
    .await
    .expect("resolve")
}

async fn resolve_once(db: &TestDb, mut resolver: SourceTimezoneResolver) -> String {
    db.read(move |conn| Ok(resolver.resolve(conn).expect("resolve")))
        .await
        .expect("resolve")
}

fn contact_resolver(explicit: Option<&str>) -> SourceTimezoneResolver {
    SourceTimezoneResolver::new(
        WS,
        2,
        SourceTimezoneStrategy::ContactThenWorkspace,
        explicit,
    )
}

// ---- contact custom field normalization ----

#[test]
fn contact_custom_field_normalization_normalizes_naive_temporal_values_using_the_source_timezone() {
    let vn = "Asia/Ho_Chi_Minh";
    assert_eq!(
        run(NormalizeInput::new("date", "2026-07-22"), vn).0,
        some("2026-07-22T00:00:00+07:00")
    );
    assert_eq!(
        run(NormalizeInput::new("datetime", "2026-07-22 15:30"), vn).0,
        some("2026-07-22T08:30:00.000Z")
    );
}

#[test]
fn contact_custom_field_normalization_returns_null_not_the_raw_value_when_a_temporal_value_can_t_be_normalized()
 {
    let vn = "Asia/Ho_Chi_Minh";
    assert_eq!(run(NormalizeInput::new("date", "2026-02-30"), vn).0, None);
    assert_eq!(
        run(NormalizeInput::new("datetime", "not-a-date"), vn).0,
        None
    );
}

#[test]
fn contact_custom_field_normalization_passes_non_temporal_values_through_unchanged() {
    let (out, calls) = run(
        NormalizeInput::new("shortText", "hello world"),
        "Asia/Ho_Chi_Minh",
    );
    assert_eq!(out, some("hello world"));
    assert_eq!(calls, 0);
}

#[test]
fn contact_custom_field_normalization_passes_through_explicit_offsets_without_reinterpreting_them()
{
    let (out, calls) = run(
        NormalizeInput::new("datetime", "2026-07-22T15:30:00+07:00"),
        "Asia/Ho_Chi_Minh",
    );
    assert_eq!(out, some("2026-07-22T08:30:00.000Z"));
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn contact_custom_field_normalization_resolves_the_source_timezone_once_and_prefers_contact_over_workspace()
 {
    let db = zones(Some("Asia/Tokyo"), Some("Asia/Ho_Chi_Minh")).await;
    let (first, second) = resolve_twice(&db, contact_resolver(None)).await;
    assert_eq!(first, "Asia/Tokyo");
    assert_eq!(second, "Asia/Tokyo");
}

#[tokio::test]
async fn contact_custom_field_normalization_falls_back_to_the_workspace_timezone_when_the_contact_has_none()
 {
    let db = zones(None, Some("Asia/Ho_Chi_Minh")).await;
    assert_eq!(
        resolve_once(&db, contact_resolver(None)).await,
        "Asia/Ho_Chi_Minh"
    );
}

#[tokio::test]
async fn contact_custom_field_normalization_defaults_to_utc_when_neither_contact_nor_workspace_has_a_usable_timezone()
 {
    let db = zones(None, None).await;
    // The contact row is absent (id 3).
    let resolver =
        SourceTimezoneResolver::new(WS, 3, SourceTimezoneStrategy::ContactThenWorkspace, None);
    assert_eq!(resolve_once(&db, resolver).await, "UTC");
}

#[test]
fn contact_custom_field_normalization_anchors_a_date_to_the_explicit_client_timezone_without_consulting_the_resolver()
 {
    let input = NormalizeInput {
        explicit_timezone: Some("America/New_York"),
        ..NormalizeInput::new("date", "2026-07-22")
    };
    let (out, calls) = run(input, "Asia/Tokyo");
    assert_eq!(out, some("2026-07-22T00:00:00-04:00"));
    assert_eq!(calls, 0);
}

#[test]
fn contact_custom_field_normalization_ignores_the_explicit_client_timezone_for_a_naive_datetime_and_uses_the_resolver()
 {
    let input = NormalizeInput {
        explicit_timezone: Some("America/New_York"),
        ..NormalizeInput::new("datetime", "2026-07-22 15:30")
    };
    let (out, calls) = run(input, "Asia/Ho_Chi_Minh");
    assert_eq!(out, some("2026-07-22T08:30:00.000Z"));
    assert_eq!(calls, 1);
}

#[test]
fn contact_custom_field_normalization_falls_back_to_the_resolver_when_a_date_s_explicit_client_timezone_is_blank()
 {
    let input = NormalizeInput {
        explicit_timezone: Some("   "),
        ..NormalizeInput::new("date", "2026-07-22")
    };
    let (out, calls) = run(input, "Asia/Tokyo");
    assert_eq!(out, some("2026-07-22T00:00:00+09:00"));
    assert_eq!(calls, 1);
}

// ---- lenient temporal parsing and workspace strategy ----

const VN: &str = "Asia/Ho_Chi_Minh";

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_parses_a_dmy_datetime_and_anchors_it_to_the_source_zone()
 {
    assert_eq!(
        run(lenient("datetime", "23/07/2026 09:30"), VN).0,
        some("2026-07-23T02:30:00.000Z")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_parses_a_dmy_date_offset_preserved_in_the_source_zone()
 {
    assert_eq!(
        run(lenient("date", "23/07/2026"), VN).0,
        some("2026-07-23T00:00:00+07:00")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_keeps_the_authored_calendar_day_of_an_offset_bearing_date()
 {
    assert_eq!(
        run(lenient("date", "2026-05-19T23:30:00-04:00"), VN).0,
        some("2026-05-19T00:00:00+07:00")
    );
    assert_eq!(
        run(lenient("date", "2026-05-19T20:00:00Z"), VN).0,
        some("2026-05-19T00:00:00+07:00")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_still_re_anchors_a_date_the_strict_normalizer_rejects()
 {
    assert_eq!(
        run(lenient("date", "1700000000"), VN).0,
        some("2023-11-15T00:00:00+07:00")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_parses_a_unix_timestamp_into_a_datetime_utc_instant()
 {
    assert_eq!(
        run(lenient("datetime", "1721800800"), VN).0,
        some("2024-07-24T06:00:00.000Z")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_remains_a_strict_superset_for_canonical_fractional_datetimes()
 {
    assert_eq!(
        run(lenient("datetime", "2026-07-23T09:30:00.123"), VN).0,
        some("2026-07-23T02:30:00.123Z")
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_strict_mode_by_default_still_rejects_a_non_iso_value()
 {
    assert_eq!(
        run(NormalizeInput::new("datetime", "23/07/2026 09:30"), VN).0,
        None
    );
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_returns_null_for_genuinely_unparseable_input()
 {
    assert_eq!(run(lenient("datetime", "not-a-date-at-all"), VN).0, None);
}

#[test]
fn lenient_temporal_parsing_and_workspace_strategy_lenient_mode_gives_a_time_only_cell_today_s_date_in_the_source_zone()
 {
    assert_eq!(
        run_at(lenient("datetime", "09:30"), VN, FAKE_NOW).0,
        some("2026-07-23T02:30:00.000Z")
    );
    assert_eq!(
        run_at(lenient("date", "09:30"), VN, FAKE_NOW).0,
        some("2026-07-23T00:00:00+07:00")
    );
}

#[tokio::test]
async fn lenient_temporal_parsing_and_workspace_strategy_workspace_strategy_resolves_the_workspace_zone_without_a_contact_query()
 {
    let db = zones(Some("Asia/Tokyo"), Some("Asia/Ho_Chi_Minh")).await;
    let resolver = SourceTimezoneResolver::new(WS, 2, SourceTimezoneStrategy::Workspace, None);
    assert_eq!(resolve_once(&db, resolver).await, "Asia/Ho_Chi_Minh");
}

// ---- explicit source-timezone override ----

#[tokio::test]
async fn explicit_source_timezone_override_anchors_every_temporal_type_without_a_db_lookup() {
    let db = zones(Some("Asia/Tokyo"), Some("Asia/Ho_Chi_Minh")).await;
    let (first, second) = resolve_twice(&db, contact_resolver(Some("America/New_York"))).await;
    assert_eq!(first, "America/New_York");
    assert_eq!(second, "America/New_York");
}

#[tokio::test]
async fn explicit_source_timezone_override_falls_back_to_the_contact_workspace_resolver_when_the_override_is_blank()
 {
    let db = zones(None, Some("Asia/Ho_Chi_Minh")).await;
    assert_eq!(
        resolve_once(&db, contact_resolver(Some("   "))).await,
        "Asia/Ho_Chi_Minh"
    );
}

#[tokio::test]
async fn explicit_source_timezone_override_a_naive_datetime_anchors_to_the_override_zone_ignoring_the_client_zone()
 {
    let db = zones(Some("Asia/Tokyo"), Some("UTC")).await;
    let out = db
        .read(|conn| {
            let mut resolver = contact_resolver(Some("Asia/Ho_Chi_Minh"));
            let mut resolve = || resolver.resolve(conn);
            let input = NormalizeInput {
                explicit_timezone: Some("America/New_York"),
                ..NormalizeInput::new("datetime", "2026-07-22 15:30")
            };
            Ok(normalize_custom_field_value_for_storage(
                &input,
                &mut resolve,
                FAKE_NOW,
            ))
        })
        .await
        .expect("read")
        .expect("normalize");
    assert_eq!(out, some("2026-07-22T08:30:00.000Z"));
}

// ---- blank temporal value stamps now ----

#[test]
fn blank_temporal_value_stamps_now_date_stamps_today_s_start_of_day_in_the_resolved_zone() {
    let input = NormalizeInput {
        fill_empty_temporal_with_now: true,
        ..NormalizeInput::new("date", "")
    };
    assert_eq!(
        run_at(input, VN, FAKE_NOW).0,
        some("2026-07-23T00:00:00+07:00")
    );
}

#[test]
fn blank_temporal_value_stamps_now_datetime_stamps_the_current_instant_as_utc() {
    let input = NormalizeInput {
        fill_empty_temporal_with_now: true,
        ..NormalizeInput::new("datetime", "")
    };
    assert_eq!(
        run_at(input, VN, FAKE_NOW).0,
        some("2026-07-22T18:00:00.000Z")
    );
}

#[test]
fn blank_temporal_value_stamps_now_leaves_a_blank_temporal_value_empty_when_the_flag_is_off() {
    let (out, calls) = run(NormalizeInput::new("date", ""), VN);
    assert_eq!(out, some(""));
    assert_eq!(calls, 0);
}

#[test]
fn blank_temporal_value_stamps_now_passes_a_blank_non_temporal_value_through_even_when_the_flag_is_on()
 {
    let input = NormalizeInput {
        fill_empty_temporal_with_now: true,
        ..NormalizeInput::new("shortText", "")
    };
    let (out, calls) = run(input, VN);
    assert_eq!(out, some(""));
    assert_eq!(calls, 0);
}
