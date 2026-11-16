//! Ported from `packages/business/__tests__/workspace-deletion-schedule.test.ts`.

use chatbotx_business::workspace::deletion_schedule::{
    WORKSPACE_DELETION_GRACE_MS, ceil_to_purge_boundary, next_scheduled_deletion_at,
};

fn ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .expect("iso")
        .timestamp_millis()
}

#[test]
fn rounds_up_to_the_next_boundary() {
    for (input, expected) in [
        ("2026-08-06T10:20:00Z", "2026-08-06T10:30:00Z"),
        ("2026-08-06T10:31:00Z", "2026-08-06T11:00:00Z"),
        ("2026-08-06T10:00:00.001Z", "2026-08-06T10:30:00Z"),
    ] {
        assert_eq!(ceil_to_purge_boundary(ms(input)), ms(expected), "{input}");
    }
}

#[test]
fn keeps_a_timestamp_already_on_a_boundary_unchanged() {
    let on_boundary = ms("2026-08-06T10:30:00Z");
    assert_eq!(ceil_to_purge_boundary(on_boundary), on_boundary);
}

#[test]
fn result_always_lands_on_a_30_minute_boundary() {
    let result = ceil_to_purge_boundary(ms("2026-08-06T10:07:23.456Z"));
    let date = chrono::DateTime::from_timestamp_millis(result).expect("date");
    use chrono::Timelike as _;
    assert_eq!(date.minute() % 30, 0);
    assert_eq!(date.second(), 0);
    assert_eq!(date.timestamp_subsec_millis(), 0);
}

#[test]
fn returns_now_plus_24h_grace_rounded_up_to_the_purge_boundary() {
    let now = ms("2026-08-06T10:20:00Z");
    assert_eq!(next_scheduled_deletion_at(now), ms("2026-08-07T10:30:00Z"));
}

#[test]
fn keeps_grace_within_a_predictable_one_day_window() {
    let now = ms("2026-08-06T10:20:00Z");
    let grace = next_scheduled_deletion_at(now) - now;
    assert!(grace >= WORKSPACE_DELETION_GRACE_MS);
    assert!(grace <= WORKSPACE_DELETION_GRACE_MS + 30 * 60 * 1000);
}
