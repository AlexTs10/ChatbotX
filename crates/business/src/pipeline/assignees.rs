//! `pipeline/item-assignees.ts`.

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineItemAssignee;

use crate::errors::Result;

/// `PipelineItemAssignees`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignees {
    pub workspace_member_ids: Vec<i64>,
    pub inbox_team_ids: Vec<i64>,
}

/// `groupAssignees`: members and teams per item, in row order.
pub fn group_assignees(rows: &[PipelineItemAssignee]) -> HashMap<i64, Assignees> {
    let _ = rows;
    todo!("P3.6-D1p")
}

/// `primaryFromSets`: `u_<userId>` of the first member that resolves, else
/// `t_<teamId>` of the first team.
pub async fn primary_from_sets(
    db: &Db,
    workspace_id: i64,
    sets: &Assignees,
) -> Result<Option<String>> {
    let _ = (db, workspace_id, sets);
    todo!("P3.6-D1p")
}
