//! Port of `packages/sequence-scheduler/__tests__/send-time-validator.test.ts`
//! (spec 5.2). The TypeScript test builds local-time dates in the process
//! zone; the Rust fn takes the zone, and the tests pass UTC.

use chatbotx_jobs::sequence::send_time::{SendWindow, calculate_next_valid_send_time};
use chrono::TimeZone;
use chrono_tz::Tz;

const TZ: Tz = chrono_tz::UTC;

/// `new Date(y, m0, d, h, min, 0, 0)` in `TZ` (month is 0-based as in JS).
fn local(year: i32, month0: u32, day: u32, hour: u32, minute: u32) -> i64 {
    TZ.with_ymd_and_hms(year, month0 + 1, day, hour, minute, 0)
        .single()
        .expect("valid local time")
        .timestamp_millis()
}

fn window(anytime: bool, days: Option<&str>, start: Option<&str>, end: Option<&str>) -> SendWindow {
    SendWindow {
        anytime,
        send_days: days.map(str::to_owned),
        send_time_start: start.map(str::to_owned),
        send_time_end: end.map(str::to_owned),
    }
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:10 "calculateNextValidSendTime > returns baseTime unchanged when anytime is true"
#[test]
fn returns_basetime_unchanged_when_anytime_is_true() {
    let base = local(2024, 0, 1, 10, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(true, Some(r#"["monday"]"#), Some("09:00"), Some("17:00")),
        TZ,
    );

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:26 "calculateNextValidSendTime > rolls to startOfDay next allowed day when current day is not in sendDays"
#[test]
fn rolls_to_startofday_next_allowed_day_when_current_day_is() {
    let base = local(2024, 0, 1, 10, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(false, Some(r#"["tuesday"]"#), None, None),
        TZ,
    );

    assert_eq!(result, local(2024, 0, 2, 0, 0));
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:45 "calculateNextValidSendTime > snaps to window start time when current time is before the send window"
#[test]
fn snaps_to_window_start_time_when_current_time_is_before() {
    let base = local(2024, 0, 1, 7, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(false, Some(r#"["monday"]"#), Some("09:00"), Some("17:00")),
        TZ,
    );

    assert_eq!(result, local(2024, 0, 1, 9, 0));
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:64 "calculateNextValidSendTime > snaps to window start on next allowed day when time equals window end"
#[test]
fn snaps_to_window_start_on_next_allowed_day_when_time_equals() {
    let base = local(2024, 0, 1, 17, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(
            false,
            Some(r#"["monday","tuesday"]"#),
            Some("09:00"),
            Some("17:00"),
        ),
        TZ,
    );

    assert_eq!(result, local(2024, 0, 2, 9, 0));
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:84 "calculateNextValidSendTime > snaps to window start on next allowed day when time is after window end"
#[test]
fn snaps_to_window_start_on_next_allowed_day_when_time_is() {
    let base = local(2024, 0, 1, 20, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(
            false,
            Some(r#"["monday","tuesday"]"#),
            Some("09:00"),
            Some("17:00"),
        ),
        TZ,
    );

    assert_eq!(result, local(2024, 0, 2, 9, 0));
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:102 "calculateNextValidSendTime > returns unchanged time when time is inside the send window"
#[test]
fn returns_unchanged_time_when_time_is_inside_the_send_window() {
    let base = local(2024, 0, 1, 12, 30);

    let result = calculate_next_valid_send_time(
        base,
        &window(false, Some(r#"["monday"]"#), Some("09:00"), Some("17:00")),
        TZ,
    );

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:119 "calculateNextValidSendTime > returns unchanged time when no time window is set and day is allowed"
#[test]
fn returns_unchanged_time_when_no_time_window_is_set_and_day() {
    let base = local(2024, 0, 1, 5, 0);

    let result =
        calculate_next_valid_send_time(base, &window(false, Some(r#"["monday"]"#), None, None), TZ);

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:135 "calculateNextValidSendTime > falls back to ALL_DAYS when sendDays is invalid JSON"
#[test]
fn falls_back_to_all_days_when_senddays_is_invalid_json() {
    let base = local(2024, 0, 1, 10, 0);

    let result = calculate_next_valid_send_time(
        base,
        &window(false, Some("not-valid-json"), None, None),
        TZ,
    );

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:152 "calculateNextValidSendTime > allows all days when sendDays is null"
#[test]
fn allows_all_days_when_senddays_is_null() {
    // Saturday Jan 6 2024
    let base = local(2024, 0, 6, 14, 0);

    let result = calculate_next_valid_send_time(base, &window(false, None, None, None), TZ);

    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:170 "calculateNextValidSendTime > returns original baseTime after MAX_ATTEMPTS when sendDays is empty array"
#[test]
fn returns_original_basetime_after_max_attempts_when_senddays() {
    let base = local(2024, 0, 1, 10, 0);

    let result = calculate_next_valid_send_time(base, &window(false, Some("[]"), None, None), TZ);

    assert_eq!(chatbotx_jobs::sequence::send_time::MAX_ATTEMPTS, 14);
    assert_eq!(result, base);
}

/// Port of: packages/sequence-scheduler/__tests__/send-time-validator.test.ts:188 "calculateNextValidSendTime > skips multiple non-allowed days before landing on an allowed one"
#[test]
fn skips_multiple_non_allowed_days_before_landing_on_an() {
    let base = local(2024, 0, 1, 10, 0);

    let result =
        calculate_next_valid_send_time(base, &window(false, Some(r#"["friday"]"#), None, None), TZ);

    assert_eq!(result, local(2024, 0, 5, 0, 0));
}
