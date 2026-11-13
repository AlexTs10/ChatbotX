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
