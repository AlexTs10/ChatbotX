//! Port of `packages/variables/__tests__/queued-messages.test.ts`.

mod support;

use chatbotx_core::variables::*;
use support::iso_ms;
use support::variables::*;

fn context(contact_inbox: Option<ContactInboxModel>) -> VariableContext {
    VariableContext {
        contact: ContactModel {
            id: "contact-1".into(),
            workspace_id: "workspace-1".into(),
            ..Default::default()
        },
        contact_inbox,
        conversation: None,
        workspace: Some(WorkspaceModel {
            id: "workspace-1".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn contact_inbox() -> ContactInboxModel {
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        created_at: Some(iso_ms("2026-01-01T00:00:00.000Z")),
        first_interaction_at: None,
        last_outbound_message_at: None,
        ..Default::default()
    }
}

/// Port of: packages/variables/__tests__/queued-messages.test.ts:55 "getQueuedMessages > uses last outbound message time before older fallback anchors"
#[tokio::test]
async fn getqueuedmessages_uses_last_outbound_message_time_before_older_fallback_anchors() {
    let h = Harness::new(FakeState {
        incoming_texts: vec!["new".into(), "old".into()],
        ..Default::default()
    });
    let ci = ContactInboxModel {
        first_interaction_at: Some(iso_ms("2026-01-02T00:00:00.000Z")),
        last_outbound_message_at: Some(iso_ms("2026-01-03T00:00:00.000Z")),
        ..contact_inbox()
    };
    assert_eq!(
        h.v().get_queued_messages(&context(Some(ci))).await.unwrap(),
        s("old\nnew")
    );
    assert!(h.fake.called(
        "list_incoming_texts_by_contact_inbox(contact-inbox-1, 50, 2026-01-03T00:00:00.000Z, workspace-1)"
    ));
}

/// Port of: packages/variables/__tests__/queued-messages.test.ts:78 "getQueuedMessages > falls back from first interaction time to contact inbox creation time"
#[tokio::test]
async fn getqueuedmessages_falls_back_from_first_interaction_time_to_contact_inbox_creation_time() {
    let h = Harness::new(FakeState {
        incoming_texts: vec!["hello".into()],
        ..Default::default()
    });
    let ci = ContactInboxModel {
        first_interaction_at: Some(iso_ms("2026-01-02T00:00:00.000Z")),
        ..contact_inbox()
    };
    h.v().get_queued_messages(&context(Some(ci))).await.unwrap();
    assert!(
        h.fake
            .last_call_to("list_incoming_texts_by_contact_inbox")
            .unwrap()
            .contains("2026-01-02T00:00:00.000Z")
    );
    h.v()
        .get_queued_messages(&context(Some(contact_inbox())))
        .await
        .unwrap();
    assert!(
        h.fake
            .last_call_to("list_incoming_texts_by_contact_inbox")
            .unwrap()
            .contains("2026-01-01T00:00:00.000Z")
    );
}

/// Port of: packages/variables/__tests__/queued-messages.test.ts:101 "getQueuedMessages > returns null for empty results"
#[tokio::test]
async fn getqueuedmessages_returns_null_for_empty_results() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .get_queued_messages(&context(Some(contact_inbox())))
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/queued-messages.test.ts:109 "getQueuedMessages > returns null without querying when contact inbox is unavailable"
#[tokio::test]
async fn getqueuedmessages_returns_null_without_querying_when_contact_inbox_is_unavailable() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v().get_queued_messages(&context(None)).await.unwrap(),
        None
    );
    assert_eq!(h.fake.count("list_incoming_texts_by_contact_inbox"), 0);
}
