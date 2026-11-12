//! Port of `packages/utils/__tests__/whatsapp-call-hours.test.ts`. The
//! schedule is its jsonb value (`serde_json::Value`); `Date` is epoch ms.

use chatbotx_core::js::date_utc;
use chatbotx_core::utils::whatsapp_call_hours::{is_within_call_hours, parse_call_time};
use serde_json::{Value, json};

fn schedule(over: Value) -> Value {
    let mut base = json!({
        "status": "ENABLED",
        "timezoneId": "Asia/Ho_Chi_Minh",
        "weeklyOperatingHours": [{ "dayOfWeek": "MONDAY", "openTime": "0900", "closeTime": "1700" }],
    });
    for (key, value) in over.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

/// A Monday (2026-09-14), in UTC. Vietnam is UTC+7.
fn monday_utc(hour: i64, minute: i64) -> i64 {
    date_utc(2026, 8, 14, hour, minute, 0, 0)
}

fn utc(day: i64, hour: i64, minute: i64) -> i64 {
    date_utc(2026, 8, day, hour, minute, 0, 0)
}

fn overnight(days: &[&str]) -> Value {
    let windows: Vec<Value> = days
        .iter()
        .map(|day| json!({ "dayOfWeek": day, "openTime": "2200", "closeTime": "0200" }))
        .collect();
    schedule(json!({ "timezoneId": "Etc/UTC", "weeklyOperatingHours": windows }))
}

fn with_holiday(holidays: Value) -> Value {
    schedule(json!({ "timezoneId": "Etc/UTC", "holidaySchedule": holidays }))
}

fn london() -> Value {
    schedule(json!({
        "timezoneId": "Europe/London",
        "weeklyOperatingHours": [{ "dayOfWeek": "MONDAY", "openTime": "0900", "closeTime": "1700" }],
    }))
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:24 "parseCallTime > reads 0900 as 540 minutes"
#[test]
fn parsecalltime_reads_0900_as_540_minutes() {
    assert_eq!(parse_call_time("0900"), Some(540));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:24 "parseCallTime > reads 09:00 as 540 minutes"
#[test]
fn parsecalltime_reads_09_00_as_540_minutes() {
    assert_eq!(parse_call_time("09:00"), Some(540));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:24 "parseCallTime > reads 0000 as 0 minutes"
#[test]
fn parsecalltime_reads_0000_as_0_minutes() {
    assert_eq!(parse_call_time("0000"), Some(0));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:24 "parseCallTime > reads 2359 as 1439 minutes"
#[test]
fn parsecalltime_reads_2359_as_1439_minutes() {
    assert_eq!(parse_call_time("2359"), Some(1439));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:33 "parseCallTime > rejects  rather than guessing"
#[test]
fn parsecalltime_rejects_rather_than_guessing() {
    assert_eq!(parse_call_time(""), None);
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:33 "parseCallTime > rejects abcd rather than guessing"
#[test]
fn parsecalltime_rejects_abcd_rather_than_guessing() {
    assert_eq!(parse_call_time("abcd"), None);
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:33 "parseCallTime > rejects 2460 rather than guessing"
#[test]
fn parsecalltime_rejects_2460_rather_than_guessing() {
    assert_eq!(parse_call_time("2460"), None);
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:33 "parseCallTime > rejects 0960 rather than guessing"
#[test]
fn parsecalltime_rejects_0960_rather_than_guessing() {
    assert_eq!(parse_call_time("0960"), None);
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:33 "parseCallTime > rejects 9 rather than guessing"
#[test]
fn parsecalltime_rejects_9_rather_than_guessing() {
    assert_eq!(parse_call_time("9"), None);
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:47 "isWithinCallHours — open by default > no schedule at all accepts calls"
#[test]
fn iswithincallhours_open_by_default_no_schedule_at_all_accepts_calls() {
    assert!(is_within_call_hours(None, monday_utc(20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:47 "isWithinCallHours — open by default > an undefined schedule accepts calls"
#[test]
fn iswithincallhours_open_by_default_an_undefined_schedule_accepts_calls() {
    assert!(is_within_call_hours(None, monday_utc(20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:54 "isWithinCallHours — open by default > a DISABLED schedule accepts calls even outside its windows"
#[test]
fn iswithincallhours_open_by_default_a_disabled_schedule_accepts_calls_even_outside_its_windows() {
    assert!(is_within_call_hours(
        Some(&schedule(json!({ "status": "DISABLED" }))),
        monday_utc(20, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:60 "isWithinCallHours — open by default > an ENABLED schedule with no windows accepts calls"
#[test]
fn iswithincallhours_open_by_default_an_enabled_schedule_with_no_windows_accepts_calls() {
    assert!(is_within_call_hours(
        Some(&schedule(json!({ "weeklyOperatingHours": [] }))),
        monday_utc(20, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:66 "isWithinCallHours — open by default > an unknown timezone accepts calls instead of locking customers out"
#[test]
fn iswithincallhours_open_by_default_an_unknown_timezone_accepts_calls_instead_of_locking_customers_out()
 {
    assert!(is_within_call_hours(
        Some(&schedule(json!({ "timezoneId": "Not/AZone" }))),
        monday_utc(20, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:72 "isWithinCallHours — open by default > an unparseable window accepts calls rather than closing the number"
#[test]
fn iswithincallhours_open_by_default_an_unparseable_window_accepts_calls_rather_than_closing_the_number()
 {
    let s = schedule(json!({
        "weeklyOperatingHours": [{ "dayOfWeek": "MONDAY", "openTime": "nope", "closeTime": "1700" }],
    }));
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:88 "isWithinCallHours — open by default > a non-array weeklyOperatingHours accepts calls instead of throwing"
#[test]
fn iswithincallhours_open_by_default_a_non_array_weeklyoperatinghours_accepts_calls_instead_of_throwing()
 {
    let s = schedule(json!({ "weeklyOperatingHours": { "foo": "bar" } }));
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:88 "isWithinCallHours — open by default > a null entry accepts calls instead of throwing"
#[test]
fn iswithincallhours_open_by_default_a_null_entry_accepts_calls_instead_of_throwing() {
    let s = schedule(json!({ "weeklyOperatingHours": [null] }));
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:88 "isWithinCallHours — open by default > a primitive entry accepts calls instead of throwing"
#[test]
fn iswithincallhours_open_by_default_a_primitive_entry_accepts_calls_instead_of_throwing() {
    let s = schedule(json!({ "weeklyOperatingHours": ["MONDAY"] }));
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:88 "isWithinCallHours — open by default > an entry with no day accepts calls instead of throwing"
#[test]
fn iswithincallhours_open_by_default_an_entry_with_no_day_accepts_calls_instead_of_throwing() {
    let s =
        schedule(json!({ "weeklyOperatingHours": [{ "openTime": "0900", "closeTime": "1700" }] }));
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:88 "isWithinCallHours — open by default > an entry with numeric times accepts calls instead of throwing"
#[test]
fn iswithincallhours_open_by_default_an_entry_with_numeric_times_accepts_calls_instead_of_throwing()
{
    let s = schedule(
        json!({ "weeklyOperatingHours": [{ "dayOfWeek": "MONDAY", "openTime": 900, "closeTime": 1700 }] }),
    );
    assert!(is_within_call_hours(Some(&s), monday_utc(3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:111 "isWithinCallHours — open by default > a day name Meta does not define accepts calls instead of closing the week"
#[test]
fn iswithincallhours_open_by_default_a_day_name_meta_does_not_define_accepts_calls_instead_of_closing_the_week()
 {
    let s = schedule(json!({
        "weeklyOperatingHours": [{ "dayOfWeek": "FUNDAY", "openTime": "0900", "closeTime": "1700" }],
    }));
    assert!(is_within_call_hours(Some(&s), monday_utc(20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:126 "isWithinCallHours — open by default > a valid window still decides when a malformed one sits beside it"
#[test]
fn iswithincallhours_open_by_default_a_valid_window_still_decides_when_a_malformed_one_sits_beside_it()
 {
    let s = schedule(json!({
        "weeklyOperatingHours": [
            { "dayOfWeek": "MONDAY", "openTime": "0900", "closeTime": "1700" },
            { "dayOfWeek": "TUESDAY", "openTime": "??", "closeTime": "??" },
        ],
    }));
    assert!(!is_within_call_hours(Some(&s), monday_utc(1, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:144 "isWithinCallHours — the schedule is read in its own timezone > 02:00 UTC is 09:00 local — open"
#[test]
fn iswithincallhours_the_schedule_is_read_in_its_own_timezone_02_00_utc_is_09_00_local_open() {
    assert!(is_within_call_hours(
        Some(&schedule(json!({}))),
        monday_utc(2, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:148 "isWithinCallHours — the schedule is read in its own timezone > 09:59 UTC is 16:59 local — still open"
#[test]
fn iswithincallhours_the_schedule_is_read_in_its_own_timezone_09_59_utc_is_16_59_local_still_open()
{
    assert!(is_within_call_hours(
        Some(&schedule(json!({}))),
        monday_utc(9, 59)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:152 "isWithinCallHours — the schedule is read in its own timezone > 10:00 UTC is 17:00 local — closed, the close minute is exclusive"
#[test]
fn iswithincallhours_the_schedule_is_read_in_its_own_timezone_10_00_utc_is_17_00_local_closed_the_close_minute_is_exclusive()
 {
    assert!(!is_within_call_hours(
        Some(&schedule(json!({}))),
        monday_utc(10, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:156 "isWithinCallHours — the schedule is read in its own timezone > 01:59 UTC is 08:59 local — not open yet"
#[test]
fn iswithincallhours_the_schedule_is_read_in_its_own_timezone_01_59_utc_is_08_59_local_not_open_yet()
 {
    assert!(!is_within_call_hours(
        Some(&schedule(json!({}))),
        monday_utc(1, 59)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:160 "isWithinCallHours — the schedule is read in its own timezone > a day with no window is closed all day"
#[test]
fn iswithincallhours_the_schedule_is_read_in_its_own_timezone_a_day_with_no_window_is_closed_all_day()
 {
    assert!(!is_within_call_hours(
        Some(&schedule(json!({}))),
        date_utc(2026, 8, 15, 2, 0, 0, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:182 "isWithinCallHours — windows that run past midnight > open on the evening side of its own day"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_open_on_the_evening_side_of_its_own_day() {
    assert!(is_within_call_hours(
        Some(&overnight(&["MONDAY"])),
        utc(14, 23, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:186 "isWithinCallHours — windows that run past midnight > open on the after-midnight side, which falls on the NEXT day"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_open_on_the_after_midnight_side_which_falls_on_the_next_day()
 {
    assert!(is_within_call_hours(
        Some(&overnight(&["MONDAY"])),
        utc(15, 1, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:190 "isWithinCallHours — windows that run past midnight > closed once the spillover has passed"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_closed_once_the_spillover_has_passed() {
    assert!(!is_within_call_hours(
        Some(&overnight(&["MONDAY"])),
        utc(15, 3, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:194 "isWithinCallHours — windows that run past midnight > closed in the gap before it opens"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_closed_in_the_gap_before_it_opens() {
    assert!(!is_within_call_hours(
        Some(&overnight(&["MONDAY"])),
        utc(14, 21, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:201 "isWithinCallHours — windows that run past midnight > does NOT open the early hours of its own start day"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_does_not_open_the_early_hours_of_its_own_start_day()
 {
    assert!(!is_within_call_hours(
        Some(&overnight(&["MONDAY"])),
        utc(14, 1, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:205 "isWithinCallHours — windows that run past midnight > the previous day's window is what opens those early hours"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_the_previous_day_s_window_is_what_opens_those_early_hours()
 {
    assert!(is_within_call_hours(
        Some(&overnight(&["SUNDAY"])),
        utc(14, 1, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:209 "isWithinCallHours — windows that run past midnight > wraps across the week boundary — Sunday into Monday"
#[test]
fn iswithincallhours_windows_that_run_past_midnight_wraps_across_the_week_boundary_sunday_into_monday()
 {
    assert!(is_within_call_hours(
        Some(&overnight(&["SUNDAY"])),
        utc(13, 23, 0)
    ));
    assert!(is_within_call_hours(
        Some(&overnight(&["SUNDAY"])),
        utc(14, 1, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:225 "isWithinCallHours — holiday overrides > a shortened holiday closes hours the weekly schedule would have opened"
#[test]
fn iswithincallhours_holiday_overrides_a_shortened_holiday_closes_hours_the_weekly_schedule_would_have_opened()
 {
    let s = with_holiday(json!([{ "date": "2026-09-14", "startTime": "0900", "endTime": "1200" }]));
    assert!(!is_within_call_hours(Some(&s), utc(14, 14, 0)));
    assert!(is_within_call_hours(Some(&s), utc(14, 10, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:235 "isWithinCallHours — holiday overrides > an all-day holiday stays open through its final minute"
#[test]
fn iswithincallhours_holiday_overrides_an_all_day_holiday_stays_open_through_its_final_minute() {
    let s = with_holiday(json!([{ "date": "2026-09-14", "startTime": "0000", "endTime": "2359" }]));
    assert!(is_within_call_hours(Some(&s), utc(14, 23, 59)));
    assert!(is_within_call_hours(Some(&s), utc(14, 3, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:246 "isWithinCallHours — holiday overrides > a holiday on another date leaves the weekly schedule alone"
#[test]
fn iswithincallhours_holiday_overrides_a_holiday_on_another_date_leaves_the_weekly_schedule_alone()
{
    let s = with_holiday(json!([{ "date": "2026-12-25", "startTime": "0000", "endTime": "0000" }]));
    assert!(is_within_call_hours(Some(&s), utc(14, 10, 0)));
    assert!(!is_within_call_hours(Some(&s), utc(14, 20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:255 "isWithinCallHours — holiday overrides > the holiday date is read in the schedule's own timezone"
#[test]
fn iswithincallhours_holiday_overrides_the_holiday_date_is_read_in_the_schedule_s_own_timezone() {
    let s = schedule(json!({
        "timezoneId": "Pacific/Auckland",
        "weeklyOperatingHours": [{ "dayOfWeek": "TUESDAY", "openTime": "0900", "closeTime": "1700" }],
        "holidaySchedule": [{ "date": "2026-09-15", "startTime": "0000", "endTime": "0100" }],
    }));
    assert!(!is_within_call_hours(Some(&s), utc(14, 18, 0)));
    assert!(is_within_call_hours(Some(&s), utc(14, 12, 30)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:278 "isWithinCallHours — holiday overrides > a reversed range accepts calls all day"
#[test]
fn iswithincallhours_holiday_overrides_a_reversed_range_accepts_calls_all_day() {
    let s = with_holiday(json!([{ "date": "2026-09-14", "startTime": "2200", "endTime": "0200" }]));
    assert!(is_within_call_hours(Some(&s), utc(14, 5, 0)));
    assert!(is_within_call_hours(Some(&s), utc(14, 20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:278 "isWithinCallHours — holiday overrides > an empty range accepts calls all day"
#[test]
fn iswithincallhours_holiday_overrides_an_empty_range_accepts_calls_all_day() {
    let s = with_holiday(json!([{ "date": "2026-09-14", "startTime": "0900", "endTime": "0900" }]));
    assert!(is_within_call_hours(Some(&s), utc(14, 5, 0)));
    assert!(is_within_call_hours(Some(&s), utc(14, 20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:288 "isWithinCallHours — holiday overrides > an unreadable holiday accepts calls rather than closing the number"
#[test]
fn iswithincallhours_holiday_overrides_an_unreadable_holiday_accepts_calls_rather_than_closing_the_number()
 {
    let s = with_holiday(json!([{ "date": "2026-09-14", "startTime": "??", "endTime": "??" }]));
    assert!(is_within_call_hours(Some(&s), utc(14, 20, 0)));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:307 "isWithinCallHours — daylight saving > summer: 08:00 UTC is 09:00 local — open"
#[test]
fn iswithincallhours_daylight_saving_summer_08_00_utc_is_09_00_local_open() {
    assert!(is_within_call_hours(
        Some(&london()),
        date_utc(2026, 6, 13, 8, 0, 0, 0)
    ));
}

/// Port of: packages/utils/__tests__/whatsapp-call-hours.test.ts:313 "isWithinCallHours — daylight saving > winter: 08:00 UTC is 08:00 local — not open yet"
#[test]
fn iswithincallhours_daylight_saving_winter_08_00_utc_is_08_00_local_not_open_yet() {
    assert!(!is_within_call_hours(
        Some(&london()),
        date_utc(2026, 0, 12, 8, 0, 0, 0)
    ));
}
