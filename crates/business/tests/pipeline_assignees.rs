//! Port of `packages/business/__tests__/pipeline-assignees.test.ts` (pure).

use chatbotx_business::pipeline::assignees::{Assignees, group_assignees};
use chatbotx_db::repo::models::PipelineItemAssignee;

fn row(item_id: i64, member: Option<i64>, team: Option<i64>) -> PipelineItemAssignee {
    PipelineItemAssignee {
        id: 0,
        created_at: 0,
        updated_at: 0,
        workspace_id: 1,
        item_id,
        workspace_member_id: member,
        inbox_team_id: team,
    }
}

#[test]
fn group_assignees_buckets_members_and_teams_per_item_in_row_order() {
    let grouped = group_assignees(&[
        row(1, Some(11), None),
        row(1, None, Some(21)),
        row(2, Some(12), None),
    ]);
    assert_eq!(
        grouped.get(&1).cloned().unwrap_or_default(),
        Assignees {
            workspace_member_ids: vec![11],
            inbox_team_ids: vec![21],
        }
    );
    assert_eq!(
        grouped
            .get(&2)
            .cloned()
            .unwrap_or_default()
            .workspace_member_ids,
        vec![12]
    );
}

#[test]
fn group_assignees_returns_an_empty_bucket_for_an_item_without_assignees() {
    assert_eq!(
        group_assignees(&[]).get(&9).cloned().unwrap_or_default(),
        Assignees::default()
    );
}
