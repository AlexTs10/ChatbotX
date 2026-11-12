//! Port of `packages/variables/__tests__/system-fields-helpers.test.ts`. The original mocks the helper
//! modules; here the helpers run on [`Fake`] data, and "which helper with
//! which arguments" is read from the recorded source calls.

mod support;

use chatbotx_core::variables::*;
use support::now_ms;
use support::variables::*;

fn context() -> VariableContext {
    VariableContext {
        contact: ContactModel {
            id: "contact-1".into(),
            workspace_id: "workspace-1".into(),
            ..Default::default()
        },
        contact_inbox: Some(ContactInboxModel {
            id: "contact-inbox-1".into(),
            ..Default::default()
        }),
        conversation: Some(ConversationModel {
            id: "conversation-1".into(),
            ..Default::default()
        }),
        workspace: Some(WorkspaceModel {
            id: "workspace-1".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn latest_conversation() -> Option<ConversationModel> {
    Some(ConversationModel {
        id: "conversation-1".into(),
        workspace_id: "workspace-1".into(),
        last_activity_at: Some(now_ms()),
        ..Default::default()
    })
}

async fn value(h: &Harness, key: SystemFieldType) -> Option<String> {
    h.v().get_system_field_value(&context(), key).await.unwrap()
}

fn limit_of(call: &str) -> &str {
    call.split(", ").nth(1).unwrap()
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:100 "getSystemFieldValue — helper-backed fields > chat history variants differ only by limit and detail flag"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_chat_history_variants_differ_only_by_limit_and_detail_flag()
 {
    let h = Harness::new(FakeState {
        latest_conversation: latest_conversation(),
        last_messages: vec![ChatMessage {
            text: s("transcript"),
            sender_type: "user".into(),
        }],
        ..Default::default()
    });
    assert_eq!(
        value(&h, SystemFieldType::ChatHistory).await,
        s("transcript")
    );
    assert_eq!(
        limit_of(&h.fake.last_call_to("list_last_messages").unwrap()),
        "50"
    );
    assert_eq!(
        value(&h, SystemFieldType::ChatHistoryLarge).await,
        s("transcript")
    );
    assert_eq!(
        limit_of(&h.fake.last_call_to("list_last_messages").unwrap()),
        "200"
    );
    assert_eq!(
        value(&h, SystemFieldType::ChatHistoryDetails).await,
        s("User: transcript")
    );
    assert_eq!(
        limit_of(&h.fake.last_call_to("list_last_messages").unwrap()),
        "50"
    );
    assert_eq!(
        value(&h, SystemFieldType::ChatHistoryDetailsLarge).await,
        s("User: transcript")
    );
    assert_eq!(
        limit_of(&h.fake.last_call_to("list_last_messages").unwrap()),
        "200"
    );
    assert!(
        h.fake
            .calls_to("find_latest_conversation_by_contact")
            .iter()
            .all(|c| c == "find_latest_conversation_by_contact(contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:128 "getSystemFieldValue — helper-backed fields > ai queued messages delegates to its queued-message helper"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_ai_queued_messages_delegates_to_its_queued_message_helper()
 {
    let h = Harness::new(FakeState {
        incoming_texts: vec!["second".into(), "first".into()],
        ..Default::default()
    });
    assert_eq!(
        value(&h, SystemFieldType::AiQueuedMessages).await,
        s("first\nsecond")
    );
    assert!(
        h.fake
            .called("list_incoming_texts_by_contact_inbox(contact-inbox-1, 50, null, workspace-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:138 "getSystemFieldValue — helper-backed fields > assigned admin fields resolve from the conversation assignee"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_assigned_admin_fields_resolve_from_the_conversation_assignee()
 {
    let h = Harness::new(FakeState {
        conversation_by_contact: Some(ConversationModel {
            assigned_user_id: s("admin-1"),
            ..Default::default()
        }),
        member: Some(MemberUser {
            id: "admin-1".into(),
            name: s("Ada"),
            email: s("ada@example.com"),
        }),
        ..Default::default()
    });
    assert_eq!(
        value(&h, SystemFieldType::AssignedAdminName).await,
        s("Ada")
    );
    assert_eq!(
        value(&h, SystemFieldType::AssignedAdminEmail).await,
        s("ada@example.com")
    );
    assert_eq!(
        value(&h, SystemFieldType::AssignedAdminId).await,
        s("admin-1")
    );
    assert_eq!(
        h.fake.calls_to("find_conversation_by_contact"),
        vec!["find_conversation_by_contact(contact-1, workspace-1)"; 3]
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:167 "getSystemFieldValue — helper-backed fields > member_name shares the assignee name resolver, team_name resolves by contact"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_member_name_shares_the_assignee_name_resolver_team_name_resolves_by_contact()
 {
    let h = Harness::new(FakeState {
        conversation_by_contact: Some(ConversationModel {
            assigned_user_id: s("admin-1"),
            assigned_inbox_team_id: s("team-1"),
            ..Default::default()
        }),
        member: Some(MemberUser {
            id: "admin-1".into(),
            name: s("Grace"),
            email: None,
        }),
        team_name: s("Support"),
        ..Default::default()
    });
    assert_eq!(value(&h, SystemFieldType::MemberName).await, s("Grace"));
    assert_eq!(value(&h, SystemFieldType::TeamName).await, s("Support"));
    assert!(
        h.fake
            .called("find_conversation_by_contact(contact-1, workspace-1)")
    );
    assert!(h.fake.called("find_conversation_by_contact(contact-1)"));
    assert!(h.fake.called("find_inbox_team_name(team-1)"));
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:185 "getSystemFieldValue — helper-backed fields > tag and note fields each call their own contact helper"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_tag_and_note_fields_each_call_their_own_contact_helper()
 {
    let h = Harness::new(FakeState {
        tags: vec!["vip".into(), "lead".into()],
        notes: vec!["note two".into(), "note one".into()],
        ..Default::default()
    });
    assert_eq!(value(&h, SystemFieldType::UserTags).await, s("vip, lead"));
    // All notes, newest first; the latest note is the first one.
    assert_eq!(
        value(&h, SystemFieldType::UserNotes).await,
        s("note two\nnote one")
    );
    assert_eq!(
        value(&h, SystemFieldType::LastUserNote).await,
        s("note two")
    );
    assert!(h.fake.called("list_tag_names_by_contact_id(contact-1)"));
    assert_eq!(
        h.fake.calls_to("list_contact_note_texts"),
        vec!["list_contact_note_texts(workspace-1, contact-1)"; 2]
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:211 "getSystemFieldValue — helper-backed fields > ad_ids and ad_id read the contact's ad touch log through their own helpers"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_ad_ids_and_ad_id_read_the_contact_s_ad_touch_log_through_their_own_helpers()
 {
    let h = Harness::new(FakeState {
        ad_ids: vec!["ad-1".into(), "ad-2".into()],
        ..Default::default()
    });
    assert_eq!(value(&h, SystemFieldType::AdIds).await, s("ad-1, ad-2"));
    assert_eq!(value(&h, SystemFieldType::AdId).await, s("ad-1"));
    assert!(h.fake.called("list_ad_ids(workspace-1, contact-1, 50)"));
    assert!(h.fake.called("list_ad_ids(workspace-1, contact-1, 1)"));
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:232 "getSystemFieldValue — helper-backed fields > last_input and last_input_type call separate helpers"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_last_input_and_last_input_type_call_separate_helpers()
 {
    let h = Harness::new(FakeState {
        latest_conversation: latest_conversation(),
        latest_input: Some(MessageWithAttachments {
            content_type: "text".into(),
            text: s("hello"),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(value(&h, SystemFieldType::LastInput).await, s("hello"));
    assert_eq!(value(&h, SystemFieldType::LastInputType).await, s("text"));
    assert_eq!(
        h.fake.calls_to("find_latest_conversation_by_contact"),
        vec!["find_latest_conversation_by_contact(contact-1)"; 2]
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:247 "getSystemFieldValue — helper-backed fields > last_text_input calls its own helper"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_last_text_input_calls_its_own_helper() {
    let h = Harness::new(FakeState {
        latest_conversation: latest_conversation(),
        latest_input: Some(MessageWithAttachments {
            content_type: "text".into(),
            text: s("typed text"),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(
        value(&h, SystemFieldType::LastTextInput).await,
        s("typed text")
    );
    assert!(
        h.fake
            .called("find_latest_conversation_by_contact(contact-1)")
    );
}

/// Port of: packages/variables/__tests__/system-fields-helpers.test.ts:257 "getSystemFieldValue — helper-backed fields > a null from any helper passes straight through"
#[tokio::test]
async fn getsystemfieldvalue_helper_backed_fields_a_null_from_any_helper_passes_straight_through() {
    let h = Harness::new(FakeState::default());
    for key in [
        SystemFieldType::ChatHistory,
        SystemFieldType::AiQueuedMessages,
        SystemFieldType::AdIds,
        SystemFieldType::AdId,
        SystemFieldType::LastUserNote,
        SystemFieldType::MemberName,
        SystemFieldType::TeamName,
        SystemFieldType::AssignedAdminName,
        SystemFieldType::AssignedAdminEmail,
        SystemFieldType::AssignedAdminId,
        SystemFieldType::LastInput,
        SystemFieldType::LastInputType,
    ] {
        assert_eq!(
            value(&h, key).await,
            None,
            "{key} did not pass a null helper result through"
        );
    }
    // `listContactTagsString` / `listContactNotesString` always return a
    // string; with no tags / notes it is empty.
    assert_eq!(value(&h, SystemFieldType::UserTags).await, s(""));
    assert_eq!(value(&h, SystemFieldType::UserNotes).await, s(""));
}
