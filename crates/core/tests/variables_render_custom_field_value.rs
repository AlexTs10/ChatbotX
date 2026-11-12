//! Port of `packages/variables/__tests__/render-custom-field-value.test.ts`.

use chatbotx_core::variables::text::render_custom_field_value;

/// Port of: packages/variables/__tests__/render-custom-field-value.test.ts:5 "renderCustomFieldValue > renders date values as the literal calendar day"
#[test]
fn rendercustomfieldvalue_renders_date_values_as_the_literal_calendar_day() {
    assert_eq!(
        render_custom_field_value(
            "date",
            Some("2026-07-22T00:00:00+07:00"),
            Some("America/New_York")
        ),
        "2026-07-22"
    );
}

/// Port of: packages/variables/__tests__/render-custom-field-value.test.ts:15 "renderCustomFieldValue > formats temporal values in the requested timezone"
#[test]
fn rendercustomfieldvalue_formats_temporal_values_in_the_requested_timezone() {
    assert_eq!(
        render_custom_field_value(
            "datetime",
            Some("2026-07-22T08:30:00.000Z"),
            Some("Asia/Ho_Chi_Minh")
        ),
        "2026-07-22 15:30:00"
    );
}

/// Port of: packages/variables/__tests__/render-custom-field-value.test.ts:25 "renderCustomFieldValue > leaves non-temporal values unchanged"
#[test]
fn rendercustomfieldvalue_leaves_non_temporal_values_unchanged() {
    assert_eq!(
        render_custom_field_value("shortText", Some("hello"), Some("Asia/Ho_Chi_Minh")),
        "hello"
    );
}
