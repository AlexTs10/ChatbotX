//! Port of `packages/utils/__tests__/datetime.test.ts`. Helpers that throw
//! in the original return `Result` (unwrapped where the test expects a
//! value); `Date` arguments are epoch ms.

mod support;

use chatbotx_core::utils::datetime::{
    DEFAULT_FILTER_TIMEZONE, InstantWindow, SourceTimezoneStrategy, TemporalCustomFieldSaveFormat,
    TemporalCustomFieldType, TemporalInputParsing, TemporalPrecision, WallClockWindow,
    current_temporal_literal, date_part_of, detect_temporal_precision,
    filter_value_to_utc_day_end_iso, filter_value_to_utc_day_start_iso,
    filter_value_to_utc_instant_window, filter_value_to_utc_iso,
    format_custom_field_value_in_time_zone, has_explicit_offset, has_time_component,
    is_real_calendar_date, normalize_temporal_custom_field_value, resolve_filter_timezone,
    resolve_temporal_custom_field_form_value, resolve_temporal_custom_field_save_format,
    temporal_wall_clock_window, to_naive_wall_clock_literal, to_zoned_day_start_iso,
};

const VN: &str = "Asia/Ho_Chi_Minh";
const NY: &str = "America/New_York";

fn window(start: &str, end: &str) -> WallClockWindow {
    WallClockWindow {
        start: start.to_owned(),
        end: end.to_owned(),
    }
}

fn instants(start_iso: &str, end_iso: &str) -> InstantWindow {
    InstantWindow {
        start_iso: start_iso.to_owned(),
        end_iso: end_iso.to_owned(),
    }
}

fn now() -> i64 {
    support::iso_ms("2026-07-22T18:30:00Z")
}

