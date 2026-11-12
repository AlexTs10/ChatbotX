//! Port of `packages/utils/__tests__/form-answer-format.test.ts`.

use chatbotx_core::utils::form_answer_format::format_date_range;
use serde_json::json;

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:5 "formatDateRange > a CalendarPicker range answer becomes start - end"
#[test]
fn formatdaterange_a_calendarpicker_range_answer_becomes_start_end() {
    assert_eq!(
        format_date_range(&json!({ "start-date": "2024-10-21", "end-date": "2024-10-25" }))
            .as_deref(),
        Some("2024-10-21 - 2024-10-25")
    );
}

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:11 "formatDateRange > a date string is not a range"
#[test]
fn formatdaterange_a_date_string_is_not_a_range() {
    assert_eq!(format_date_range(&json!("2024-10-21")), None);
}

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:11 "formatDateRange > a list is not a range"
#[test]
fn formatdaterange_a_list_is_not_a_range() {
    assert_eq!(
        format_date_range(&json!(["2024-10-21", "2024-10-25"])),
        None
    );
}

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:11 "formatDateRange > half a range is not a range"
#[test]
fn formatdaterange_half_a_range_is_not_a_range() {
    assert_eq!(
        format_date_range(&json!({ "start-date": "2024-10-21" })),
        None
    );
}

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:11 "formatDateRange > a non-string end is not a range"
#[test]
fn formatdaterange_a_non_string_end_is_not_a_range() {
    assert_eq!(
        format_date_range(&json!({ "start-date": "2024-10-21", "end-date": 5 })),
        None
    );
}

/// Port of: packages/utils/__tests__/form-answer-format.test.ts:11 "formatDateRange > null is not a range"
#[test]
fn formatdaterange_null_is_not_a_range() {
    assert_eq!(format_date_range(&json!(null)), None);
}
