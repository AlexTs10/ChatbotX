//! Port of `packages/database/__tests__/get-safe-since-time.test.ts` (P3.3).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::iso;
use chatbotx_db::repo::message::{SinceTimeInput, get_safe_since_time};
use repo_common::ms;

/// Port of: packages/database/__tests__/get-safe-since-time.test.ts:5 "getSafeSinceTime > accepts Date instances"
#[test]
fn get_safe_since_time_accepts_date_instances() {
    let result = get_safe_since_time(
        Some(SinceTimeInput::Date(ms("2026-07-15T10:34:56.000Z"))),
        None,
    );
    assert_eq!(result.map(iso).as_deref(), Some("2026-07-15T09:00:00.000Z"));
}

/// Port of: packages/database/__tests__/get-safe-since-time.test.ts:11 "getSafeSinceTime > accepts ISO strings from JSON payloads"
#[test]
fn get_safe_since_time_accepts_iso_strings_from_json_payloads() {
    let result = get_safe_since_time(Some(SinceTimeInput::Text("2026-07-15T10:34:56.000Z")), None);
    assert_eq!(result.map(iso).as_deref(), Some("2026-07-15T09:00:00.000Z"));
}

/// Port of: packages/database/__tests__/get-safe-since-time.test.ts:17 "getSafeSinceTime > accepts numeric timestamps"
#[test]
fn get_safe_since_time_accepts_numeric_timestamps() {
    let timestamp = ms("2026-07-15T10:34:56.000Z");
    let result = get_safe_since_time(Some(SinceTimeInput::Ms(timestamp)), None);
    assert_eq!(result.map(iso).as_deref(), Some("2026-07-15T09:00:00.000Z"));
}

/// Port of: packages/database/__tests__/get-safe-since-time.test.ts:24 "getSafeSinceTime > returns undefined for invalid strings"
#[test]
fn get_safe_since_time_returns_undefined_for_invalid_strings() {
    assert_eq!(
        get_safe_since_time(Some(SinceTimeInput::Text("not-a-date")), None),
        None
    );
}

/// Port of: packages/database/__tests__/get-safe-since-time.test.ts:28 "getSafeSinceTime > applies an explicit buffer before flooring to the hour"
#[test]
fn get_safe_since_time_applies_an_explicit_buffer_before_flooring_to_the_hour() {
    let result = get_safe_since_time(
        Some(SinceTimeInput::Text("2026-07-15T10:34:56.000Z")),
        Some(60 * 60 * 1000),
    );
    assert_eq!(result.map(iso).as_deref(), Some("2026-07-15T09:00:00.000Z"));
}
