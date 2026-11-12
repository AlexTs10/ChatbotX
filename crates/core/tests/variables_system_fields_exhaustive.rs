//! Port of `packages/variables/__tests__/system-fields-exhaustive.test.ts`. The original stubs every helper
//! module; here [`Fake`] is fully populated instead, and the fields read
//! from the inbox's integration use an inbox of their own channel (the real
//! `getIntegrationField` is null for another channel).

mod support;

use chatbotx_core::variables::*;
use support::variables::*;
use support::{RecordingLog, iso_ms, now_ms};
use tracing::Level;

fn contact() -> ContactModel {
    ContactModel {
        id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        total_tagged: Some(4),
        total_new_tagged: Some(1),
        first_name: s("Ada"),
        last_name: s("Lovelace"),
        email: s("ada@example.com"),
        phone_number: s("+84900000000"),
        avatar: s("avatars/a.png"),
        gender: s("male"),
        locale: s("vi_VN"),
        timezone: s("7"),
        country: s("VN"),
        state: s("HCM"),
        city: s("Thu Duc"),
        ref_: s("launch"),
        location: Some(ContactLocation {
            latitude: Some(10.75),
            longitude: Some(106.66),
        }),
    }
}

fn contact_inbox(channel: &str) -> ContactInboxModel {
    let at = iso_ms("2026-01-02T03:04:05.000Z");
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        inbox_id: "inbox-1".into(),
        source_id: "source-1".into(),
        channel: channel.into(),
        source: s("ads"),
        created_at: Some(at),
        contact_last_read_at: Some(at),
        last_incoming_message_at: Some(at),
        last_outbound_message_at: Some(at),
        last_btn_title: s("Choose plan"),
        last_story_id: s("story-1"),
        last_input_failure: s("Invalid email"),
        last_error_log: s("(#551) unavailable"),
        webchat_parent_url: s("https://shop.example"),
        consecutive_failed_reply: Some(2),
        last_comment_message_id: s("message-1"),
        last_comment_message_at: Some(iso_ms("2026-01-03T03:04:05.000Z")),
        referral: Some(Referral {
            ref_: s("launch"),
            ad_id: s("ad-123"),
            ad_title: s("Launch ad"),
            ad_body: s("Launch offer"),
            ctwa_clid: s("ctwa-1"),
            source_url: s("https://example.com/ad"),
            source_platform: s("facebook"),
        }),
        source_username: s("handle"),
        source_user_id: s("user.1"),
        ..Default::default()
    }
}

fn workspace() -> WorkspaceModel {
    WorkspaceModel {
        id: "workspace-1".into(),
        name: s("Workspace One"),
        logo: s("logos/w.png"),
        timezone: s("UTC"),
        language: s("vi"),
        scheduled_deletion_at: None,
    }
}

/// The channel whose inbox backs `key` (the integration fields are per channel).
fn channel_for(key: SystemFieldType) -> &'static str {
    use SystemFieldType as F;
    match key {
        F::IgUserName
        | F::IgFollowers
        | F::IgVerified
        | F::IgFollowBusiness
        | F::IgBusinessFollowUser => "instagram",
        F::Webchat => "webchat",
        F::WaUserId | F::WaUserName => "whatsapp",
        _ => "messenger",
    }
}

fn full_context(key: SystemFieldType) -> VariableContext {
    VariableContext {
        contact: contact(),
        contact_inbox: Some(contact_inbox(channel_for(key))),
        conversation: Some(ConversationModel {
            id: "conversation-1".into(),
            contact_id: "contact-1".into(),
            workspace_id: "workspace-1".into(),
            ..Default::default()
        }),
        appointment_id: s("appointment-1"),
        sent_by_user_id: s("user-1"),
        workspace: Some(workspace()),
        ..Default::default()
    }
}

fn empty_context() -> VariableContext {
    VariableContext {
        contact: contact(),
        ..Default::default()
    }
}

fn row(id: &str) -> Option<IntegrationRow> {
    Some(IntegrationRow {
        id: id.into(),
        name: s("Page"),
        page_id: s("page-1"),
        auth: serde_json::json!({}),
    })
}

