//! Port of `packages/variables/__tests__/replace-custom-field-timezone.test.ts`.

mod support;

use chatbotx_core::variables::*;
use support::variables::*;

// 2026-07-22T08:30:00Z is 15:30 wall-clock at UTC+7 (VN / Bangkok).
const DATETIME_UTC: &str = "2026-07-22T08:30:00.000Z";

fn props(contact_tz: Option<&str>, workspace_tz: Option<&str>) -> VariableContext {
    VariableContext {
        contact: ContactModel {
            timezone: contact_tz.map(str::to_owned),
            ..Default::default()
        },
        contact_inbox: None,
        conversation: None,
        workspace: workspace_tz.map(|tz| WorkspaceModel {
            timezone: Some(tz.to_owned()),
            ..Default::default()
        }),
        custom_fields_map: custom_fields(&[("booking_at", "datetime", Some(DATETIME_UTC))]),
        ..Default::default()
    }
}

/// Port of: packages/variables/__tests__/replace-custom-field-timezone.test.ts:31 "replaceAll renders date/datetime custom fields contact-first > prefers the contact timezone over the workspace timezone"
#[tokio::test]
async fn replaceall_renders_date_datetime_custom_fields_contact_first_prefers_the_contact_timezone_over_the_workspace_timezone()
 {
    let h = Harness::new(FakeState::default());
    let text = h
        .v()
        .replace_all(
            "Booking: {{booking_at}}",
            &props(Some("Asia/Ho_Chi_Minh"), Some("UTC")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(text, "Booking: 2026-07-22 15:30:00");
}

/// Port of: packages/variables/__tests__/replace-custom-field-timezone.test.ts:40 "replaceAll renders date/datetime custom fields contact-first > falls back to the workspace timezone when the contact has none"
#[tokio::test]
async fn replaceall_renders_date_datetime_custom_fields_contact_first_falls_back_to_the_workspace_timezone_when_the_contact_has_none()
 {
    let h = Harness::new(FakeState::default());
    let text = h
        .v()
        .replace_all(
            "Booking: {{booking_at}}",
            &props(None, Some("Asia/Ho_Chi_Minh")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(text, "Booking: 2026-07-22 15:30:00");
}

/// Port of: packages/variables/__tests__/replace-custom-field-timezone.test.ts:49 "replaceAll renders date/datetime custom fields contact-first > normalizes an offset-style contact timezone (e.g. Messenger '+7')"
#[tokio::test]
async fn replaceall_renders_date_datetime_custom_fields_contact_first_normalizes_an_offset_style_contact_timezone_e_g_messenger_7()
 {
    let h = Harness::new(FakeState::default());
    let text = h
        .v()
        .replace_all(
            "Booking: {{booking_at}}",
            &props(Some("+7"), Some("UTC")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(text, "Booking: 2026-07-22 15:30:00");
}
