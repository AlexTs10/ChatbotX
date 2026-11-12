//! Port of `packages/variables/__tests__/system-field-timezone.test.ts`.

mod support;

use chatbotx_core::variables::*;
use support::iso_ms;
use support::variables::*;

// 2026-07-22T20:00Z is 2026-07-23 03:00 in VN (UTC+7).
const NEAR_MIDNIGHT_UTC: &str = "2026-07-22T20:00:00.000Z";
// 2026-07-22T08:30Z is 15:30 in VN, 08:30 in UTC.
const MIDDAY_UTC: &str = "2026-07-22T08:30:00.000Z";

fn context(
    contact_tz: Option<&str>,
    workspace_tz: Option<&str>,
    inbox: ContactInboxModel,
) -> VariableContext {
    VariableContext {
        contact: ContactModel {
            timezone: contact_tz.map(str::to_owned),
            ..Default::default()
        },
        contact_inbox: Some(inbox),
        conversation: None,
        workspace: workspace_tz.map(|tz| WorkspaceModel {
            timezone: Some(tz.to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Port of: packages/variables/__tests__/system-field-timezone.test.ts:25 "subscribed_date renders contact-first > uses the contact timezone over the workspace timezone"
#[tokio::test]
async fn subscribed_date_renders_contact_first_uses_the_contact_timezone_over_the_workspace_timezone()
 {
    let h = Harness::new(FakeState::default());
    let ctx = context(
        Some("Asia/Ho_Chi_Minh"),
        Some("UTC"),
        ContactInboxModel {
            created_at: Some(iso_ms(NEAR_MIDNIGHT_UTC)),
            ..Default::default()
        },
    );
    assert_eq!(
        h.v()
            .get_system_field_value(&ctx, SystemFieldType::SubscribedDate)
            .await
            .unwrap(),
        s("2026-07-23")
    );
}

/// Port of: packages/variables/__tests__/system-field-timezone.test.ts:34 "subscribed_date renders contact-first > falls back to the workspace timezone when the contact has none"
#[tokio::test]
async fn subscribed_date_renders_contact_first_falls_back_to_the_workspace_timezone_when_the_contact_has_none()
 {
    let h = Harness::new(FakeState::default());
    let ctx = context(
        None,
        Some("Asia/Ho_Chi_Minh"),
        ContactInboxModel {
            created_at: Some(iso_ms(NEAR_MIDNIGHT_UTC)),
            ..Default::default()
        },
    );
    assert_eq!(
        h.v()
            .get_system_field_value(&ctx, SystemFieldType::SubscribedDate)
            .await
            .unwrap(),
        s("2026-07-23")
    );
}

/// Port of: packages/variables/__tests__/system-field-timezone.test.ts:45 "last_seen renders contact-first > uses the contact timezone over the workspace timezone"
#[tokio::test]
async fn last_seen_renders_contact_first_uses_the_contact_timezone_over_the_workspace_timezone() {
    let h = Harness::new(FakeState::default());
    let ctx = context(
        Some("Asia/Ho_Chi_Minh"),
        Some("UTC"),
        ContactInboxModel {
            contact_last_read_at: Some(iso_ms(MIDDAY_UTC)),
            ..Default::default()
        },
    );
    assert_eq!(
        h.v()
            .get_system_field_value(&ctx, SystemFieldType::LastSeen)
            .await
            .unwrap(),
        s("2026-07-22 15:30:00")
    );
}

/// Port of: packages/variables/__tests__/system-field-timezone.test.ts:54 "last_seen renders contact-first > normalizes an offset-style contact timezone (Messenger '+7')"
#[tokio::test]
async fn last_seen_renders_contact_first_normalizes_an_offset_style_contact_timezone_messenger_7() {
    let h = Harness::new(FakeState::default());
    let ctx = context(
        Some("+7"),
        Some("UTC"),
        ContactInboxModel {
            contact_last_read_at: Some(iso_ms(MIDDAY_UTC)),
            ..Default::default()
        },
    );
    assert_eq!(
        h.v()
            .get_system_field_value(&ctx, SystemFieldType::LastSeen)
            .await
            .unwrap(),
        s("2026-07-22 15:30:00")
    );
}
