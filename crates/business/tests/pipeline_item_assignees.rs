//! Port of `packages/business/__tests__/pipeline-item-assignees.test.ts`.
//! Adapted: the mocked `listWorkspaceMembers` is a real `WorkspaceMember` row
//! (member 31 of user 1001 in `WS`).

mod common;

use chatbotx_business::pipeline::assignees::{Assignees, primary_from_sets};
use common::{WS, setup, two_workspaces, workspace_member};

#[tokio::test]
async fn primary_from_sets_follows_the_first_member_that_resolves_to_a_user() {
    let db = two_workspaces().await;
    setup(&db, |tx| workspace_member(tx, 31, WS, 1001)).await;
    let sets = Assignees {
        workspace_member_ids: vec![31],
        inbox_team_ids: vec![41],
    };
    assert_eq!(
        primary_from_sets(&db, WS, &sets).await.unwrap().as_deref(),
        Some("u_1001")
    );
}

#[tokio::test]
async fn primary_from_sets_falls_through_to_the_first_team_when_the_first_member_no_longer_resolves()
 {
    let db = two_workspaces().await;
    let sets = Assignees {
        workspace_member_ids: vec![31],
        inbox_team_ids: vec![41],
    };
    assert_eq!(
        primary_from_sets(&db, WS, &sets).await.unwrap().as_deref(),
        Some("t_41")
    );
}

#[tokio::test]
async fn primary_from_sets_is_null_when_nobody_is_assigned() {
    let db = two_workspaces().await;
    assert_eq!(
        primary_from_sets(&db, WS, &Assignees::default())
            .await
            .unwrap(),
        None
    );
}

/// `assignContact`: the contact's unassigned conversation and the one still
/// with the previous primary move to the current primary; a conversation
/// someone else owns keeps its assignee.
#[tokio::test]
async fn assign_contact_moves_only_unassigned_and_previous_conversations() {
    use chatbotx_business::pipeline::assignees::{ContactAssignment, apply_contact_assignments};
    use common::{contact, conversation, pipeline_row, workspace};
    let db = two_workspaces().await;
    setup(&db, |tx| {
        workspace(tx, 1003, 2003)?;
        workspace_member(tx, 31, WS, 1001)?;
        workspace_member(tx, 32, WS, 1002)?;
        workspace_member(tx, 33, WS, 1003)?;
        contact(tx, 7, WS)?;
        pipeline_row(tx, 50, WS, "Deals", None)?;
        tx.execute(
            r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position")
               VALUES (60, ?1, 50, 'New', 0)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "PipelineItem" ("id", "workspaceId", "pipelineId", "stageId", "contactId",
                   "number", "title", "status", "priority", "fieldValues", "stageEnteredAt")
               VALUES (70, ?1, 50, 60, 7, 1, 'Deal', 'open', 'medium', '{}', 0)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "PipelineItemAssignee" ("id", "workspaceId", "itemId", "workspaceMemberId")
               VALUES (80, ?1, 70, 31)"#,
            [WS],
        )?;
        // One DM conversation per contact; the other two are comment threads.
        conversation(tx, 91, WS, 7)?;
        for (id, owner) in [(92, 1002), (93, 1003)] {
            tx.execute(
                r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "assignedUserId")
                   VALUES (?1, ?2, 7, ?3, ?4)"#,
                chatbotx_db::rusqlite::params![id, WS, format!("thread-{id}"), owner],
            )?;
        }
        Ok(())
    })
    .await;
    let done = apply_contact_assignments(
        &db,
        &[ContactAssignment {
            workspace_id: WS,
            contact_id: 7,
            item_id: 70,
            assigned_id: "u_1001".into(),
            previous_assigned_id: Some("u_1002".into()),
        }],
    )
    .await
    .unwrap();
    assert_eq!(done.len(), 2);
    let owner = |id| {
        common::scalar_i64(
            &db,
            r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = ?1"#,
            id,
        )
    };
    assert_eq!(owner(91).await, Some(1001));
    assert_eq!(owner(92).await, Some(1001));
    assert_eq!(owner(93).await, Some(1003));
}
