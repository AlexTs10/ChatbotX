//! Port of `packages/variables/__tests__/assigned-helpers.test.ts`. The `vi.fn()` mocks are
//! [`Fake`] answers; `toHaveBeenCalledWith` checks the recorded calls.

mod support;

use chatbotx_core::variables::*;
use support::variables::*;

fn assigned() -> Harness {
    Harness::new(FakeState {
        conversation_by_contact: Some(ConversationModel {
            assigned_user_id: s("user-1"),
            ..Default::default()
        }),
        member: Some(MemberUser {
            id: "user-1".into(),
            name: s("Ada"),
            email: s("ada@example.com"),
        }),
        ..Default::default()
    })
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:60 "assigned field helpers > resolves assigned admin fields from the current conversation assignee"
#[tokio::test]
async fn assigned_field_helpers_resolves_assigned_admin_fields_from_the_current_conversation_assignee()
 {
    let h = assigned();
    let v = h.v();
    assert_eq!(
        v.resolve_assignee_name("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("Ada")
    );
    assert_eq!(
        v.resolve_assignee_email("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("ada@example.com")
    );
    assert_eq!(
        v.resolve_assignee_id("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("user-1")
    );
    assert!(
        h.fake
            .called("find_conversation_by_contact(contact-1, workspace-1)")
    );
    assert!(h.fake.called("find_member_user(workspace-1, user-1)"));
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:80 "assigned field helpers > returns null when the assigned user is no longer a workspace member"
#[tokio::test]
async fn assigned_field_helpers_returns_null_when_the_assigned_user_is_no_longer_a_workspace_member()
 {
    let h = assigned();
    h.fake.set(|st| st.member = None);
    let v = h.v();
    assert_eq!(
        v.resolve_assignee_name("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        v.resolve_assignee_email("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        v.resolve_assignee_id("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:94 "assigned field helpers > does not fall back to email when the assigned user's name is null"
#[tokio::test]
async fn assigned_field_helpers_does_not_fall_back_to_email_when_the_assigned_user_s_name_is_null()
{
    let h = assigned();
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-1".into(),
            name: None,
            email: Some("ada@example.com".into()),
        })
    });
    let v = h.v();
    assert_eq!(
        v.resolve_assignee_name("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        v.resolve_assignee_email("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("ada@example.com")
    );
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:118 "sender field helpers > resolveSenderName reads the sender's name through the workspace membership"
#[tokio::test]
async fn sender_field_helpers_resolvesendername_reads_the_sender_s_name_through_the_workspace_membership()
 {
    let h = Harness::new(FakeState {
        member: Some(MemberUser {
            id: "user-2".into(),
            name: s("Grace"),
            email: s("grace@example.com"),
        }),
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .resolve_sender_name("workspace-1", "user-2")
            .await
            .unwrap(),
        s("Grace")
    );
    assert!(h.fake.called("find_member_user(workspace-1, user-2)"));
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:133 "sender field helpers > resolveSenderName is null for a user who is not a workspace member or has no name"
#[tokio::test]
async fn sender_field_helpers_resolvesendername_is_null_for_a_user_who_is_not_a_workspace_member_or_has_no_name()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .resolve_sender_name("workspace-1", "user-2")
            .await
            .unwrap(),
        None
    );
    h.fake.set(|st| {
        st.member = Some(MemberUser {
            id: "user-2".into(),
            name: None,
            email: Some("grace@example.com".into()),
        })
    });
    assert_eq!(
        h.v()
            .resolve_sender_name("workspace-1", "user-2")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:148 "sender field helpers > listSenderTeamNames joins the team names of the sender in the workspace"
#[tokio::test]
async fn sender_field_helpers_listsenderteamnames_joins_the_team_names_of_the_sender_in_the_workspace()
 {
    let h = Harness::new(FakeState {
        team_names: vec!["Sales".into(), "Support".into()],
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .list_sender_team_names("workspace-1", "user-2")
            .await
            .unwrap(),
        s("Sales, Support")
    );
    assert!(
        h.fake
            .called("list_team_names_by_user_id(workspace-1, user-2)")
    );
}

/// Port of: packages/variables/__tests__/assigned-helpers.test.ts:160 "sender field helpers > listSenderTeamNames is null when the sender is in no team"
#[tokio::test]
async fn sender_field_helpers_listsenderteamnames_is_null_when_the_sender_is_in_no_team() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .list_sender_team_names("workspace-1", "user-2")
            .await
            .unwrap(),
        None
    );
}
