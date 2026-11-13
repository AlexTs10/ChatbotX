//! Port of `packages/business/src/ecommerce-open-hours/__tests__/is-open.test.ts`.

use chatbotx_business::ecommerce_open_hours::{OpenHoursRange, is_within_open_hours};

fn at(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("ISO date")
}

fn range(day: u32, from: &str, to: &str) -> OpenHoursRange {
    OpenHoursRange {
        day,
        from: from.to_string(),
        to: to.to_string(),
    }
}

// 2026-10-19 is a Monday.

#[test]
fn is_within_open_hours_is_open_inside_the_range_in_the_workspace_timezone() {
    let weekdays = [range(1, "09:00", "17:00")];
    // 09:30 in Ho Chi Minh City is 02:30 UTC.
    assert!(is_within_open_hours(
        &weekdays,
        "Asia/Ho_Chi_Minh",
        at("2026-10-19T02:30:00Z")
    ));
}

#[test]
fn is_within_open_hours_is_closed_before_the_range_and_at_its_end() {
    let weekdays = [range(1, "09:00", "17:00")];
    assert!(!is_within_open_hours(
        &weekdays,
        "Asia/Ho_Chi_Minh",
        at("2026-10-19T01:59:00Z")
    ));
    assert!(!is_within_open_hours(
        &weekdays,
        "Asia/Ho_Chi_Minh",
        at("2026-10-19T10:00:00Z")
    ));
}

#[test]
fn is_within_open_hours_is_closed_on_another_day() {
    let weekdays = [range(1, "09:00", "17:00")];
    assert!(!is_within_open_hours(
        &weekdays,
        "UTC",
        at("2026-10-20T10:00:00Z")
    ));
}

#[test]
fn is_within_open_hours_a_range_past_midnight_covers_the_early_hours_of_the_next_day() {
    let late = [range(1, "22:00", "02:00")];
    assert!(is_within_open_hours(
        &late,
        "UTC",
        at("2026-10-19T23:00:00Z")
    ));
    assert!(is_within_open_hours(
        &late,
        "UTC",
        at("2026-10-20T01:30:00Z")
    ));
    assert!(!is_within_open_hours(
        &late,
        "UTC",
        at("2026-10-20T02:00:00Z")
    ));
    assert!(!is_within_open_hours(
        &late,
        "UTC",
        at("2026-10-19T21:00:00Z")
    ));
}

#[test]
fn is_within_open_hours_no_ranges_is_never_open() {
    assert!(!is_within_open_hours(
        &[],
        "UTC",
        at("2026-10-19T10:00:00Z")
    ));
}
