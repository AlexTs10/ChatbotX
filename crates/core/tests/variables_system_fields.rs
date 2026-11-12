//! Port of `packages/variables/__tests__/system-fields.test.ts`. The business / integration mocks are
//! [`Fake`] answers (the first describe's `beforeEach` is [`base`]); the
//! mocked `withCache` is the real in-process cache; `signMeLink` and
//! `resolveGenderLabel` run for real (their results are checked instead of
//! their calls).

mod support;

use chatbotx_core::encryption::MeLinkParams;
use chatbotx_core::variables::*;
use ring::hmac;
use support::variables::*;
use support::{TEST_ENCRYPTION_KEY, iso_ms};

use SystemFieldType as F;

fn contact() -> ContactModel {
    ContactModel {
        id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn contact_inbox() -> ContactInboxModel {
    let at = iso_ms("2026-01-02T03:04:05.000Z");
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        created_at: Some(at),
        contact_last_read_at: Some(at),
        last_incoming_message_at: Some(at),
        channel: "messenger".into(),
        inbox_id: "inbox-1".into(),
        source: s("ads"),
        source_id: "source-1".into(),
        ..Default::default()
    }
}

fn workspace() -> WorkspaceModel {
    WorkspaceModel {
        id: "workspace-1".into(),
        name: s("Workspace One"),
        logo: None,
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn conversation() -> ConversationModel {
    ConversationModel {
        id: "conversation-1".into(),
        contact_id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        ..Default::default()
    }
}

/// `createContext()`: the default contact, inbox, conversation, workspace.
fn ctx() -> VariableContext {
    VariableContext {
        contact: contact(),
        contact_inbox: Some(contact_inbox()),
        conversation: Some(conversation()),
        workspace: Some(workspace()),
        ..Default::default()
    }
}

fn with_contact(contact: ContactModel) -> VariableContext {
    VariableContext { contact, ..ctx() }
}

fn with_inbox(contact_inbox: Option<ContactInboxModel>) -> VariableContext {
    VariableContext {
        contact_inbox,
        ..ctx()
    }
}

/// The hoisted defaults plus the first describe's `beforeEach`.
fn base_state() -> FakeState {
    FakeState {
        conversation_by_contact: Some(ConversationModel {
            assigned_user_id: s("user-1"),
            ..Default::default()
        }),
        member: Some(MemberUser {
            id: "user-1".into(),
            name: s("Admin"),
            email: s("admin@example.com"),
        }),
        integration_ctx: Ok(serde_json::json!({ "workspaceId": "workspace-1" })),
        inbox: Ok(Some(InboxWithIntegrations {
            id: "inbox-1".into(),
            workspace_id: "workspace-1".into(),
            integration_instagram: Some(IntegrationRow {
                id: "instagram-integration-1".into(),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn base() -> Harness {
    Harness::new(base_state())
}

async fn get(h: &Harness, context: &VariableContext, key: SystemFieldType) -> Option<String> {
    h.v().get_system_field_value(context, key).await.unwrap()
}

fn instagram_inbox() -> ContactInboxModel {
    ContactInboxModel {
        channel: "instagram".into(),
        source_id: "igsid-1".into(),
        source_username: None,
        follows_business: None,
        business_follows_contact: None,
        account_verified: None,
        follower_count: None,
        ..contact_inbox()
    }
}

fn live_snapshot() -> ContactProfileSnapshot {
    ContactProfileSnapshot {
        username: s("contact_username"),
        follower_count: Some(123),
        follows_business: Some(true),
        business_follows_contact: Some(false),
        account_verified: Some(true),
    }
}

fn integration(id: &str) -> Option<IntegrationRow> {
    Some(IntegrationRow {
        id: id.into(),
        ..Default::default()
    })
}

fn messenger_auth() -> serde_json::Value {
    serde_json::json!({ "tokens": { "accessToken": "token" }, "version": "v23.0" })
}

fn inbox(
    f: impl FnOnce(&mut InboxWithIntegrations),
) -> Result<Option<InboxWithIntegrations>, String> {
    let mut inbox = InboxWithIntegrations::default();
    f(&mut inbox);
    Ok(Some(inbox))
}

fn storage_tenant() -> TenantSettings {
    TenantSettings {
        storage_url: "http://localhost:3123/storage/".into(),
        ..Default::default()
    }
}

fn profile_contact() -> ContactModel {
    ContactModel {
        first_name: s("Ada"),
        last_name: s("Lovelace"),
        email: s("ada@example.com"),
        phone_number: s("+84900000000"),
        country: s("VN"),
        state: s("Ho Chi Minh"),
        city: s("Thu Duc"),
        gender: s("female"),
        timezone: s("7"),
        ..contact()
    }
}

fn appointment(id: &str, calendar: &str, start: &str) -> Appointment {
    Appointment {
        id: id.into(),
        workspace_id: "workspace-1".into(),
        contact_id: "contact-1".into(),
        conversation_id: None,
        start_at: Some(iso_ms(start)),
        invitee_timezone: s("Asia/Ho_Chi_Minh"),
        calendar_name: s(calendar),
    }
}

fn latest_lead() -> LeadAdsLead {
    LeadAdsLead {
        leadgen_id: s("lead-9"),
        form_id: s("form-3"),
        form_name: s("Summer promo"),
        adgroup_id: s("adgroup-5"),
    }
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:288 "getSystemFieldValue > user_hash uses ENCRYPTION_KEY with the contact inbox source id and id"
#[tokio::test]
async fn getsystemfieldvalue_user_hash_uses_encryption_key_with_the_contact_inbox_source_id_and_id()
{
    let h = base();
    let key_bytes: Vec<u8> = (0..TEST_ENCRYPTION_KEY.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&TEST_ENCRYPTION_KEY[i..i + 2], 16).unwrap())
        .collect();
    let tag = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, &key_bytes),
        b"source-1:contact-inbox-1",
    );
    let expected: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(get(&h, &ctx(), F::UserHash).await, Some(expected));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:298 "getSystemFieldValue > user_hash returns null without a contact inbox context"
#[tokio::test]
async fn getsystemfieldvalue_user_hash_returns_null_without_a_contact_inbox_context() {
    let h = base();
    assert_eq!(get(&h, &with_inbox(None), F::UserHash).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:307 "getSystemFieldValue > user_channel capitalizes the first letter"
#[tokio::test]
async fn getsystemfieldvalue_user_channel_capitalizes_the_first_letter() {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::UserChannel).await, s("Messenger"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:313 "getSystemFieldValue > user_channel capitalizes the fallback primary channel"
#[tokio::test]
async fn getsystemfieldvalue_user_channel_capitalizes_the_fallback_primary_channel() {
    let h = base();
    h.fake.set(|st| {
        st.recent_contact_inbox = Some(ContactInboxModel {
            channel: "webchat".into(),
            ..contact_inbox()
        })
    });
    assert_eq!(
        get(&h, &with_inbox(None), F::UserChannel).await,
        s("Webchat")
    );
    assert!(
        h.fake
            .called("find_recent_contact_inbox(workspace-1, contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:354 "getSystemFieldValue > ig_user_name prefers the stored handle and makes no provider call"
#[tokio::test]
async fn getsystemfieldvalue_ig_user_name_prefers_the_stored_handle_and_makes_no_provider_call() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        source_username: s("stored_handle"),
        ..instagram_inbox()
    }));
    assert_eq!(get(&h, &context, F::IgUserName).await, s("stored_handle"));
    assert_eq!(h.fake.count("resolve_integration_context"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:368 "getSystemFieldValue > ig_user_name falls back to the live handle when none is stored"
#[tokio::test]
async fn getsystemfieldvalue_ig_user_name_falls_back_to_the_live_handle_when_none_is_stored() {
    let h = base();
    h.fake.set(|st| st.snapshot = Ok(live_snapshot()));
    assert_eq!(
        get(&h, &with_inbox(Some(instagram_inbox())), F::IgUserName).await,
        s("contact_username")
    );
    assert!(
        h.fake
            .called("resolve_integration_context(workspace-1, instagram, igsid-1)")
    );
    assert!(
        h.fake
            .called("run_get_profile_snapshot({\"workspaceId\":\"workspace-1\"}, igsid-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:394 "getSystemFieldValue > profile fields each map to their own attribute with one provider call"
#[tokio::test]
async fn getsystemfieldvalue_profile_fields_each_map_to_their_own_attribute_with_one_provider_call()
{
    let h = base();
    h.fake.set(|st| st.snapshot = Ok(live_snapshot()));
    let context = with_inbox(Some(instagram_inbox()));
    assert_eq!(get(&h, &context, F::IgFollowers).await, s("123"));
    assert_eq!(get(&h, &context, F::IgVerified).await, s("true"));
    assert_eq!(get(&h, &context, F::IgFollowBusiness).await, s("true"));
    assert_eq!(get(&h, &context, F::IgBusinessFollowUser).await, s("false"));
    assert_eq!(h.fake.count("run_get_profile_snapshot"), 1);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:417 "getSystemFieldValue > renders genuine false and 0 values instead of dropping them"
#[tokio::test]
async fn getsystemfieldvalue_renders_genuine_false_and_0_values_instead_of_dropping_them() {
    let h = base();
    h.fake.set(|st| {
        st.snapshot = Ok(ContactProfileSnapshot {
            username: None,
            follower_count: Some(0),
            follows_business: Some(false),
            business_follows_contact: Some(false),
            account_verified: Some(false),
        })
    });
    let context = with_inbox(Some(instagram_inbox()));
    assert_eq!(get(&h, &context, F::IgFollowers).await, s("0"));
    assert_eq!(get(&h, &context, F::IgVerified).await, s("false"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:435 "getSystemFieldValue > writes a successful fetch through to the contact-inbox columns"
#[tokio::test]
async fn getsystemfieldvalue_writes_a_successful_fetch_through_to_the_contact_inbox_columns() {
    let h = base();
    h.fake.set(|st| st.snapshot = Ok(live_snapshot()));
    get(&h, &with_inbox(Some(instagram_inbox())), F::IgFollowers).await;
    assert!(h.fake.called(&format!(
        "refresh_profile_snapshot(contact-inbox-1, inbox-1, {})",
        serde_json::to_string(&live_snapshot()).unwrap()
    )));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:450 "getSystemFieldValue > a failed persist does not fail the rendered value"
#[tokio::test]
async fn getsystemfieldvalue_a_failed_persist_does_not_fail_the_rendered_value() {
    let h = base();
    h.fake.set(|st| {
        st.snapshot = Ok(live_snapshot());
        st.refresh_snapshot = Err("db down".into());
    });
    assert_eq!(
        get(&h, &with_inbox(Some(instagram_inbox())), F::IgFollowers).await,
        s("123")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:462 "getSystemFieldValue > profile fields return null when the provider returns nothing and nothing is stored"
#[tokio::test]
async fn getsystemfieldvalue_profile_fields_return_null_when_the_provider_returns_nothing_and_nothing_is_stored()
 {
    let h = base();
    let context = with_inbox(Some(instagram_inbox()));
    for key in [
        F::IgUserName,
        F::IgFollowers,
        F::IgVerified,
        F::IgFollowBusiness,
        F::IgBusinessFollowUser,
    ] {
        assert_eq!(get(&h, &context, key).await, None, "{key}");
    }
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:483 "getSystemFieldValue > falls back to the stored columns when the provider call fails, without persisting"
#[tokio::test]
async fn getsystemfieldvalue_falls_back_to_the_stored_columns_when_the_provider_call_fails_without_persisting()
 {
    let h = base();
    h.fake.set(|st| st.snapshot = Err("(#230) consent".into()));
    let context = with_inbox(Some(ContactInboxModel {
        follower_count: Some(0),
        account_verified: Some(false),
        follows_business: Some(true),
        business_follows_contact: Some(false),
        ..instagram_inbox()
    }));
    assert_eq!(get(&h, &context, F::IgFollowers).await, s("0"));
    assert_eq!(get(&h, &context, F::IgVerified).await, s("false"));
    assert_eq!(get(&h, &context, F::IgFollowBusiness).await, s("true"));
    assert_eq!(h.fake.count("refresh_profile_snapshot"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:506 "getSystemFieldValue > falls back to the stored columns when the integration cannot be resolved"
#[tokio::test]
async fn getsystemfieldvalue_falls_back_to_the_stored_columns_when_the_integration_cannot_be_resolved()
 {
    let h = base();
    h.fake
        .set(|st| st.integration_ctx = Err("integration_auth_missing".into()));
    let context = with_inbox(Some(ContactInboxModel {
        follower_count: Some(42),
        ..instagram_inbox()
    }));
    assert_eq!(get(&h, &context, F::IgFollowers).await, s("42"));
    assert_eq!(h.fake.count("run_get_profile_snapshot"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:522 "getSystemFieldValue > returns null on a failed fetch when nothing is stored"
#[tokio::test]
async fn getsystemfieldvalue_returns_null_on_a_failed_fetch_when_nothing_is_stored() {
    let h = base();
    h.fake.set(|st| st.snapshot = Err("boom".into()));
    assert_eq!(
        get(&h, &with_inbox(Some(instagram_inbox())), F::IgFollowers).await,
        None
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:533 "getSystemFieldValue > returns null for a channel without profile snapshot support"
#[tokio::test]
async fn getsystemfieldvalue_returns_null_for_a_channel_without_profile_snapshot_support() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        channel: "messenger".into(),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::IgVerified).await, None);
    assert_eq!(h.fake.count("resolve_integration_context"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:551 "getSystemFieldValue > wa_user_id / wa_user_name resolve from the stored ContactInbox columns"
#[tokio::test]
async fn getsystemfieldvalue_wa_user_id_wa_user_name_resolve_from_the_stored_contactinbox_columns()
{
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| i.integration_whatsapp = integration("whatsapp-integration-1"))
    });
    let context = with_inbox(Some(ContactInboxModel {
        channel: "whatsapp".into(),
        source_id: "user.9373001".into(),
        source_user_id: s("user.9373001"),
        source_username: s("@handle"),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::WaUserId).await, s("user.9373001"));
    assert_eq!(get(&h, &context, F::WaUserName).await, s("@handle"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:577 "getSystemFieldValue > wa_user_id / wa_user_name resolve to null when absent (phone-keyed contact, no adopted username)"
#[tokio::test]
async fn getsystemfieldvalue_wa_user_id_wa_user_name_resolve_to_null_when_absent_phone_keyed_contact_no_adopted_username()
 {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| i.integration_whatsapp = integration("whatsapp-integration-1"))
    });
    let context = with_inbox(Some(ContactInboxModel {
        channel: "whatsapp".into(),
        source_id: "84901234567".into(),
        source_user_id: None,
        source_username: None,
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::WaUserId).await, None);
    assert_eq!(get(&h, &context, F::WaUserName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:603 "getSystemFieldValue > wa_user_id / wa_user_name resolve to null on a non-WhatsApp inbox (channel guard)"
#[tokio::test]
async fn getsystemfieldvalue_wa_user_id_wa_user_name_resolve_to_null_on_a_non_whatsapp_inbox_channel_guard()
 {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| i.integration_instagram = integration("instagram-integration-1"))
    });
    assert_eq!(get(&h, &ctx(), F::WaUserId).await, None);
    assert_eq!(get(&h, &ctx(), F::WaUserName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:616 "getSystemFieldValue > fb_chat_link resolves the Business Suite conversation link"
#[tokio::test]
async fn getsystemfieldvalue_fb_chat_link_resolves_the_business_suite_conversation_link() {
    let h = base();
    h.fake.set(|st| {
        st.user_inbox_link = Ok(s(
            "https://business.facebook.com/1453585961452628/inbox/1453594044785153",
        ));
        st.inbox = inbox(|i| {
            i.integration_messenger = Some(IntegrationRow {
                id: "messenger-integration-1".into(),
                page_id: s("12345"),
                auth: messenger_auth(),
                ..Default::default()
            })
        });
    });
    assert_eq!(
        get(&h, &ctx(), F::FbChatLink).await,
        s("https://business.facebook.com/1453585961452628/inbox/1453594044785153")
    );
    assert!(h.fake.called(&format!(
        "messenger_user_inbox_link({}, source-1)",
        messenger_auth()
    )));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:641 "getSystemFieldValue > fb_chat_link returns null when the inbox has no messenger integration"
#[tokio::test]
async fn getsystemfieldvalue_fb_chat_link_returns_null_when_the_inbox_has_no_messenger_integration()
{
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    assert_eq!(get(&h, &ctx(), F::FbChatLink).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:649 "getSystemFieldValue > timezone_name maps a stored utc offset to an IANA zone"
#[tokio::test]
async fn getsystemfieldvalue_timezone_name_maps_a_stored_utc_offset_to_an_iana_zone() {
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    let zone = |tz: &str| {
        with_contact(ContactModel {
            timezone: s(tz),
            ..contact()
        })
    };
    assert_eq!(
        get(&h, &zone("7"), F::TimezoneName).await,
        s("Asia/Bangkok")
    );
    assert_eq!(
        get(&h, &zone("-5"), F::TimezoneName).await,
        s("America/New_York")
    );
    assert_eq!(
        get(&h, &zone("+07:00"), F::TimezoneName).await,
        s("Asia/Bangkok")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:678 "getSystemFieldValue > timezone_name falls back to the raw value for an unmapped offset"
#[tokio::test]
async fn getsystemfieldvalue_timezone_name_falls_back_to_the_raw_value_for_an_unmapped_offset() {
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    let context = with_contact(ContactModel {
        timezone: s("Asia/Ho_Chi_Minh"),
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::TimezoneName).await,
        s("Asia/Ho_Chi_Minh")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:691 "getSystemFieldValue > timezone_name returns null when the contact has no timezone"
#[tokio::test]
async fn getsystemfieldvalue_timezone_name_returns_null_when_the_contact_has_no_timezone() {
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    let context = with_contact(ContactModel {
        timezone: None,
        ..contact()
    });
    assert_eq!(get(&h, &context, F::TimezoneName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:704 "getSystemFieldValue > inbox_link opens the active conversation when available"
#[tokio::test]
async fn getsystemfieldvalue_inbox_link_opens_the_active_conversation_when_available() {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|_| {});
        st.tenant = TenantSettings {
            app_url: "https://builder.example".into(),
            ..Default::default()
        };
    });
    assert_eq!(
        get(&h, &ctx(), F::InboxLink).await,
        s("https://builder.example/space/workspace-1/inbox?conversationId=conversation-1")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:717 "getSystemFieldValue > inbox_link falls back to the inbox root without conversation context"
#[tokio::test]
async fn getsystemfieldvalue_inbox_link_falls_back_to_the_inbox_root_without_conversation_context()
{
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|_| {});
        st.tenant = TenantSettings {
            app_url: "https://builder.example/".into(),
            ..Default::default()
        };
    });
    let context = VariableContext {
        conversation: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::InboxLink).await,
        s("https://builder.example/space/workspace-1/inbox")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:731 "getSystemFieldValue > contact inbox tracking fields resolve from context"
#[tokio::test]
async fn getsystemfieldvalue_contact_inbox_tracking_fields_resolve_from_context() {
    let h = base();
    let tracking = Some(ContactInboxModel {
        last_btn_title: s("Choose plan"),
        last_story_id: s("story-42"),
        consecutive_failed_reply: Some(2),
        last_outbound_message_at: Some(iso_ms("2026-01-04T03:04:05.000Z")),
        last_input_failure: s("Invalid email"),
        referral: Some(Referral {
            ref_: s("launch"),
            ad_id: s("ad-123"),
            ad_title: s("Launch ad"),
            ad_body: s("Launch offer"),
            ctwa_clid: s("ctwa-1"),
            source_url: s("https://example.com/ad"),
            source_platform: s("facebook"),
        }),
        ..contact_inbox()
    });
    let context = with_inbox(tracking);
    assert_eq!(get(&h, &context, F::LastBtnTitle).await, s("Choose plan"));
    assert_eq!(get(&h, &context, F::LastStoryId).await, s("story-42"));
    assert_eq!(get(&h, &with_inbox(None), F::LastStoryId).await, None);
    // Legacy spelling of last_btn_title, kept so imported flows resolve.
    assert_eq!(get(&h, &context, F::LastBttTitle).await, s("Choose plan"));
    assert_eq!(get(&h, &context, F::ConsecutiveFailedReply).await, s("2"));
    assert_eq!(
        get(&h, &context, F::LastOutboundMessageAt).await,
        s("2026-01-04 03:04:05")
    );
    assert_eq!(get(&h, &context, F::LastAd).await, s("ad-123"));
    assert_eq!(get(&h, &context, F::LastAdHeadline).await, s("Launch ad"));
    assert_eq!(get(&h, &context, F::LastAdBody).await, s("Launch offer"));
    assert_eq!(get(&h, &context, F::LastCtwa).await, s("ctwa-1"));
    assert_eq!(
        get(&h, &context, F::LastAdSourceUrl).await,
        s("https://example.com/ad")
    );
    assert_eq!(
        get(&h, &context, F::LastAdSourcePlatform).await,
        s("facebook")
    );
    assert_eq!(
        get(&h, &context, F::LastInputFailure).await,
        s("Invalid email")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:831 "getSystemFieldValue > referral fields return null when the key is absent or blank"
#[tokio::test]
async fn getsystemfieldvalue_referral_fields_return_null_when_the_key_is_absent_or_blank() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        referral: Some(Referral {
            ad_title: s(""),
            source_url: s("https://example.com/ad"),
            ..Default::default()
        }),
        ..contact_inbox()
    }));
    for key in [
        F::LastAd,
        F::LastAdHeadline,
        F::LastAdBody,
        F::LastCtwa,
        F::LastAdSourcePlatform,
    ] {
        assert_eq!(get(&h, &context, key).await, None, "{key}");
    }
    assert_eq!(
        get(&h, &context, F::LastAdSourceUrl).await,
        s("https://example.com/ad")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:878 "getSystemFieldValue > tag counters > resolve the contact's accumulated totals"
#[tokio::test]
async fn getsystemfieldvalue_tag_counters_resolve_the_contact_s_accumulated_totals() {
    let h = base();
    let context = with_contact(ContactModel {
        total_tagged: Some(7),
        total_new_tagged: Some(3),
        ..contact()
    });
    assert_eq!(get(&h, &context, F::TotalTagged).await, s("7"));
    assert_eq!(get(&h, &context, F::TotalNewTagged).await, s("3"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:897 "getSystemFieldValue > tag counters > keep a zero count as '0'"
#[tokio::test]
async fn getsystemfieldvalue_tag_counters_keep_a_zero_count_as_0() {
    let h = base();
    let context = with_contact(ContactModel {
        total_tagged: Some(0),
        total_new_tagged: Some(0),
        ..contact()
    });
    assert_eq!(get(&h, &context, F::TotalTagged).await, s("0"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:909 "getSystemFieldValue > tag counters > stay null when the contact carries no counters"
#[tokio::test]
async fn getsystemfieldvalue_tag_counters_stay_null_when_the_contact_carries_no_counters() {
    let h = base();
    let context = with_contact(ContactModel {
        total_tagged: None,
        total_new_tagged: None,
        ..contact()
    });
    assert_eq!(get(&h, &context, F::TotalTagged).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:923 "getSystemFieldValue > user_id resolves to the contact id even without a contact inbox"
#[tokio::test]
async fn getsystemfieldvalue_user_id_resolves_to_the_contact_id_even_without_a_contact_inbox() {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::UserId).await, s("contact-1"));
    assert_eq!(get(&h, &with_inbox(None), F::UserId).await, s("contact-1"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:935 "getSystemFieldValue > contact inbox passthrough fields resolve from context"
#[tokio::test]
async fn getsystemfieldvalue_contact_inbox_passthrough_fields_resolve_from_context() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        webchat_parent_url: s("https://shop.example/checkout"),
        last_error_log: s("(#551) This person isn't available right now."),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::UserExternalId).await, s("source-1"));
    assert_eq!(
        get(&h, &context, F::WebchatParentUrl).await,
        s("https://shop.example/checkout")
    );
    assert_eq!(
        get(&h, &context, F::LastErrorLog).await,
        s("(#551) This person isn't available right now.")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:962 "getSystemFieldValue > contact inbox passthrough fields return null without a contact inbox"
#[tokio::test]
async fn getsystemfieldvalue_contact_inbox_passthrough_fields_return_null_without_a_contact_inbox()
{
    let h = base();
    for key in [F::UserExternalId, F::WebchatParentUrl, F::LastErrorLog] {
        assert_eq!(get(&h, &with_inbox(None), key).await, None, "{key}");
    }
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:974 "getSystemFieldValue > timezone renders signed offsets from legacy and IANA storage"
#[tokio::test]
async fn getsystemfieldvalue_timezone_renders_signed_offsets_from_legacy_and_iana_storage() {
    let h = base();
    let zone = |tz: Option<&str>| {
        with_contact(ContactModel {
            timezone: tz.map(str::to_owned),
            ..contact()
        })
    };
    assert_eq!(get(&h, &zone(Some("7")), F::Timezone).await, s("+7"));
    assert_eq!(get(&h, &zone(Some("-5")), F::Timezone).await, s("-5"));
    assert_eq!(
        get(&h, &zone(Some("Asia/Ho_Chi_Minh")), F::Timezone).await,
        s("+7")
    );
    assert_eq!(get(&h, &zone(None), F::Timezone).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1012 "getSystemFieldValue > language reads ContactInbox.language before falling back to contact locale"
#[tokio::test]
async fn getsystemfieldvalue_language_reads_contactinbox_language_before_falling_back_to_contact_locale()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            locale: s("vi_VN"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            language: s("en"),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::Language).await, s("en"));
    let context = VariableContext {
        contact: ContactModel {
            locale: s("vi-VN"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            language: None,
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::Language).await, s("vi"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1040 "getSystemFieldValue > user_source maps the stored source to a human label"
#[tokio::test]
async fn getsystemfieldvalue_user_source_maps_the_stored_source_to_a_human_label() {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::UserSource).await, s("Ads"));
    let context = with_inbox(Some(ContactInboxModel {
        source: s("comments"),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::UserSource).await,
        s("Facebook/IG Comment")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1058 "getSystemFieldValue > user_source labels a livestream comment source"
#[tokio::test]
async fn getsystemfieldvalue_user_source_labels_a_livestream_comment_source() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        channel: "messenger".into(),
        source: s("liveComments"),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::UserSource).await,
        s("Facebook/IG Livestream")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1073 "getSystemFieldValue > user_source names the tiktok comment channel"
#[tokio::test]
async fn getsystemfieldvalue_user_source_names_the_tiktok_comment_channel() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        channel: "tiktok".into(),
        source: s("comments"),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::UserSource).await, s("TikTok Comment"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1073 "getSystemFieldValue > user_source names the threads comment channel"
#[tokio::test]
async fn getsystemfieldvalue_user_source_names_the_threads_comment_channel() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        channel: "threads".into(),
        source: s("comments"),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::UserSource).await, s("Threads Comment"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1073 "getSystemFieldValue > user_source names the instagram comment channel"
#[tokio::test]
async fn getsystemfieldvalue_user_source_names_the_instagram_comment_channel() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        channel: "instagram".into(),
        source: s("comments"),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::UserSource).await,
        s("Facebook/IG Comment")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1092 "getSystemFieldValue > user_source falls back to the raw value for an unmapped source"
#[tokio::test]
async fn getsystemfieldvalue_user_source_falls_back_to_the_raw_value_for_an_unmapped_source() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        source: s("some_future_source"),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::UserSource).await,
        s("some_future_source")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1106 "getSystemFieldValue > user_source returns Unknown without a contact inbox source"
#[tokio::test]
async fn getsystemfieldvalue_user_source_returns_unknown_without_a_contact_inbox_source() {
    let h = base();
    assert_eq!(
        get(&h, &with_inbox(None), F::UserSource).await,
        s("Unknown")
    );
    let context = with_inbox(Some(ContactInboxModel {
        source: None,
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::UserSource).await, s("Unknown"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1123 "getSystemFieldValue > comment fields resolve from the stored message pointer"
#[tokio::test]
async fn getsystemfieldvalue_comment_fields_resolve_from_the_stored_message_pointer() {
    let h = base();
    let at = iso_ms("2026-01-04T03:04:05.000Z");
    h.fake.set(|st| {
        st.message = Some(MessageModel {
            id: "message-1".into(),
            created_at: Some(at),
            source_id: s("user-comment-1"),
            text: s("Nice post"),
            content_attributes: Some(serde_json::json!({ "postId": "post-1" })),
            deleted_at: None,
        })
    });
    let context = with_inbox(Some(ContactInboxModel {
        last_comment_message_id: s("message-1"),
        last_comment_message_at: Some(at),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::LastCommentId).await,
        s("user-comment-1")
    );
    assert_eq!(get(&h, &context, F::LastFbComment).await, s("Nice post"));
    assert_eq!(get(&h, &context, F::LastPostId).await, s("post-1"));
    assert!(
        h.fake
            .called("find_message_by_id(message-1, 2026-01-04T03:04:05.000Z, workspace-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1167 "getSystemFieldValue > comment fields ignore a deleted pointed message"
#[tokio::test]
async fn getsystemfieldvalue_comment_fields_ignore_a_deleted_pointed_message() {
    let h = base();
    let at = iso_ms("2026-01-04T03:04:05.000Z");
    h.fake.set(|st| {
        st.message = Some(MessageModel {
            id: "message-1".into(),
            created_at: Some(at),
            source_id: s("user-comment-1"),
            text: s("Deleted comment"),
            content_attributes: Some(serde_json::json!({ "postId": "post-1" })),
            deleted_at: Some(iso_ms("2026-01-05T00:00:00.000Z")),
        })
    });
    let context = with_inbox(Some(ContactInboxModel {
        last_comment_message_id: s("message-1"),
        last_comment_message_at: Some(at),
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::LastCommentId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1192 "getSystemFieldValue > last_commented_post_text is lazily fetched and cached"
#[tokio::test]
async fn getsystemfieldvalue_last_commented_post_text_is_lazily_fetched_and_cached() {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| {
            i.integration_messenger = Some(IntegrationRow {
                id: "messenger-1".into(),
                auth: messenger_auth(),
                ..Default::default()
            })
        });
        st.post_message = Ok(s("Post text"));
        st.message = Some(MessageModel {
            id: "message-1".into(),
            source_id: s("user-comment-1"),
            text: s("Nice post"),
            content_attributes: Some(serde_json::json!({ "postId": "post-1" })),
            created_at: Some(iso_ms("2026-01-04T03:04:05.000Z")),
            deleted_at: None,
        });
    });
    let context = with_inbox(Some(ContactInboxModel {
        last_comment_message_id: s("message-1"),
        last_comment_message_at: Some(iso_ms("2026-01-04T03:04:05.000Z")),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::LastCommentedPostText).await,
        s("Post text")
    );
    assert_eq!(
        get(&h, &context, F::LastCommentedPostText).await,
        s("Post text")
    );
    assert_eq!(h.fake.count("messenger_post_message"), 1);
    assert!(h.fake.called(&format!(
        "messenger_post_message({}, post-1)",
        messenger_auth()
    )));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1241 "getSystemFieldValue > me creates a signed privacy link"
#[tokio::test]
async fn getsystemfieldvalue_me_creates_a_signed_privacy_link() {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| {
            i.integration_messenger = Some(IntegrationRow {
                id: "messenger-integration-1".into(),
                name: s("Messenger Page"),
                ..Default::default()
            })
        });
        st.app_url = "https://brand.example".into();
        st.me_row_id = "system-field-1".into();
    });
    let value = get(&h, &ctx(), F::Me).await;
    let hash = h.encryption.sign_me_link(&MeLinkParams {
        workspace_id: "workspace-1".into(),
        source_id: "source-1".into(),
        integration_id: "messenger-integration-1".into(),
        form_id: "system-field-1".into(),
    });
    assert_eq!(
        value,
        Some(format!(
            "https://brand.example/extensions/me/?w=workspace-1&u=source-1&ib=messenger-integration-1&id=system-field-1&hash={hash}"
        ))
    );
    assert!(h.fake.called(
        "create_me_system_field(workspace-1, messenger, messenger-integration-1, source-1, contact-inbox-1, conversation-1, contact-1)"
    ));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1280 "getSystemFieldValue > me returns null without creating a link when workspace is scheduled for deletion"
#[tokio::test]
async fn getsystemfieldvalue_me_returns_null_without_creating_a_link_when_workspace_is_scheduled_for_deletion()
 {
    let h = base();
    let context = VariableContext {
        workspace: Some(WorkspaceModel {
            scheduled_deletion_at: Some(iso_ms("2026-07-23T00:00:00.000Z")),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::Me).await, None);
    assert_eq!(h.fake.count("find_inbox_with_integrations"), 0);
    assert_eq!(h.fake.count("workspace_app_url"), 0);
    assert_eq!(h.fake.count("create_me_system_field"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1298 "getSystemFieldValue > location and flow step fields resolve from persisted contact state"
#[tokio::test]
async fn getsystemfieldvalue_location_and_flow_step_fields_resolve_from_persisted_contact_state() {
    let h = base();
    h.fake.set(|st| {
        st.dm_conversation = Some(ConversationModel {
            last_step: s("step-1"),
            current_step: s("step-2"),
            ..Default::default()
        })
    });
    let context = with_contact(ContactModel {
        location: Some(ContactLocation {
            latitude: Some(10.75),
            longitude: Some(106.66),
        }),
        ..contact()
    });
    assert_eq!(get(&h, &context, F::LastLatitude).await, s("10.75"));
    assert_eq!(get(&h, &context, F::LastLongitude).await, s("106.66"));
    assert_eq!(get(&h, &context, F::LastStep).await, s("step-1"));
    assert_eq!(get(&h, &context, F::CurrentStep).await, s("step-2"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1335 "getSystemFieldValue > profile_pic resolves storage paths to public URLs"
#[tokio::test]
async fn getsystemfieldvalue_profile_pic_resolves_storage_paths_to_public_urls() {
    let h = base();
    h.fake.set(|st| st.tenant = storage_tenant());
    let context = with_contact(ContactModel {
        avatar: s("public/space/workspace-1/avatars/a.png"),
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::ProfilePic).await,
        s("http://localhost:3123/storage/public/space/workspace-1/avatars/a.png")
    );
    assert_eq!(h.fake.count("ensure_contact_avatar_mirrored"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1356 "getSystemFieldValue > avatar keeps an absolute mirrored URL unchanged"
#[tokio::test]
async fn getsystemfieldvalue_avatar_keeps_an_absolute_mirrored_url_unchanged() {
    let h = base();
    let context = with_contact(ContactModel {
        avatar: s("https://cdn.example.com/a.png"),
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::Avatar).await,
        s("https://cdn.example.com/a.png")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1370 "getSystemFieldValue > avatar resolves a mirrored storage key to its public URL"
#[tokio::test]
async fn getsystemfieldvalue_avatar_resolves_a_mirrored_storage_key_to_its_public_url() {
    let h = base();
    h.fake.set(|st| st.tenant = storage_tenant());
    let context = with_contact(ContactModel {
        avatar: s("public/space/workspace-1/avatars/avatar-field.png"),
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::Avatar).await,
        s("http://localhost:3123/storage/public/space/workspace-1/avatars/avatar-field.png")
    );
    assert_eq!(h.fake.count("ensure_contact_avatar_mirrored"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1391 "getSystemFieldValue > profile_pic returns a durable URL for a pending messenger avatar"
#[tokio::test]
async fn getsystemfieldvalue_profile_pic_returns_a_durable_url_for_a_pending_messenger_avatar() {
    let h = base();
    h.fake.set(|st| {
        st.tenant = storage_tenant();
        st.mirrored_avatar = s("public/space/workspace-1/avatars/mirrored.png");
    });
    let context = VariableContext {
        contact: ContactModel {
            avatar: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            channel: "messenger".into(),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::ProfilePic).await,
        s("http://localhost:3123/storage/public/space/workspace-1/avatars/mirrored.png")
    );
    assert!(
        h.fake
            .called("ensure_contact_avatar_mirrored(contact-inbox-1, workspace-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1391 "getSystemFieldValue > avatar returns a durable URL for a pending instagram avatar"
#[tokio::test]
async fn getsystemfieldvalue_avatar_returns_a_durable_url_for_a_pending_instagram_avatar() {
    let h = base();
    h.fake.set(|st| {
        st.tenant = storage_tenant();
        st.mirrored_avatar = s("public/space/workspace-1/avatars/mirrored.png");
    });
    let context = VariableContext {
        contact: ContactModel {
            avatar: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            channel: "instagram".into(),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::Avatar).await,
        s("http://localhost:3123/storage/public/space/workspace-1/avatars/mirrored.png")
    );
    assert!(
        h.fake
            .called("ensure_contact_avatar_mirrored(contact-inbox-1, workspace-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1419 "getSystemFieldValue > profile_pic resolves a no-avatar sentinel to its public URL"
#[tokio::test]
async fn getsystemfieldvalue_profile_pic_resolves_a_no_avatar_sentinel_to_its_public_url() {
    let h = base();
    h.fake.set(|st| {
        st.tenant = storage_tenant();
        st.mirrored_avatar = s("public/img/no_avatar.jpg?time=1234");
    });
    let context = with_contact(ContactModel {
        avatar: None,
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::ProfilePic).await,
        s("http://localhost:3123/storage/public/img/no_avatar.jpg?time=1234")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1419 "getSystemFieldValue > avatar resolves a no-avatar sentinel to its public URL"
#[tokio::test]
async fn getsystemfieldvalue_avatar_resolves_a_no_avatar_sentinel_to_its_public_url() {
    let h = base();
    h.fake.set(|st| {
        st.tenant = storage_tenant();
        st.mirrored_avatar = s("public/img/no_avatar.jpg?time=1234");
    });
    let context = with_contact(ContactModel {
        avatar: None,
        ..contact()
    });
    assert_eq!(
        get(&h, &context, F::Avatar).await,
        s("http://localhost:3123/storage/public/img/no_avatar.jpg?time=1234")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1442 "getSystemFieldValue > profile_pic remains null when the mirror reports a missing contact"
#[tokio::test]
async fn getsystemfieldvalue_profile_pic_remains_null_when_the_mirror_reports_a_missing_contact() {
    let h = base();
    let context = with_contact(ContactModel {
        avatar: None,
        ..contact()
    });
    assert_eq!(get(&h, &context, F::ProfilePic).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1442 "getSystemFieldValue > avatar remains null when the mirror reports a missing contact"
#[tokio::test]
async fn getsystemfieldvalue_avatar_remains_null_when_the_mirror_reports_a_missing_contact() {
    let h = base();
    let context = with_contact(ContactModel {
        avatar: None,
        ..contact()
    });
    assert_eq!(get(&h, &context, F::Avatar).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1456 "getSystemFieldValue > profile_pic returns null for a WhatsApp contact without an avatar"
#[tokio::test]
async fn getsystemfieldvalue_profile_pic_returns_null_for_a_whatsapp_contact_without_an_avatar() {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            avatar: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            channel: "whatsapp".into(),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::ProfilePic).await, None);
    assert_eq!(h.fake.count("ensure_contact_avatar_mirrored"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1456 "getSystemFieldValue > avatar returns null for a WhatsApp contact without an avatar"
#[tokio::test]
async fn getsystemfieldvalue_avatar_returns_null_for_a_whatsapp_contact_without_an_avatar() {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            avatar: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            channel: "whatsapp".into(),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::Avatar).await, None);
    assert_eq!(h.fake.count("ensure_contact_avatar_mirrored"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1475 "getSystemFieldValue > account_image resolves workspace logo storage paths"
#[tokio::test]
async fn getsystemfieldvalue_account_image_resolves_workspace_logo_storage_paths() {
    let h = base();
    h.fake.set(|st| st.tenant = storage_tenant());
    let context = VariableContext {
        workspace: Some(WorkspaceModel {
            logo: s("public/space/workspace-1/logo.png"),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::AccountImage).await,
        s("http://localhost:3123/storage/public/space/workspace-1/logo.png")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1495 "getSystemFieldValue > locale2 returns the language from underscore and hyphen locales"
#[tokio::test]
async fn getsystemfieldvalue_locale2_returns_the_language_from_underscore_and_hyphen_locales() {
    let h = base();
    let locale = |l: &str| {
        with_contact(ContactModel {
            locale: s(l),
            ..contact()
        })
    };
    assert_eq!(get(&h, &locale("en_US"), F::Locale2).await, s("en"));
    assert_eq!(get(&h, &locale("en-US"), F::Locale2).await, s("en"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1515 "getSystemFieldValue > last_seen uses the context contact inbox read timestamp"
#[tokio::test]
async fn getsystemfieldvalue_last_seen_uses_the_context_contact_inbox_read_timestamp() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        contact_last_read_at: Some(iso_ms("2026-01-03T03:04:05.000Z")),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::LastSeen).await,
        s("2026-01-03 03:04:05")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1529 "getSystemFieldValue > last_interaction uses the context contact inbox inbound timestamp"
#[tokio::test]
async fn getsystemfieldvalue_last_interaction_uses_the_context_contact_inbox_inbound_timestamp() {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        last_incoming_message_at: Some(iso_ms("2026-01-03T03:04:05.000Z")),
        ..contact_inbox()
    }));
    assert_eq!(
        get(&h, &context, F::LastInteraction).await,
        s("2026-01-03 03:04:05")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1543 "getSystemFieldValue > last_seen and last_interaction return null when no inbox timestamp exists"
#[tokio::test]
async fn getsystemfieldvalue_last_seen_and_last_interaction_return_null_when_no_inbox_timestamp_exists()
 {
    let h = base();
    let context = with_inbox(Some(ContactInboxModel {
        contact_last_read_at: None,
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::LastSeen).await, None);
    let context = with_inbox(Some(ContactInboxModel {
        last_incoming_message_at: None,
        ..contact_inbox()
    }));
    assert_eq!(get(&h, &context, F::LastInteraction).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1568 "getSystemFieldValue > subscribed_date formats the context contact inbox createdAt in the contact timezone"
#[tokio::test]
async fn getsystemfieldvalue_subscribed_date_formats_the_context_contact_inbox_createdat_in_the_contact_timezone()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("Asia/Ho_Chi_Minh"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            created_at: Some(iso_ms("2026-01-01T23:30:00.000Z")),
            ..contact_inbox()
        }),
        workspace: Some(WorkspaceModel {
            timezone: s("UTC"),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::SubscribedDate).await, s("2026-01-02"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1586 "getSystemFieldValue > subscribed_date returns null without a context contact inbox"
#[tokio::test]
async fn getsystemfieldvalue_subscribed_date_returns_null_without_a_context_contact_inbox() {
    let h = base();
    assert_eq!(get(&h, &with_inbox(None), F::SubscribedDate).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1595 "getSystemFieldValue > last_seen formats using the contact timezone before the workspace timezone"
#[tokio::test]
async fn getsystemfieldvalue_last_seen_formats_using_the_contact_timezone_before_the_workspace_timezone()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("Asia/Ho_Chi_Minh"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            contact_last_read_at: Some(iso_ms("2026-01-01T23:30:00.000Z")),
            ..contact_inbox()
        }),
        workspace: Some(WorkspaceModel {
            timezone: s("UTC"),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::LastSeen).await,
        s("2026-01-02 06:30:00")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1614 "getSystemFieldValue > last_interaction falls back to the contact timezone when workspace is missing"
#[tokio::test]
async fn getsystemfieldvalue_last_interaction_falls_back_to_the_contact_timezone_when_workspace_is_missing()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("Asia/Ho_Chi_Minh"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            last_incoming_message_at: Some(iso_ms("2026-01-01T23:30:00.000Z")),
            ..contact_inbox()
        }),
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::LastInteraction).await,
        s("2026-01-02 06:30:00")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1630 "getSystemFieldValue > last_seen and last_interaction fall back to UTC when timezone is null"
#[tokio::test]
async fn getsystemfieldvalue_last_seen_and_last_interaction_fall_back_to_utc_when_timezone_is_null()
{
    let h = base();
    let at = Some(iso_ms("2026-01-01T23:30:00.000Z"));
    let context = VariableContext {
        contact: ContactModel {
            timezone: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            contact_last_read_at: at,
            ..contact_inbox()
        }),
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::LastSeen).await,
        s("2026-01-01 23:30:00")
    );
    let context = VariableContext {
        contact: ContactModel {
            timezone: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            last_incoming_message_at: at,
            ..contact_inbox()
        }),
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::LastInteraction).await,
        s("2026-01-01 23:30:00")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1661 "getSystemFieldValue > last_seen normalizes legacy numeric contact timezone values"
#[tokio::test]
async fn getsystemfieldvalue_last_seen_normalizes_legacy_numeric_contact_timezone_values() {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("7"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            contact_last_read_at: Some(iso_ms("2026-01-01T23:30:00.000Z")),
            ..contact_inbox()
        }),
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::LastSeen).await,
        s("2026-01-02 06:30:00")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1697 "getSystemFieldValue — contact profile columns > reads plain contact columns straight through"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_reads_plain_contact_columns_straight_through()
{
    let h = base();
    let context = with_contact(profile_contact());
    for (key, value) in [
        (F::FirstName, "Ada"),
        (F::LastName, "Lovelace"),
        (F::Email, "ada@example.com"),
        (F::Phone, "+84900000000"),
        (F::UserCountry, "VN"),
        (F::UserState, "Ho Chi Minh"),
        (F::UserCity, "Thu Duc"),
        // timezone follows the signed-offset shape; timezone_name maps to IANA.
        (F::Timezone, "+7"),
    ] {
        assert_eq!(get(&h, &context, key).await, s(value), "{key}");
    }
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1716 "getSystemFieldValue — contact profile columns > full_name joins the parts and drops the missing half"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_full_name_joins_the_parts_and_drops_the_missing_half()
 {
    let h = base();
    assert_eq!(
        get(&h, &with_contact(profile_contact()), F::FullName).await,
        s("Ada Lovelace")
    );
    let context = with_contact(ContactModel {
        last_name: None,
        ..profile_contact()
    });
    assert_eq!(get(&h, &context, F::FullName).await, s("Ada"));
    let context = with_contact(ContactModel {
        first_name: None,
        last_name: None,
        ..profile_contact()
    });
    assert_eq!(get(&h, &context, F::FullName).await, s(""));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1747 "getSystemFieldValue — contact profile columns > gender is localised through the workspace language even when the contact speaks another"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_gender_is_localised_through_the_workspace_language_even_when_the_contact_speaks_another()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            locale: s("en_US"),
            ..profile_contact()
        },
        contact_inbox: Some(ContactInboxModel {
            language: s("en"),
            ..contact_inbox()
        }),
        workspace: Some(WorkspaceModel {
            language: s("vi"),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::Gender).await, s("Chị"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1766 "getSystemFieldValue — contact profile columns > gender passes an undefined language when there is no workspace"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_gender_passes_an_undefined_language_when_there_is_no_workspace()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            locale: s("vi_VN"),
            ..profile_contact()
        },
        contact_inbox: Some(ContactInboxModel {
            language: s("vi"),
            ..contact_inbox()
        }),
        workspace: None,
        ..ctx()
    };
    // No workspace language: the English label, not the contact's own locale.
    assert_eq!(get(&h, &context, F::Gender).await, s("Female"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1785 "getSystemFieldValue — contact profile columns > last_ref prefers the contact column over the referral payload"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_last_ref_prefers_the_contact_column_over_the_referral_payload()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            ref_: s("from-contact"),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            referral: Some(Referral {
                ref_: s("from-referral"),
                ..Default::default()
            }),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::LastRef).await, s("from-contact"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1800 "getSystemFieldValue — contact profile columns > last_ref falls back to the referral payload, then to null"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_last_ref_falls_back_to_the_referral_payload_then_to_null()
 {
    let h = base();
    let context = VariableContext {
        contact: ContactModel {
            ref_: None,
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            referral: Some(Referral {
                ref_: s("from-referral"),
                ..Default::default()
            }),
            ..contact_inbox()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::LastRef).await, s("from-referral"));
    let context = VariableContext {
        contact: ContactModel {
            ref_: None,
            ..contact()
        },
        contact_inbox: None,
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::LastRef).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1824 "getSystemFieldValue — contact profile columns > last_order stays null until an order source exists"
#[tokio::test]
async fn getsystemfieldvalue_contact_profile_columns_last_order_stays_null_until_an_order_source_exists()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastOrder).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1837 "getSystemFieldValue — workspace and account fields > workspace and account aliases resolve from the same sources"
#[tokio::test]
async fn getsystemfieldvalue_workspace_and_account_fields_workspace_and_account_aliases_resolve_from_the_same_sources()
 {
    let h = base();
    h.fake
        .set(|st| st.api_token = Ok("cbx_ws_plaintext".into()));
    assert_eq!(get(&h, &ctx(), F::WorkspaceId).await, s("workspace-1"));
    assert_eq!(get(&h, &ctx(), F::AccountId).await, s("workspace-1"));
    assert_eq!(get(&h, &ctx(), F::WorkspaceName).await, s("Workspace One"));
    assert_eq!(get(&h, &ctx(), F::AccountName).await, s("Workspace One"));
    assert_eq!(get(&h, &ctx(), F::ApiKey).await, s("cbx_ws_plaintext"));
    assert!(h.fake.called("resolve_default_api_token(workspace-1)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1864 "getSystemFieldValue — workspace and account fields > page_api_key resolves the same default token as api_key"
#[tokio::test]
async fn getsystemfieldvalue_workspace_and_account_fields_page_api_key_resolves_the_same_default_token_as_api_key()
 {
    let h = base();
    h.fake
        .set(|st| st.api_token = Ok("cbx_ws_plaintext".into()));
    assert_eq!(get(&h, &ctx(), F::PageApiKey).await, s("cbx_ws_plaintext"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1872 "getSystemFieldValue — workspace and account fields > api_key resolves null when the workspace is scheduled for deletion"
#[tokio::test]
async fn getsystemfieldvalue_workspace_and_account_fields_api_key_resolves_null_when_the_workspace_is_scheduled_for_deletion()
 {
    let h = base();
    h.fake
        .set(|st| st.api_token = Ok("cbx_ws_plaintext".into()));
    let context = VariableContext {
        workspace: Some(WorkspaceModel {
            scheduled_deletion_at: Some(iso_ms("2026-01-01T00:00:00.000Z")),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::ApiKey).await, None);
    assert_eq!(h.fake.count("resolve_default_api_token"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1887 "getSystemFieldValue — workspace and account fields > api_key resolves null when resolveDefaultTokenPlaintext throws, instead of failing the render"
#[tokio::test]
async fn getsystemfieldvalue_workspace_and_account_fields_api_key_resolves_null_when_resolvedefaulttokenplaintext_throws_instead_of_failing_the_render()
 {
    let h = base();
    h.fake
        .set(|st| st.api_token = Err("decrypt failed: AAD mismatch".into()));
    assert_eq!(get(&h, &ctx(), F::ApiKey).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1898 "getSystemFieldValue — workspace and account fields > workspace-backed fields are null without a workspace, but ids and api_key survive"
#[tokio::test]
async fn getsystemfieldvalue_workspace_and_account_fields_workspace_backed_fields_are_null_without_a_workspace_but_ids_and_api_key_survive()
 {
    let h = base();
    h.fake
        .set(|st| st.api_token = Ok("cbx_ws_plaintext".into()));
    let context = VariableContext {
        workspace: None,
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::WorkspaceName).await, None);
    assert_eq!(get(&h, &context, F::AccountName).await, None);
    assert_eq!(get(&h, &context, F::ApiKey).await, s("cbx_ws_plaintext"));
    assert_eq!(get(&h, &context, F::WorkspaceId).await, s("workspace-1"));
    assert_eq!(get(&h, &context, F::AccountId).await, s("workspace-1"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1935 "getSystemFieldValue — clock fields > current_time uses workspace timezone while current_user_time uses contact timezone first"
#[tokio::test]
async fn getsystemfieldvalue_clock_fields_current_time_uses_workspace_timezone_while_current_user_time_uses_contact_timezone_first()
 {
    let h = Harness::at(iso_ms("2026-01-02T03:04:05.000Z"), base_state());
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("Asia/Tokyo"),
            ..profile_contact()
        },
        workspace: Some(WorkspaceModel {
            timezone: s("Asia/Bangkok"),
            ..workspace()
        }),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::CurrentTime).await,
        s("2026-01-02 10:04:05")
    );
    assert_eq!(
        get(&h, &context, F::CurrentUserTime).await,
        s("2026-01-02 12:04:05")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1949 "getSystemFieldValue — clock fields > clock fields fall back to the contact timezone, then to UTC"
#[tokio::test]
async fn getsystemfieldvalue_clock_fields_clock_fields_fall_back_to_the_contact_timezone_then_to_utc()
 {
    let h = Harness::at(iso_ms("2026-01-02T03:04:05.000Z"), base_state());
    let context = VariableContext {
        contact: ContactModel {
            timezone: s("Asia/Tokyo"),
            ..contact()
        },
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::CurrentUserTime).await,
        s("2026-01-02 12:04:05")
    );
    let context = VariableContext {
        contact: ContactModel {
            timezone: None,
            ..contact()
        },
        workspace: None,
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::CurrentTime).await,
        s("2026-01-02 03:04:05")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:1978 "getSystemFieldValue — appointment fields > booking fields resolve from explicit appointment context"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_booking_fields_resolve_from_explicit_appointment_context()
 {
    let h = base();
    h.fake.set(|st| {
        st.appointment = Some(appointment(
            "appointment-1",
            "Discovery Call",
            "2026-01-02T03:04:05.000Z",
        ))
    });
    let context = VariableContext {
        appointment_id: s("appointment-1"),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::BookingCalendar).await,
        s("Discovery Call")
    );
    assert_eq!(
        get(&h, &context, F::BookingDate).await,
        s("2026-01-02 10:04:05")
    );
    assert_eq!(get(&h, &context, F::BookingId).await, s("appointment-1"));
    assert!(
        h.fake
            .called("find_appointment(workspace-1, appointment-1)")
    );
    assert_eq!(h.fake.count("find_latest_appointment_for_contact"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2006 "getSystemFieldValue — appointment fields > booking fields fall back to the contact's latest appointment without explicit appointment context"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_booking_fields_fall_back_to_the_contact_s_latest_appointment_without_explicit_appointment_context()
 {
    let h = base();
    h.fake.set(|st| {
        st.latest_appointment = Some(appointment(
            "appointment-2",
            "Discovery Call",
            "2026-01-02T03:04:05.000Z",
        ))
    });
    assert_eq!(
        get(&h, &ctx(), F::BookingCalendar).await,
        s("Discovery Call")
    );
    assert_eq!(
        get(&h, &ctx(), F::BookingDate).await,
        s("2026-01-02 10:04:05")
    );
    assert_eq!(get(&h, &ctx(), F::BookingId).await, s("appointment-2"));
    assert_eq!(h.fake.count("find_appointment"), 0);
    assert!(
        h.fake
            .called("find_latest_appointment_for_contact(workspace-1, contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2034 "getSystemFieldValue — appointment fields > booking_id resolves null when the explicit appointment belongs to another contact"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_booking_id_resolves_null_when_the_explicit_appointment_belongs_to_another_contact()
 {
    let h = base();
    h.fake.set(|st| {
        st.appointment = Some(Appointment {
            contact_id: "someone-else".into(),
            ..appointment("appointment-9", "X", "2026-01-02T03:04:05.000Z")
        })
    });
    let context = VariableContext {
        appointment_id: s("appointment-9"),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::BookingId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2049 "getSystemFieldValue — appointment fields > booking_link resolves null without any appointment for the contact"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_booking_link_resolves_null_without_any_appointment_for_the_contact()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::BookingId).await, None);
    assert_eq!(get(&h, &ctx(), F::BookingLink).await, None);
    assert_eq!(h.fake.count("find_appointment"), 0);
    assert!(
        h.fake
            .called("find_latest_appointment_for_contact(workspace-1, contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2066 "getSystemFieldValue — appointment fields > booking_link resolves a schedule URL with an appointmentId"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_booking_link_resolves_a_schedule_url_with_an_appointmentid()
 {
    let h = base();
    h.fake.set(|st| {
        st.appointment = Some(Appointment {
            conversation_id: s("conversation-1"),
            ..appointment(
                "appointment-1",
                "Discovery Call",
                "2026-01-02T03:04:05.000Z",
            )
        });
        st.tenant = TenantSettings {
            app_url: "https://app.example.test".into(),
            ..Default::default()
        };
    });
    let context = VariableContext {
        appointment_id: s("appointment-1"),
        ..ctx()
    };
    let value = get(&h, &context, F::BookingLink).await.unwrap();
    let prefix = "https://app.example.test/booking/schedule?token=";
    assert!(
        value.starts_with(prefix) && value.len() > prefix.len(),
        "{value}"
    );
    assert_eq!(h.fake.count("find_latest_appointment_for_contact"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2090 "getSystemFieldValue — appointment fields > explicit appointmentId takes priority over the contact's latest appointment"
#[tokio::test]
async fn getsystemfieldvalue_appointment_fields_explicit_appointmentid_takes_priority_over_the_contact_s_latest_appointment()
 {
    let h = base();
    h.fake.set(|st| {
        st.appointment = Some(appointment(
            "appointment-1",
            "Discovery Call",
            "2026-01-02T03:04:05.000Z",
        ));
        st.latest_appointment = Some(appointment(
            "appointment-2",
            "Newer Calendar",
            "2026-02-02T03:04:05.000Z",
        ));
    });
    let context = VariableContext {
        appointment_id: s("appointment-1"),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::BookingCalendar).await,
        s("Discovery Call")
    );
    assert_eq!(h.fake.count("find_latest_appointment_for_contact"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2123 "getSystemFieldValue — questionnaire fields > last_points resolves the points of the contact's latest completed submission"
#[tokio::test]
async fn getsystemfieldvalue_questionnaire_fields_last_points_resolves_the_points_of_the_contact_s_latest_completed_submission()
 {
    let h = base();
    h.fake.set(|st| st.points = Some(7.0));
    assert_eq!(get(&h, &ctx(), F::LastPoints).await, s("7"));
    assert!(
        h.fake
            .called("latest_questionnaire_points(workspace-1, contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2139 "getSystemFieldValue — questionnaire fields > last_points keeps a legitimate zero score"
#[tokio::test]
async fn getsystemfieldvalue_questionnaire_fields_last_points_keeps_a_legitimate_zero_score() {
    let h = base();
    h.fake.set(|st| st.points = Some(0.0));
    assert_eq!(get(&h, &ctx(), F::LastPoints).await, s("0"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2151 "getSystemFieldValue — questionnaire fields > last_points resolves 0 when the contact has no completed scored submission"
#[tokio::test]
async fn getsystemfieldvalue_questionnaire_fields_last_points_resolves_0_when_the_contact_has_no_completed_scored_submission()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastPoints).await, s("0"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2163 "getSystemFieldValue — empty > empty resolves to an empty string"
#[tokio::test]
async fn getsystemfieldvalue_empty_empty_resolves_to_an_empty_string() {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::Empty).await, s(""));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2169 "getSystemFieldValue — empty > empty resolves to an empty string even without a contact inbox"
#[tokio::test]
async fn getsystemfieldvalue_empty_empty_resolves_to_an_empty_string_even_without_a_contact_inbox()
{
    let h = base();
    assert_eq!(get(&h, &with_inbox(None), F::Empty).await, s(""));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2190 "getSystemFieldValue — inbox sender fields > admin_name resolves the member who sent the flow from the inbox"
#[tokio::test]
async fn getsystemfieldvalue_inbox_sender_fields_admin_name_resolves_the_member_who_sent_the_flow_from_the_inbox()
 {
    let h = base();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-7".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        });
        st.team_names = vec!["Sales".into(), "Support".into()];
    });

    let context = VariableContext {
        sent_by_user_id: s("user-7"),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::AdminName).await, s("Grace"));
    assert!(h.fake.called("find_member_user(workspace-1, user-7)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2204 "getSystemFieldValue — inbox sender fields > admin_team_name joins the names of all the sender's teams"
#[tokio::test]
async fn getsystemfieldvalue_inbox_sender_fields_admin_team_name_joins_the_names_of_all_the_sender_s_teams()
 {
    let h = base();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-7".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        });
        st.team_names = vec!["Sales".into(), "Support".into()];
    });

    let context = VariableContext {
        sent_by_user_id: s("user-7"),
        ..ctx()
    };
    assert_eq!(
        get(&h, &context, F::AdminTeamName).await,
        s("Sales, Support")
    );
    assert!(
        h.fake
            .called("list_team_names_by_user_id(workspace-1, user-7)")
    );
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2218 "getSystemFieldValue — inbox sender fields > both resolve null without a sender and do not query"
#[tokio::test]
async fn getsystemfieldvalue_inbox_sender_fields_both_resolve_null_without_a_sender_and_do_not_query()
 {
    let h = base();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-7".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        });
        st.team_names = vec!["Sales".into(), "Support".into()];
    });

    assert_eq!(get(&h, &ctx(), F::AdminName).await, None);
    assert_eq!(get(&h, &ctx(), F::AdminTeamName).await, None);
    assert_eq!(h.fake.count("find_member_user"), 0);
    assert_eq!(h.fake.count("list_team_names_by_user_id"), 0);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2233 "getSystemFieldValue — inbox sender fields > admin_name resolves null when the sender is no longer a workspace member"
#[tokio::test]
async fn getsystemfieldvalue_inbox_sender_fields_admin_name_resolves_null_when_the_sender_is_no_longer_a_workspace_member()
 {
    let h = base();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-7".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        });
        st.team_names = vec!["Sales".into(), "Support".into()];
    });

    h.fake.set(|st| st.member = None);
    let context = VariableContext {
        sent_by_user_id: s("user-7"),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::AdminName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2244 "getSystemFieldValue — inbox sender fields > admin_team_name resolves null when the sender belongs to no team"
#[tokio::test]
async fn getsystemfieldvalue_inbox_sender_fields_admin_team_name_resolves_null_when_the_sender_belongs_to_no_team()
 {
    let h = base();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-7".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        });
        st.team_names = vec!["Sales".into(), "Support".into()];
    });

    h.fake.set(|st| st.team_names = vec![]);
    let context = VariableContext {
        sent_by_user_id: s("user-7"),
        ..ctx()
    };
    assert_eq!(get(&h, &context, F::AdminTeamName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2269 "getSystemFieldValue — lead ads fields > last_lead_id resolves from the contact's latest lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_lead_id_resolves_from_the_contact_s_latest_lead()
{
    let h = base();
    h.fake.set(|st| st.lead = Some(latest_lead()));
    assert_eq!(get(&h, &ctx(), F::LastLeadId).await, s("lead-9"));
    assert!(h.fake.called("find_latest_lead(workspace-1, contact-1)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2269 "getSystemFieldValue — lead ads fields > last_form_id resolves from the contact's latest lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_form_id_resolves_from_the_contact_s_latest_lead()
{
    let h = base();
    h.fake.set(|st| st.lead = Some(latest_lead()));
    assert_eq!(get(&h, &ctx(), F::LastFormId).await, s("form-3"));
    assert!(h.fake.called("find_latest_lead(workspace-1, contact-1)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2269 "getSystemFieldValue — lead ads fields > last_form_name resolves from the contact's latest lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_form_name_resolves_from_the_contact_s_latest_lead()
 {
    let h = base();
    h.fake.set(|st| st.lead = Some(latest_lead()));
    assert_eq!(get(&h, &ctx(), F::LastFormName).await, s("Summer promo"));
    assert!(h.fake.called("find_latest_lead(workspace-1, contact-1)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2269 "getSystemFieldValue — lead ads fields > last_adgroup_id resolves from the contact's latest lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_adgroup_id_resolves_from_the_contact_s_latest_lead()
 {
    let h = base();
    h.fake.set(|st| st.lead = Some(latest_lead()));
    assert_eq!(get(&h, &ctx(), F::LastAdgroupId).await, s("adgroup-5"));
    assert!(h.fake.called("find_latest_lead(workspace-1, contact-1)"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2287 "getSystemFieldValue — lead ads fields > last_lead_id resolves null when the contact has no lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_lead_id_resolves_null_when_the_contact_has_no_lead()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastLeadId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2287 "getSystemFieldValue — lead ads fields > last_form_id resolves null when the contact has no lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_form_id_resolves_null_when_the_contact_has_no_lead()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastFormId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2287 "getSystemFieldValue — lead ads fields > last_form_name resolves null when the contact has no lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_form_name_resolves_null_when_the_contact_has_no_lead()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastFormName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2287 "getSystemFieldValue — lead ads fields > last_adgroup_id resolves null when the contact has no lead"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_last_adgroup_id_resolves_null_when_the_contact_has_no_lead()
 {
    let h = base();
    assert_eq!(get(&h, &ctx(), F::LastAdgroupId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2298 "getSystemFieldValue — lead ads fields > a lead without a stored form name or adgroup resolves those to null"
#[tokio::test]
async fn getsystemfieldvalue_lead_ads_fields_a_lead_without_a_stored_form_name_or_adgroup_resolves_those_to_null()
 {
    let h = base();
    h.fake.set(|st| {
        st.lead = Some(LeadAdsLead {
            form_name: None,
            adgroup_id: None,
            ..latest_lead()
        })
    });
    assert_eq!(get(&h, &ctx(), F::LastFormName).await, None);
    assert_eq!(get(&h, &ctx(), F::LastAdgroupId).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2327 "getSystemFieldValue — inbox identity fields > page_user_name reads the integration name for the inbox channel"
#[tokio::test]
async fn getsystemfieldvalue_inbox_identity_fields_page_user_name_reads_the_integration_name_for_the_inbox_channel()
 {
    let h = base();
    h.fake.set(|st| {
        st.inbox = inbox(|i| {
            i.integration_messenger = Some(IntegrationRow {
                id: "messenger-1".into(),
                name: s("Acme Page"),
                ..Default::default()
            })
        })
    });
    assert_eq!(get(&h, &ctx(), F::PageUserName).await, s("Acme Page"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2340 "getSystemFieldValue — inbox identity fields > page_user_name is null when the channel has no integration row"
#[tokio::test]
async fn getsystemfieldvalue_inbox_identity_fields_page_user_name_is_null_when_the_channel_has_no_integration_row()
 {
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    assert_eq!(get(&h, &ctx(), F::PageUserName).await, None);
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2351 "getSystemFieldValue — inbox identity fields > user_code exposes the platform-side contact id"
#[tokio::test]
async fn getsystemfieldvalue_inbox_identity_fields_user_code_exposes_the_platform_side_contact_id()
{
    let h = base();
    h.fake.set(|st| st.inbox = inbox(|_| {}));
    assert_eq!(get(&h, &ctx(), F::UserCode).await, s("source-1"));
}

/// Port of: packages/variables/__tests__/system-fields.test.ts:2359 "getSystemFieldValue — inbox identity fields > integration-backed fields are null without a contact inbox"
#[tokio::test]
async fn getsystemfieldvalue_inbox_identity_fields_integration_backed_fields_are_null_without_a_contact_inbox()
 {
    let h = base();
    assert_eq!(get(&h, &with_inbox(None), F::UserCode).await, None);
    assert_eq!(h.fake.count("find_inbox_with_integrations"), 0);
}