fn full_harness() -> Harness {
    let appointment = Appointment {
        id: "appointment-1".into(),
        workspace_id: "workspace-1".into(),
        contact_id: "contact-1".into(),
        conversation_id: s("conversation-1"),
        start_at: Some(iso_ms("2026-01-04T03:04:05.000Z")),
        invitee_timezone: s("UTC"),
        calendar_name: s("Discovery"),
    };
    Harness::new(FakeState {
        contact: Some(contact()),
        conversation_by_contact: Some(ConversationModel {
            assigned_user_id: s("admin-1"),
            assigned_inbox_team_id: s("team-1"),
            ..Default::default()
        }),
        dm_conversation: Some(ConversationModel {
            last_step: s("step-1"),
            current_step: s("step-2"),
            ..Default::default()
        }),
        latest_conversation: Some(ConversationModel {
            id: "conversation-1".into(),
            workspace_id: "workspace-1".into(),
            last_activity_at: Some(now_ms()),
            ..Default::default()
        }),
        member: Some(MemberUser {
            id: "admin-1".into(),
            name: s("Admin"),
            email: s("admin@example.com"),
        }),
        team_name: s("Team"),
        team_names: vec!["Team A".into(), "Team B".into()],
        tags: vec!["vip".into()],
        ad_ids: vec!["ad-1".into(), "ad-2".into()],
        notes: vec!["note".into()],
        message: Some(MessageModel {
            id: "message-1".into(),
            source_id: s("comment-1"),
            text: s("Nice post"),
            content_attributes: Some(serde_json::json!({ "postId": "post-1" })),
            ..Default::default()
        }),
        incoming_texts: vec!["queued messages".into()],
        last_messages: vec![ChatMessage {
            text: s("history"),
            sender_type: "user".into(),
        }],
        latest_input: Some(MessageWithAttachments {
            content_type: "text".into(),
            text: s("hello"),
            ..Default::default()
        }),
        recorded_call: Some(CallRecord {
            id: "call-1".into(),
            recording_path: s("calls/1.ogg"),
            transcript: None,
        }),
        transcribed_call: Some(CallRecord {
            transcript: s("call transcript"),
            ..Default::default()
        }),
        signed_url: Ok("https://cdn.example/recording.ogg".into()),
        tenant: TenantSettings {
            app_url: "https://app.example.test".into(),
            storage_url: "https://cdn.example/".into(),
        },
        app_url: "https://app.example.test".into(),
        inbox: Ok(Some(InboxWithIntegrations {
            id: "inbox-1".into(),
            workspace_id: "workspace-1".into(),
            integration_messenger: row("messenger-1"),
            integration_instagram: row("instagram-1"),
            integration_whatsapp: row("whatsapp-1"),
            integration_webchat: row("webchat-1"),
            ..Default::default()
        })),
        integration_ctx: Ok(serde_json::json!({})),
        snapshot: Ok(ContactProfileSnapshot {
            follows_business: Some(true),
            business_follows_contact: Some(true),
            account_verified: Some(true),
            follower_count: Some(10),
            username: s("handle"),
        }),
        user_inbox_link: Ok(s("https://business.facebook.com/inbox/1")),
        post_message: Ok(s("post text")),
        me_row_id: "system-field-1".into(),
        appointment: Some(appointment.clone()),
        latest_appointment: Some(appointment),
        points: Some(5.0),
        lead: Some(LeadAdsLead {
            leadgen_id: s("lead-1"),
            form_id: s("form-1"),
            form_name: s("Form"),
            adgroup_id: s("adgroup-1"),
        }),
        api_token: Ok("cbx_ws_plaintext".into()),
        referrer: s("Ada Referrer"),
        last_item_id: s("pipeline-item-1"),
        ..Default::default()
    })
}

/// Port of: packages/variables/__tests__/system-fields-exhaustive.test.ts:263 "getSystemFieldValue exhaustiveness > every system field resolves to a string or null"
#[tokio::test]
async fn getsystemfieldvalue_exhaustiveness_every_system_field_resolves_to_a_string_or_null() {
    let h = full_harness();
    for key in SystemFieldType::ALL {
        let value: Option<String> = h
            .v()
            .get_system_field_value(&full_context(*key), *key)
            .await
            .unwrap();
        let _ = value;
    }
}

/// Port of: packages/variables/__tests__/system-fields-exhaustive.test.ts:273 "getSystemFieldValue exhaustiveness > no system field falls through to the unhandled branch"
#[tokio::test]
async fn getsystemfieldvalue_exhaustiveness_no_system_field_falls_through_to_the_unhandled_branch()
{
    let log = RecordingLog::install();
    let h = full_harness();
    for key in SystemFieldType::ALL {
        h.v()
            .get_system_field_value(&full_context(*key), *key)
            .await
            .unwrap();
    }
    assert!(
        log.at(Level::ERROR).is_empty(),
        "{:?}",
        log.at(Level::ERROR)
    );
}

/// Port of: packages/variables/__tests__/system-fields-exhaustive.test.ts:281 "getSystemFieldValue exhaustiveness > every system field survives an empty context without throwing"
#[tokio::test]
async fn getsystemfieldvalue_exhaustiveness_every_system_field_survives_an_empty_context_without_throwing()
 {
    let log = RecordingLog::install();
    let h = full_harness();
    for key in SystemFieldType::ALL {
        let result = h.v().get_system_field_value(&empty_context(), *key).await;
        assert!(
            result.is_ok(),
            "{key} threw on an empty context: {result:?}"
        );
    }
    assert!(
        log.at(Level::ERROR).is_empty(),
        "{:?}",
        log.at(Level::ERROR)
    );
}

/// Port of: packages/variables/__tests__/system-fields-exhaustive.test.ts:292 "getSystemFieldValue exhaustiveness > only the known-unimplemented fields are null for a fully populated contact"
#[tokio::test]
async fn getsystemfieldvalue_exhaustiveness_only_the_known_unimplemented_fields_are_null_for_a_fully_populated_contact()
 {
    let h = full_harness();
    let mut null_fields = Vec::new();
    for key in SystemFieldType::ALL {
        if h.v()
            .get_system_field_value(&full_context(*key), *key)
            .await
            .unwrap()
            .is_none()
        {
            null_fields.push(key.as_str());
        }
    }
    assert_eq!(null_fields, vec!["last_order"]);
}