/// Port of: packages/utils/__tests__/datetime.test.ts:30 "datetime utilities > resolves invalid timezones to UTC"
#[test]
fn datetime_utilities_resolves_invalid_timezones_to_utc() {
    assert_eq!(resolve_filter_timezone(None), DEFAULT_FILTER_TIMEZONE);
    assert_eq!(
        resolve_filter_timezone(Some("not-a-zone")),
        DEFAULT_FILTER_TIMEZONE
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:35 "datetime utilities > detects explicit offsets"
#[test]
fn datetime_utilities_detects_explicit_offsets() {
    assert!(has_explicit_offset("2026-07-22T08:30:00.000Z"));
    assert!(has_explicit_offset("2026-07-22T08:30:00+07:00"));
    assert!(!has_explicit_offset("2026-07-22 08:30:00"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:41 "datetime utilities > converts naive datetime values to UTC in the source timezone"
#[test]
fn datetime_utilities_converts_naive_datetime_values_to_utc_in_the_source_timezone() {
    assert_eq!(
        filter_value_to_utc_iso("2026-07-22 15:30", VN).unwrap(),
        "2026-07-22T08:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:47 "datetime utilities > passes through explicit UTC instants unchanged"
#[test]
fn datetime_utilities_passes_through_explicit_utc_instants_unchanged() {
    assert_eq!(
        filter_value_to_utc_iso("2026-07-22T08:30:00.000Z", VN).unwrap(),
        "2026-07-22T08:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:53 "datetime utilities > normalizes temporal custom-field values through the handler registry"
#[test]
fn datetime_utilities_normalizes_temporal_custom_field_values_through_the_handler_registry() {
    assert_eq!(
        normalize_temporal_custom_field_value("date", Some("2026-07-22"), Some(VN)).as_deref(),
        Some("2026-07-22T00:00:00+07:00")
    );
    assert_eq!(
        normalize_temporal_custom_field_value("datetime", Some("2026-07-22 15:30"), Some(VN))
            .as_deref(),
        Some("2026-07-22T08:30:00.000Z")
    );
    assert_eq!(
        normalize_temporal_custom_field_value(
            "datetime",
            Some("2026-07-22T15:30:00+07:00"),
            Some(NY)
        )
        .as_deref(),
        Some("2026-07-22T08:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:69 "datetime utilities > returns null for invalid temporal custom-field values"
#[test]
fn datetime_utilities_returns_null_for_invalid_temporal_custom_field_values() {
    assert_eq!(
        normalize_temporal_custom_field_value("date", Some("not-a-date"), Some(VN)),
        None
    );
    assert_eq!(
        normalize_temporal_custom_field_value("datetime", Some("2026-07-22"), Some(VN)),
        None
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:78 "datetime utilities > recognizes real calendar dates and rejects impossible one 2026-02-30"
#[test]
fn datetime_utilities_recognizes_real_calendar_dates_and_rejects_impossible_one_2026_02_30() {
    assert!(!is_real_calendar_date("2026-02-30"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:78 "datetime utilities > recognizes real calendar dates and rejects impossible one 2026-02-29"
#[test]
fn datetime_utilities_recognizes_real_calendar_dates_and_rejects_impossible_one_2026_02_29() {
    assert!(!is_real_calendar_date("2026-02-29"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:78 "datetime utilities > recognizes real calendar dates and rejects impossible one 2026-04-31"
#[test]
fn datetime_utilities_recognizes_real_calendar_dates_and_rejects_impossible_one_2026_04_31() {
    assert!(!is_real_calendar_date("2026-04-31"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:78 "datetime utilities > recognizes real calendar dates and rejects impossible one 2026-00-10"
#[test]
fn datetime_utilities_recognizes_real_calendar_dates_and_rejects_impossible_one_2026_00_10() {
    assert!(!is_real_calendar_date("2026-00-10"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:87 "datetime utilities > accepts real calendar date 2026-07-22"
#[test]
fn datetime_utilities_accepts_real_calendar_date_2026_07_22() {
    assert!(is_real_calendar_date("2026-07-22"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:87 "datetime utilities > accepts real calendar date 2024-02-29"
#[test]
fn datetime_utilities_accepts_real_calendar_date_2024_02_29() {
    assert!(is_real_calendar_date("2024-02-29"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:94 "datetime utilities > returns null instead of throwing for impossible calendar dates"
#[test]
fn datetime_utilities_returns_null_instead_of_throwing_for_impossible_calendar_dates() {
    assert_eq!(
        normalize_temporal_custom_field_value("date", Some("2026-02-30"), Some(VN)),
        None
    );
    assert_eq!(
        normalize_temporal_custom_field_value("datetime", Some("2026-02-30 10:00"), Some(VN)),
        None
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:105 "datetime utilities > degrades to the raw string when formatting a corrupt stored value"
#[test]
fn datetime_utilities_degrades_to_the_raw_string_when_formatting_a_corrupt_stored_value() {
    assert_eq!(
        format_custom_field_value_in_time_zone("datetime", Some("not-a-timestamp"), VN),
        "not-a-timestamp"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:113 "datetime utilities > converts date values to the start of the calendar day in the source timezone"
#[test]
fn datetime_utilities_converts_date_values_to_the_start_of_the_calendar_day_in_the_source_timezone()
{
    assert_eq!(
        filter_value_to_utc_day_start_iso("2026-07-22", VN).unwrap(),
        "2026-07-21T17:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:119 "datetime utilities > converts day end values across DST boundaries"
#[test]
fn datetime_utilities_converts_day_end_values_across_dst_boundaries() {
    assert_eq!(
        filter_value_to_utc_day_end_iso("2026-07-01", NY).unwrap(),
        "2026-07-02T04:00:00.000Z"
    );
    assert_eq!(
        filter_value_to_utc_day_end_iso("2026-01-01", NY).unwrap(),
        "2026-01-02T05:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:128 "datetime utilities > detects the typed precision of 2026-07-22 as day"
#[test]
fn datetime_utilities_detects_the_typed_precision_of_2026_07_22_as_day() {
    assert_eq!(
        detect_temporal_precision("2026-07-22"),
        TemporalPrecision::Day
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:128 "datetime utilities > detects the typed precision of 2026-07-22 09:30 as minute"
#[test]
fn datetime_utilities_detects_the_typed_precision_of_2026_07_22_09_30_as_minute() {
    assert_eq!(
        detect_temporal_precision("2026-07-22 09:30"),
        TemporalPrecision::Minute
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:128 "datetime utilities > detects the typed precision of 2026-07-22T09:30Z as minute"
#[test]
fn datetime_utilities_detects_the_typed_precision_of_2026_07_22t09_30z_as_minute() {
    assert_eq!(
        detect_temporal_precision("2026-07-22T09:30Z"),
        TemporalPrecision::Minute
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:128 "datetime utilities > detects the typed precision of 2026-07-22 09:30:45 as second"
#[test]
fn datetime_utilities_detects_the_typed_precision_of_2026_07_22_09_30_45_as_second() {
    assert_eq!(
        detect_temporal_precision("2026-07-22 09:30:45"),
        TemporalPrecision::Second
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:128 "datetime utilities > detects the typed precision of 2026-07-22T09:30:45+07:00 as second"
#[test]
fn datetime_utilities_detects_the_typed_precision_of_2026_07_22t09_30_45_07_00_as_second() {
    assert_eq!(
        detect_temporal_precision("2026-07-22T09:30:45+07:00"),
        TemporalPrecision::Second
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:138 "datetime utilities > builds a naive wall-clock window for 2026-07-22"
#[test]
fn datetime_utilities_builds_a_naive_wall_clock_window_for_2026_07_22() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22").unwrap(),
        window("2026-07-22T00:00:00", "2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:138 "datetime utilities > builds a naive wall-clock window for 2026-07-22 09:30"
#[test]
fn datetime_utilities_builds_a_naive_wall_clock_window_for_2026_07_22_09_30() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22 09:30").unwrap(),
        window("2026-07-22T09:30:00", "2026-07-22T09:31:00")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:138 "datetime utilities > builds a naive wall-clock window for 2026-07-22 09:30:45"
#[test]
fn datetime_utilities_builds_a_naive_wall_clock_window_for_2026_07_22_09_30_45() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22 09:30:45").unwrap(),
        window("2026-07-22T09:30:45", "2026-07-22T09:30:46")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:138 "datetime utilities > builds a naive wall-clock window for 2026-07-22 23:59"
#[test]
fn datetime_utilities_builds_a_naive_wall_clock_window_for_2026_07_22_23_59() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22 23:59").unwrap(),
        window("2026-07-22T23:59:00", "2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:138 "datetime utilities > builds a naive wall-clock window for 2026-07-22 09:59:59"
#[test]
fn datetime_utilities_builds_a_naive_wall_clock_window_for_2026_07_22_09_59_59() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22 09:59:59").unwrap(),
        window("2026-07-22T09:59:59", "2026-07-22T10:00:00")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:150 "datetime utilities > floors sub-precision digits out of the wall-clock window"
#[test]
fn datetime_utilities_floors_sub_precision_digits_out_of_the_wall_clock_window() {
    assert_eq!(
        temporal_wall_clock_window("2026-07-22T09:30:45.750Z").unwrap(),
        window("2026-07-22T09:30:45", "2026-07-22T09:30:46")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:158 "datetime utilities > anchors an instant window to the criteria timezone"
#[test]
fn datetime_utilities_anchors_an_instant_window_to_the_criteria_timezone() {
    assert_eq!(
        filter_value_to_utc_instant_window("2026-07-22 09:30", VN).unwrap(),
        instants("2026-07-22T02:30:00.000Z", "2026-07-22T02:31:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:166 "datetime utilities > spans the full day in an instant window for a date-only value"
#[test]
fn datetime_utilities_spans_the_full_day_in_an_instant_window_for_a_date_only_value() {
    assert_eq!(
        filter_value_to_utc_instant_window("2026-07-22", VN).unwrap(),
        instants("2026-07-21T17:00:00.000Z", "2026-07-22T17:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:173 "datetime utilities > honors an explicit offset over the criteria timezone in an instant window"
#[test]
fn datetime_utilities_honors_an_explicit_offset_over_the_criteria_timezone_in_an_instant_window() {
    assert_eq!(
        filter_value_to_utc_instant_window("2026-07-22T09:30:00+07:00", NY).unwrap(),
        instants("2026-07-22T02:30:00.000Z", "2026-07-22T02:30:01.000Z")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:183 "datetime utilities > keeps an instant day window DST-safe across a fall-back boundary"
#[test]
fn datetime_utilities_keeps_an_instant_day_window_dst_safe_across_a_fall_back_boundary() {
    assert_eq!(
        filter_value_to_utc_instant_window("2026-11-01", NY).unwrap(),
        instants("2026-11-01T04:00:00.000Z", "2026-11-02T05:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:191 "datetime utilities > formats temporal custom-field values in a target timezone"
#[test]
fn datetime_utilities_formats_temporal_custom_field_values_in_a_target_timezone() {
    assert_eq!(
        format_custom_field_value_in_time_zone("date", Some("2026-07-22T00:00:00+07:00"), VN),
        "2026-07-22"
    );
    assert_eq!(
        format_custom_field_value_in_time_zone("datetime", Some("2026-07-22T08:30:00.000Z"), VN),
        "2026-07-22 15:30:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:204 "datetime utilities > returns non-temporal values unchanged"
#[test]
fn datetime_utilities_returns_non_temporal_values_unchanged() {
    assert_eq!(
        format_custom_field_value_in_time_zone("shortText", Some("hello"), VN),
        "hello"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:210 "datetime utilities > normalizes a date value to an offset-preserved start of day"
#[test]
fn datetime_utilities_normalizes_a_date_value_to_an_offset_preserved_start_of_day() {
    assert_eq!(
        to_zoned_day_start_iso("2026-07-22", VN).unwrap(),
        "2026-07-22T00:00:00+07:00"
    );
    assert_eq!(date_part_of("2026-07-22T09:30:00+07:00"), "2026-07-22");
    assert!(!has_time_component("2026-07-22"));
    assert!(has_time_component("2026-07-22 09:30"));
}

/// Port of: packages/utils/__tests__/datetime.test.ts:219 "datetime utilities > strips any offset to a naive wall-clock literal"
#[test]
fn datetime_utilities_strips_any_offset_to_a_naive_wall_clock_literal() {
    assert_eq!(
        to_naive_wall_clock_literal("2026-07-20"),
        "2026-07-20T00:00:00"
    );
    assert_eq!(
        to_naive_wall_clock_literal("2026-07-20 09:30"),
        "2026-07-20T09:30"
    );
    assert_eq!(
        to_naive_wall_clock_literal("2026-07-20T09:30:00+07:00"),
        "2026-07-20T09:30:00"
    );
    assert_eq!(
        to_naive_wall_clock_literal("2026-07-20T09:30:00.000Z"),
        "2026-07-20T09:30:00.000"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:233 "datetime utilities > resolves temporal picker serialization by type"
#[test]
fn datetime_utilities_resolves_temporal_picker_serialization_by_type() {
    assert_eq!(
        resolve_temporal_custom_field_save_format("date"),
        TemporalCustomFieldSaveFormat::Formatted
    );
    assert_eq!(
        resolve_temporal_custom_field_save_format("datetime"),
        TemporalCustomFieldSaveFormat::Iso
    );
    assert_eq!(
        resolve_temporal_custom_field_save_format("date").as_str(),
        "formatted"
    );
    assert_eq!(
        resolve_temporal_custom_field_save_format("datetime").as_str(),
        "iso"
    );
    assert_eq!(
        resolve_temporal_custom_field_form_value("date", "2026-07-22T00:00:00+07:00"),
        "2026-07-22"
    );
    assert_eq!(
        resolve_temporal_custom_field_form_value("datetime", "2026-07-22T08:30:00.000Z"),
        "2026-07-22T08:30:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:258 "legacy backfill migration oracle > datetime backfill: Asia/Ho_Chi_Minh 2026-07-22 10:00 -> 2026-07-22T03:00:00.000Z"
#[test]
fn legacy_backfill_migration_oracle_datetime_backfill_asia_ho_chi_minh_2026_07_22_10_00_2026_07_22t03_00_00_000z()
 {
    assert_eq!(
        filter_value_to_utc_iso("2026-07-22 10:00", VN).unwrap(),
        "2026-07-22T03:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:258 "legacy backfill migration oracle > datetime backfill: America/New_York 2026-07-15 12:00 -> 2026-07-15T16:00:00.000Z"
#[test]
fn legacy_backfill_migration_oracle_datetime_backfill_america_new_york_2026_07_15_12_00_2026_07_15t16_00_00_000z()
 {
    assert_eq!(
        filter_value_to_utc_iso("2026-07-15 12:00", NY).unwrap(),
        "2026-07-15T16:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:258 "legacy backfill migration oracle > datetime backfill: America/New_York 2026-01-15 12:00 -> 2026-01-15T17:00:00.000Z"
#[test]
fn legacy_backfill_migration_oracle_datetime_backfill_america_new_york_2026_01_15_12_00_2026_01_15t17_00_00_000z()
 {
    assert_eq!(
        filter_value_to_utc_iso("2026-01-15 12:00", NY).unwrap(),
        "2026-01-15T17:00:00.000Z"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:267 "legacy backfill migration oracle > date backfill (start of day): Asia/Ho_Chi_Minh 2026-07-22 -> 2026-07-22T00:00:00+07:00"
#[test]
fn legacy_backfill_migration_oracle_date_backfill_start_of_day_asia_ho_chi_minh_2026_07_22_2026_07_22t00_00_00_07_00()
 {
    assert_eq!(
        to_zoned_day_start_iso("2026-07-22", VN).unwrap(),
        "2026-07-22T00:00:00+07:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:267 "legacy backfill migration oracle > date backfill (start of day): America/New_York 2026-07-15 -> 2026-07-15T00:00:00-04:00"
#[test]
fn legacy_backfill_migration_oracle_date_backfill_start_of_day_america_new_york_2026_07_15_2026_07_15t00_00_00_04_00()
 {
    assert_eq!(
        to_zoned_day_start_iso("2026-07-15", NY).unwrap(),
        "2026-07-15T00:00:00-04:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:267 "legacy backfill migration oracle > date backfill (start of day): America/New_York 2026-01-15 -> 2026-01-15T00:00:00-05:00"
#[test]
fn legacy_backfill_migration_oracle_date_backfill_start_of_day_america_new_york_2026_01_15_2026_01_15t00_00_00_05_00()
 {
    assert_eq!(
        to_zoned_day_start_iso("2026-01-15", NY).unwrap(),
        "2026-01-15T00:00:00-05:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:267 "legacy backfill migration oracle > date backfill (start of day): America/New_York 2026-03-08 -> 2026-03-08T00:00:00-05:00"
#[test]
fn legacy_backfill_migration_oracle_date_backfill_start_of_day_america_new_york_2026_03_08_2026_03_08t00_00_00_05_00()
 {
    assert_eq!(
        to_zoned_day_start_iso("2026-03-08", NY).unwrap(),
        "2026-03-08T00:00:00-05:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:279 "temporal write-path enums > parsing-mode values are stable"
#[test]
fn temporal_write_path_enums_parsing_mode_values_are_stable() {
    assert_eq!(TemporalInputParsing::Strict.as_str(), "strict");
    assert_eq!(TemporalInputParsing::Lenient.as_str(), "lenient");
}

/// Port of: packages/utils/__tests__/datetime.test.ts:284 "temporal write-path enums > source-timezone strategy values are stable"
#[test]
fn temporal_write_path_enums_source_timezone_strategy_values_are_stable() {
    assert_eq!(
        SourceTimezoneStrategy::ContactThenWorkspace.as_str(),
        "contactThenWorkspace"
    );
    assert_eq!(SourceTimezoneStrategy::Workspace.as_str(), "workspace");
}

/// Port of: packages/utils/__tests__/datetime.test.ts:299 "currentTemporalLiteral > renders a date as today's calendar day in the given zone"
#[test]
fn currenttemporalliteral_renders_a_date_as_today_s_calendar_day_in_the_given_zone() {
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Date, Some(NY), now()),
        "2026-07-22"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:303 "currentTemporalLiteral > renders a datetime as the wall clock in the given zone"
#[test]
fn currenttemporalliteral_renders_a_datetime_as_the_wall_clock_in_the_given_zone() {
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Datetime, Some(NY), now()),
        "2026-07-22 14:30:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:309 "currentTemporalLiteral > crosses midnight into the next day for an ahead-of-UTC zone"
#[test]
fn currenttemporalliteral_crosses_midnight_into_the_next_day_for_an_ahead_of_utc_zone() {
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Date, Some(VN), now()),
        "2026-07-23"
    );
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Datetime, Some(VN), now()),
        "2026-07-23 01:30:00"
    );
}

/// Port of: packages/utils/__tests__/datetime.test.ts:317 "currentTemporalLiteral > falls back to UTC for a blank or unrecognized zone"
#[test]
fn currenttemporalliteral_falls_back_to_utc_for_a_blank_or_unrecognized_zone() {
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Date, None, now()),
        "2026-07-22"
    );
    assert_eq!(
        current_temporal_literal(TemporalCustomFieldType::Datetime, Some("not-a-zone"), now()),
        "2026-07-22 18:30:00"
    );
}
