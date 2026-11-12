//! Port of `packages/utils/__tests__/temporal-input.test.ts`. The `now`
//! argument (default `new Date()`) is explicit.

mod support;

use chatbotx_core::utils::datetime::TemporalCustomFieldType;
use chatbotx_core::utils::temporal_input::parse_loose_temporal_value;

const VN: &str = "Asia/Ho_Chi_Minh";
const NY: &str = "America/New_York";

fn cross_midnight_now() -> i64 {
    support::iso_ms("2026-07-22T18:00:00Z")
}

fn kind(name: &str) -> TemporalCustomFieldType {
    TemporalCustomFieldType::parse(name).unwrap()
}

/// `parseLooseTemporalValue(type, raw, zone)` (now = the real clock).
fn parse(name: &str, raw: &str, zone: &str) -> Option<String> {
    parse_loose_temporal_value(kind(name), raw, Some(zone), support::now_ms())
}

/// `parseLooseTemporalValue(type, raw, zone, CROSS_MIDNIGHT_NOW)`.
fn parse_at(name: &str, raw: &str, zone: &str) -> Option<String> {
    parse_loose_temporal_value(kind(name), raw, Some(zone), cross_midnight_now())
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23/07/2026" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_07_2026_2026_07_23t00_00_00() {
    assert_eq!(
        parse("datetime", "23/07/2026", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23/07/2026 09:30" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_07_2026_09_30_2026_07_23t09_30_00() {
    assert_eq!(
        parse("datetime", "23/07/2026 09:30", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23/07/2026 09:30:45" -> "2026-07-23T09:30:45""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_07_2026_09_30_45_2026_07_23t09_30_45() {
    assert_eq!(
        parse("datetime", "23/07/2026 09:30:45", VN),
        s("2026-07-23T09:30:45")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "2026-07-23" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_2026_07_23_2026_07_23t00_00_00() {
    assert_eq!(
        parse("datetime", "2026-07-23", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "2026-07-23T09:30:00" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_2026_07_23t09_30_00_2026_07_23t09_30_00() {
    assert_eq!(
        parse("datetime", "2026-07-23T09:30:00", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "2026-07-23T09:30:00.123" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_2026_07_23t09_30_00_123_2026_07_23t09_30_00()
 {
    assert_eq!(
        parse("datetime", "2026-07-23T09:30:00.123", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "2026-07-23 09:30" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_2026_07_23_09_30_2026_07_23t09_30_00() {
    assert_eq!(
        parse("datetime", "2026-07-23 09:30", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "Jul 23, 2026" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_jul_23_2026_2026_07_23t00_00_00() {
    assert_eq!(
        parse("datetime", "Jul 23, 2026", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23 July 2026" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_july_2026_2026_07_23t00_00_00() {
    assert_eq!(
        parse("datetime", "23 July 2026", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23-07-2026" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_07_2026_2026_07_23t00_00_00_2() {
    assert_eq!(
        parse("datetime", "23-07-2026", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses datetime "23.07.2026" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_naive_values_parses_datetime_23_07_2026_2026_07_23t00_00_00_3() {
    assert_eq!(
        parse("datetime", "23.07.2026", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses date "23/07/2026" -> "2026-07-23""
#[test]
fn parseloosetemporalvalue_naive_values_parses_date_23_07_2026_2026_07_23() {
    assert_eq!(parse("date", "23/07/2026", VN), s("2026-07-23"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses date "23/07/2026 09:30" -> "2026-07-23""
#[test]
fn parseloosetemporalvalue_naive_values_parses_date_23_07_2026_09_30_2026_07_23() {
    assert_eq!(parse("date", "23/07/2026 09:30", VN), s("2026-07-23"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses date "2026-07-23" -> "2026-07-23""
#[test]
fn parseloosetemporalvalue_naive_values_parses_date_2026_07_23_2026_07_23() {
    assert_eq!(parse("date", "2026-07-23", VN), s("2026-07-23"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:14 "parseLooseTemporalValue - naive values > parses date "Jul 23, 2026" -> "2026-07-23""
#[test]
fn parseloosetemporalvalue_naive_values_parses_date_jul_23_2026_2026_07_23() {
    assert_eq!(parse("date", "Jul 23, 2026", VN), s("2026-07-23"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:34 "parseLooseTemporalValue - naive values > unpadded day and month components parse"
#[test]
fn parseloosetemporalvalue_naive_values_unpadded_day_and_month_components_parse() {
    assert_eq!(parse("date", "3/7/2026", VN), s("2026-07-03"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:38 "parseLooseTemporalValue - naive values > day-first wins for ambiguous values"
#[test]
fn parseloosetemporalvalue_naive_values_day_first_wins_for_ambiguous_values() {
    assert_eq!(parse("date", "12/07/2026", VN), s("2026-07-12"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:42 "parseLooseTemporalValue - naive values > structurally impossible DMY falls through to the MDY rescue"
#[test]
fn parseloosetemporalvalue_naive_values_structurally_impossible_dmy_falls_through_to_the_mdy_rescue()
 {
    assert_eq!(parse("date", "07/13/2026", VN), s("2026-07-13"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:46 "parseLooseTemporalValue - naive values > compact yyyyMMdd parses as a calendar date, not unix seconds"
#[test]
fn parseloosetemporalvalue_naive_values_compact_yyyymmdd_parses_as_a_calendar_date_not_unix_seconds()
 {
    assert_eq!(parse("date", "20260723", VN), s("2026-07-23"));
    assert_eq!(parse("datetime", "20260723", VN), s("2026-07-23T00:00:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:53 "parseLooseTemporalValue - naive values > localized numeric month labels parse"
#[test]
fn parseloosetemporalvalue_naive_values_localized_numeric_month_labels_parse() {
    assert_eq!(parse("date", "23 thg 7, 2026", VN), s("2026-07-23"));
    assert_eq!(
        parse("datetime", "23 tháng 7 2026 09:30", VN),
        s("2026-07-23T09:30:00")
    );
    assert_eq!(
        parse("datetime", "ngày 23 tháng 7 năm 2026 lúc 09:30:45", VN),
        s("2026-07-23T09:30:45")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:71 "parseLooseTemporalValue - absolute instants > datetime unix "1721800800" -> UTC "2024-07-24T06:00:00.000Z""
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_unix_1721800800_utc_2024_07_24t06_00_00_000z()
{
    assert_eq!(
        parse("datetime", "1721800800", VN),
        s("2024-07-24T06:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:71 "parseLooseTemporalValue - absolute instants > datetime unix "1721800800000" -> UTC "2024-07-24T06:00:00.000Z""
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_unix_1721800800000_utc_2024_07_24t06_00_00_000z()
 {
    assert_eq!(
        parse("datetime", "1721800800000", VN),
        s("2024-07-24T06:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:71 "parseLooseTemporalValue - absolute instants > datetime unix "1700000000" -> UTC "2023-11-14T22:13:20.000Z""
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_unix_1700000000_utc_2023_11_14t22_13_20_000z()
{
    assert_eq!(
        parse("datetime", "1700000000", VN),
        s("2023-11-14T22:13:20.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:71 "parseLooseTemporalValue - absolute instants > datetime unix "1700000000000" -> UTC "2023-11-14T22:13:20.000Z""
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_unix_1700000000000_utc_2023_11_14t22_13_20_000z()
 {
    assert_eq!(
        parse("datetime", "1700000000000", VN),
        s("2023-11-14T22:13:20.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:80 "parseLooseTemporalValue - absolute instants > datetime with explicit offset becomes a UTC instant"
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_with_explicit_offset_becomes_a_utc_instant() {
    assert_eq!(
        parse("datetime", "2026-07-22T15:30:00+07:00", VN),
        s("2026-07-22T08:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:86 "parseLooseTemporalValue - absolute instants > datetime with Z remains the same UTC instant"
#[test]
fn parseloosetemporalvalue_absolute_instants_datetime_with_z_remains_the_same_utc_instant() {
    assert_eq!(
        parse("datetime", "2026-07-22T08:30:00.000Z", VN),
        s("2026-07-22T08:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:92 "parseLooseTemporalValue - absolute instants > date from a unix instant resolves the calendar day in the anchor zone"
#[test]
fn parseloosetemporalvalue_absolute_instants_date_from_a_unix_instant_resolves_the_calendar_day_in_the_anchor_zone()
 {
    assert_eq!(parse("date", "1700000000", VN), s("2023-11-15"));
    assert_eq!(parse("date", "1721800800", VN), s("2024-07-24"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:97 "parseLooseTemporalValue - absolute instants > short spreadsheet numerics are not treated as unix timestamps"
#[test]
fn parseloosetemporalvalue_absolute_instants_short_spreadsheet_numerics_are_not_treated_as_unix_timestamps()
 {
    assert_eq!(parse("datetime", "45497", VN), None);
    assert_eq!(parse("datetime", "2026", VN), None);
    assert_eq!(parse("datetime", "946684800000", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "09:30" takes today in the anchor zone -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_09_30_takes_today_in_the_anchor_zone_2026_07_23t09_30_00()
 {
    assert_eq!(parse_at("datetime", "09:30", VN), s("2026-07-23T09:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "9:30" takes today in the anchor zone -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_9_30_takes_today_in_the_anchor_zone_2026_07_23t09_30_00()
 {
    assert_eq!(parse_at("datetime", "9:30", VN), s("2026-07-23T09:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "09:30:45" takes today in the anchor zone -> "2026-07-23T09:30:45""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_09_30_45_takes_today_in_the_anchor_zone_2026_07_23t09_30_45()
 {
    assert_eq!(
        parse_at("datetime", "09:30:45", VN),
        s("2026-07-23T09:30:45")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "9:30 AM" takes today in the anchor zone -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_9_30_am_takes_today_in_the_anchor_zone_2026_07_23t09_30_00()
 {
    assert_eq!(
        parse_at("datetime", "9:30 AM", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "9:30 PM" takes today in the anchor zone -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_9_30_pm_takes_today_in_the_anchor_zone_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse_at("datetime", "9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "23:59" takes today in the anchor zone -> "2026-07-23T23:59:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_23_59_takes_today_in_the_anchor_zone_2026_07_23t23_59_00()
 {
    assert_eq!(parse_at("datetime", "23:59", VN), s("2026-07-23T23:59:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:105 "parseLooseTemporalValue - time-only values > datetime "00:00" takes today in the anchor zone -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_time_only_values_datetime_00_00_takes_today_in_the_anchor_zone_2026_07_23t00_00_00()
 {
    assert_eq!(parse_at("datetime", "00:00", VN), s("2026-07-23T00:00:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:119 "parseLooseTemporalValue - time-only values > today comes from the anchor zone, not the server clock"
#[test]
fn parseloosetemporalvalue_time_only_values_today_comes_from_the_anchor_zone_not_the_server_clock()
{
    assert_eq!(parse_at("datetime", "09:30", VN), s("2026-07-23T09:30:00"));
    assert_eq!(parse_at("datetime", "09:30", NY), s("2026-07-22T09:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:129 "parseLooseTemporalValue - time-only values > a date field keeps only today's calendar day"
#[test]
fn parseloosetemporalvalue_time_only_values_a_date_field_keeps_only_today_s_calendar_day() {
    assert_eq!(parse_at("date", "09:30", VN), s("2026-07-23"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "25:00""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_25_00() {
    assert_eq!(parse_at("datetime", "25:00", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "24:00""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_24_00() {
    assert_eq!(parse_at("datetime", "24:00", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "9:60""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_9_60() {
    assert_eq!(parse_at("datetime", "9:60", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "12:30 XM""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_12_30_xm() {
    assert_eq!(parse_at("datetime", "12:30 XM", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "9:""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_9() {
    assert_eq!(parse_at("datetime", "9:", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time ":30""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_30() {
    assert_eq!(parse_at("datetime", ":30", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:135 "parseLooseTemporalValue - time-only values > returns null for the invalid time "1030""
#[test]
fn parseloosetemporalvalue_time_only_values_returns_null_for_the_invalid_time_1030() {
    assert_eq!(parse_at("datetime", "1030", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:149 "parseLooseTemporalValue - time-only values > a full date-time still wins over the time-only fallback"
#[test]
fn parseloosetemporalvalue_time_only_values_a_full_date_time_still_wins_over_the_time_only_fallback()
 {
    assert_eq!(
        parse_at("datetime", "23/07/2026 09:30", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30 AM" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30_am_2026_07_23t09_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 AM", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30 am" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30_am_2026_07_23t09_30_00_2() {
    assert_eq!(
        parse_at("datetime", "9:30 am", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30 p.m." -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30_p_m_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 p.m.", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30AM" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30am_2026_07_23t09_30_00() {
    assert_eq!(parse_at("datetime", "9:30AM", VN), s("2026-07-23T09:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30pm" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30pm_2026_07_23t21_30_00() {
    assert_eq!(parse_at("datetime", "9:30pm", VN), s("2026-07-23T21:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9 PM" -> "2026-07-23T21:00:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_pm_2026_07_23t21_00_00() {
    assert_eq!(parse_at("datetime", "9 PM", VN), s("2026-07-23T21:00:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "9:30:45 PM" -> "2026-07-23T21:30:45""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_9_30_45_pm_2026_07_23t21_30_45() {
    assert_eq!(
        parse_at("datetime", "9:30:45 PM", VN),
        s("2026-07-23T21:30:45")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "12:00 AM" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_12_00_am_2026_07_23t00_00_00() {
    assert_eq!(
        parse_at("datetime", "12:00 AM", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:164 "parseLooseTemporalValue - meridiem (AM/PM) inputs > time-only "12:00 PM" -> "2026-07-23T12:00:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_time_only_12_00_pm_2026_07_23t12_00_00() {
    assert_eq!(
        parse_at("datetime", "12:00 PM", VN),
        s("2026-07-23T12:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "23/07/2026 09:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_23_07_2026_09_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "23/07/2026 09:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "07/23/2026 9:30 AM" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_07_23_2026_9_30_am_2026_07_23t09_30_00() {
    assert_eq!(
        parse("datetime", "07/23/2026 9:30 AM", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "2026-07-23 09:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_2026_07_23_09_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "2026-07-23 09:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "2026-07-23T09:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_2026_07_23t09_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "2026-07-23T09:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "23-07-2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_23_07_2026_9_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "23-07-2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "23.07.2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_23_07_2026_9_30_pm_2026_07_23t21_30_00_2() {
    assert_eq!(
        parse("datetime", "23.07.2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "Jul 23, 2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_jul_23_2026_9_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "Jul 23, 2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "23 Jul 2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_23_jul_2026_9_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "23 Jul 2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "July 23, 2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_july_23_2026_9_30_pm_2026_07_23t21_30_00() {
    assert_eq!(
        parse("datetime", "July 23, 2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "Jul 23, 2026 9:30:45 PM" -> "2026-07-23T21:30:45""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_jul_23_2026_9_30_45_pm_2026_07_23t21_30_45()
{
    assert_eq!(
        parse("datetime", "Jul 23, 2026 9:30:45 PM", VN),
        s("2026-07-23T21:30:45")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "23 tháng 7 2026 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_23_th_ng_7_2026_9_30_pm_2026_07_23t21_30_00()
{
    assert_eq!(
        parse("datetime", "23 tháng 7 2026 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:183 "parseLooseTemporalValue - meridiem (AM/PM) inputs > dated "ngày 23 tháng 7 năm 2026 lúc 9:30 PM" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_dated_ng_y_23_th_ng_7_n_m_2026_l_c_9_30_pm_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "ngày 23 tháng 7 năm 2026 lúc 9:30 PM", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:201 "parseLooseTemporalValue - meridiem (AM/PM) inputs > an ambiguous day reads the same with a 24-hour or a meridiem clock"
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_an_ambiguous_day_reads_the_same_with_a_24_hour_or_a_meridiem_clock()
 {
    assert_eq!(parse("date", "07/12/2026", VN), s("2026-12-07"));
    assert_eq!(parse("date", "07/12/2026 09:30", VN), s("2026-12-07"));
    assert_eq!(parse("date", "07/12/2026 9:30 AM", VN), s("2026-12-07"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:214 "parseLooseTemporalValue - meridiem (AM/PM) inputs > meridiem normalization leaves non-meridiem text alone"
#[test]
fn parseloosetemporalvalue_meridiem_am_pm_inputs_meridiem_normalization_leaves_non_meridiem_text_alone()
 {
    assert_eq!(parse("datetime", "12:30 XM", VN), None);
    assert_eq!(parse("datetime", "diagram", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 SA" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_sa_2026_07_23t09_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 SA", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 CH" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_ch_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 CH", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 sa" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_sa_2026_07_23t09_30_00_2() {
    assert_eq!(
        parse_at("datetime", "9:30 sa", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 ch" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_ch_2026_07_23t21_30_00_2() {
    assert_eq!(
        parse_at("datetime", "9:30 ch", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30SA" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30sa_2026_07_23t09_30_00() {
    assert_eq!(parse_at("datetime", "9:30SA", VN), s("2026-07-23T09:30:00"));
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "12:00 SA" -> "2026-07-23T00:00:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_12_00_sa_2026_07_23t00_00_00() {
    assert_eq!(
        parse_at("datetime", "12:00 SA", VN),
        s("2026-07-23T00:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "12:00 CH" -> "2026-07-23T12:00:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_12_00_ch_2026_07_23t12_00_00() {
    assert_eq!(
        parse_at("datetime", "12:00 CH", VN),
        s("2026-07-23T12:00:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 sáng" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_s_ng_2026_07_23t09_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 sáng", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 sang" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_sang_2026_07_23t09_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 sang", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 chiều" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_chi_u_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 chiều", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 chieu" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_chieu_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 chieu", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 tối" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_t_i_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 tối", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:221 "parseLooseTemporalValue - Vietnamese meridiem markers > time-only "9:30 toi" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_time_only_9_30_toi_2026_07_23t21_30_00() {
    assert_eq!(
        parse_at("datetime", "9:30 toi", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "23/07/2026 9:30 CH" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_23_07_2026_9_30_ch_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "23/07/2026 9:30 CH", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "23/07/2026 9:30 SA" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_23_07_2026_9_30_sa_2026_07_23t09_30_00()
 {
    assert_eq!(
        parse("datetime", "23/07/2026 9:30 SA", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "2026-07-23 9:30 CH" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_2026_07_23_9_30_ch_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "2026-07-23 9:30 CH", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "23 thg 7 2026 9:30 CH" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_23_thg_7_2026_9_30_ch_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "23 thg 7 2026 9:30 CH", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "ngày 23 tháng 7 năm 2026 lúc 9:30 CH" -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_ng_y_23_th_ng_7_n_m_2026_l_c_9_30_ch_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "ngày 23 tháng 7 năm 2026 lúc 9:30 CH", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "ngày 23 tháng 7 năm 2026 lúc 9:30 p.m." -> "2026-07-23T21:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_ng_y_23_th_ng_7_n_m_2026_l_c_9_30_p_m_2026_07_23t21_30_00()
 {
    assert_eq!(
        parse("datetime", "ngày 23 tháng 7 năm 2026 lúc 9:30 p.m.", VN),
        s("2026-07-23T21:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:243 "parseLooseTemporalValue - Vietnamese meridiem markers > dated "ngày 23 tháng 7 năm 2026 lúc 9:30 SA" -> "2026-07-23T09:30:00""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_dated_ng_y_23_th_ng_7_n_m_2026_l_c_9_30_sa_2026_07_23t09_30_00()
 {
    assert_eq!(
        parse("datetime", "ngày 23 tháng 7 năm 2026 lúc 9:30 SA", VN),
        s("2026-07-23T09:30:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:258 "parseLooseTemporalValue - Vietnamese meridiem markers > rejects the ambiguous day period "9:30 trưa""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_rejects_the_ambiguous_day_period_9_30_tr_a()
{
    assert_eq!(parse_at("datetime", "9:30 trưa", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:258 "parseLooseTemporalValue - Vietnamese meridiem markers > rejects the ambiguous day period "9:30 đêm""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_rejects_the_ambiguous_day_period_9_30_m() {
    assert_eq!(parse_at("datetime", "9:30 đêm", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:258 "parseLooseTemporalValue - Vietnamese meridiem markers > rejects the ambiguous day period "9:30 trua""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_rejects_the_ambiguous_day_period_9_30_trua()
{
    assert_eq!(parse_at("datetime", "9:30 trua", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:258 "parseLooseTemporalValue - Vietnamese meridiem markers > rejects the ambiguous day period "9:30 dem""
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_rejects_the_ambiguous_day_period_9_30_dem() {
    assert_eq!(parse_at("datetime", "9:30 dem", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:272 "parseLooseTemporalValue - Vietnamese meridiem markers > Vietnamese markers do not fire without a preceding clock digit"
#[test]
fn parseloosetemporalvalue_vietnamese_meridiem_markers_vietnamese_markers_do_not_fire_without_a_preceding_clock_digit()
 {
    assert_eq!(parse("datetime", "sang", VN), None);
    assert_eq!(parse("datetime", "March", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for datetime """
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_datetime() {
    assert_eq!(parse("datetime", "", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for datetime "   ""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_datetime_2() {
    assert_eq!(parse("datetime", "   ", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for datetime "not-a-date""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_datetime_not_a_date() {
    assert_eq!(parse("datetime", "not-a-date", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for date "hello world""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_date_hello_world() {
    assert_eq!(parse("date", "hello world", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for date "2026-02-30""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_date_2026_02_30() {
    assert_eq!(parse("date", "2026-02-30", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for date "32/01/2026""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_date_32_01_2026() {
    assert_eq!(parse("date", "32/01/2026", VN), None);
}

/// Port of: packages/utils/__tests__/temporal-input.test.ts:279 "parseLooseTemporalValue - unrecognized input > returns null for datetime "2026-13-01""
#[test]
fn parseloosetemporalvalue_unrecognized_input_returns_null_for_datetime_2026_13_01() {
    assert_eq!(parse("datetime", "2026-13-01", VN), None);
}
