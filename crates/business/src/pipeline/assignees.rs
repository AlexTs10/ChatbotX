//! `pipeline/item-assignees.ts`.

use std::collections::HashMap;

use std::collections::HashSet;

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineItemAssignee;
use chatbotx_db::repo::partials::pipeline::PIPELINE_LIMITS;
use chatbotx_db::repo::{pipeline_item, pipeline_reference};
use chatbotx_db::rusqlite::Connection;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `PipelineItemAssignees`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignees {
    pub workspace_member_ids: Vec<i64>,
    pub inbox_team_ids: Vec<i64>,
}

/// `groupAssignees`: members and teams per item, in row order.
pub fn group_assignees(rows: &[PipelineItemAssignee]) -> HashMap<i64, Assignees> {
    let mut out: HashMap<i64, Assignees> = HashMap::new();
    for row in rows {
        let bucket = out.entry(row.item_id).or_default();
        if let Some(member) = row.workspace_member_id {
            bucket.workspace_member_ids.push(member);
        }
        if let Some(team) = row.inbox_team_id {
            bucket.inbox_team_ids.push(team);
        }
    }
    out
}

/// `primaryFromSets`: `u_<userId>` of the first member that resolves, else
/// `t_<teamId>` of the first team.
pub async fn primary_from_sets(
    db: &Db,
    workspace_id: i64,
    sets: &Assignees,
) -> Result<Option<String>> {
    let sets = sets.clone();
    tx::read(db, move |conn| primary_in(conn, workspace_id, &sets)).await
}

/// [`primary_from_sets`] on an open connection.
pub fn primary_in(
    conn: &Connection,
    workspace_id: i64,
    sets: &Assignees,
) -> Result<Option<String>> {
    if let Some(&first) = sets.workspace_member_ids.first()
        && let Some(member) =
            pipeline_reference::list_workspace_members(conn, workspace_id, &[first])?.first()
    {
        return Ok(Some(format!("u_{}", member.user_id)));
    }
    Ok(sets.inbox_team_ids.first().map(|team| format!("t_{team}")))
}

fn unique(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// `assigneesFromInput`: de-duplicated, at most `assigneesPerItem` in all.
pub fn assignees_from_input(members: Option<&[i64]>, teams: Option<&[i64]>) -> Result<Assignees> {
    let out = Assignees {
        workspace_member_ids: unique(members.unwrap_or_default()),
        inbox_team_ids: unique(teams.unwrap_or_default()),
    };
    let max = PIPELINE_LIMITS.assignees_per_item;
    if out.workspace_member_ids.len() + out.inbox_team_ids.len() > max {
        return Err(BusinessError::validation(
            "assigneeMemberIds",
            format!("At most {max} assignees"),
        ));
    }
    Ok(out)
}

/// `assertAssigneesExist`.
pub fn assert_assignees_exist(
    conn: &Connection,
    workspace_id: i64,
    sets: &Assignees,
) -> Result<()> {
    let members =
        pipeline_reference::list_workspace_members(conn, workspace_id, &sets.workspace_member_ids)?;
    if members.len() != sets.workspace_member_ids.len() {
        return Err(BusinessError::not_found("Assigned admin not found"));
    }
    let teams = pipeline_reference::list_inbox_team_ids(conn, workspace_id, &sets.inbox_team_ids)?;
    if teams.len() != sets.inbox_team_ids.len() {
        return Err(BusinessError::not_found("Assigned team not found"));
    }
    Ok(())
}

/// `currentAssignees`.
pub fn current_in(conn: &Connection, workspace_id: i64, item_id: i64) -> Result<Assignees> {
    let rows = pipeline_item::list_assignees_for_items(conn, workspace_id, &[item_id])?;
    Ok(group_assignees(&rows).remove(&item_id).unwrap_or_default())
}

fn same_set(a: &[i64], b: &[i64]) -> bool {
    a.len() == b.len() && a.iter().collect::<HashSet<_>>() == b.iter().collect::<HashSet<_>>()
}

/// `applyAssignees`: replaces the sets when they differ; `true` = changed.
pub fn apply_in(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    members: Option<&[i64]>,
    teams: Option<&[i64]>,
    before: &Assignees,
) -> Result<bool> {
    if members.is_none() && teams.is_none() {
        return Ok(false);
    }
    let next = assignees_from_input(
        Some(members.unwrap_or(&before.workspace_member_ids)),
        Some(teams.unwrap_or(&before.inbox_team_ids)),
    )?;
    if same_set(&next.workspace_member_ids, &before.workspace_member_ids)
        && same_set(&next.inbox_team_ids, &before.inbox_team_ids)
    {
        return Ok(false);
    }
    pipeline_item::replace_assignees(
        conn,
        workspace_id,
        item_id,
        &next.workspace_member_ids,
        &next.inbox_team_ids,
    )?;
    Ok(true)
}
