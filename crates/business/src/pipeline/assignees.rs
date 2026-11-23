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

/// The `assignContact` effect (`assignContactOnAssign`): the item's new
/// primary assignee, for the contact's conversations after the commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactAssignment {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub item_id: i64,
    /// `u_<userId>` or `t_<inboxTeamId>`.
    pub assigned_id: String,
    pub previous_assigned_id: Option<String>,
}

/// `queueAssignmentEffect`: `None` when the setting is off, the item has no
/// primary or the primary did not change. It never unassigns.
pub fn queue_assignment_in(
    conn: &Connection,
    enabled: bool,
    item: &chatbotx_db::repo::models::PipelineItem,
    before: Option<&Assignees>,
) -> Result<Option<ContactAssignment>> {
    if !enabled {
        return Ok(None);
    }
    let ws = item.workspace_id;
    let Some(after) = primary_in(conn, ws, &current_in(conn, ws, item.id)?)? else {
        return Ok(None);
    };
    let before = match before {
        Some(sets) => primary_in(conn, ws, sets)?,
        None => None,
    };
    if before.as_deref() == Some(after.as_str()) {
        return Ok(None);
    }
    Ok(Some(ContactAssignment {
        workspace_id: ws,
        contact_id: item.contact_id,
        item_id: item.id,
        assigned_id: after,
        previous_assigned_id: before,
    }))
}

/// The `assignContact` effect runner: the contact's conversations that are
/// unassigned or still with either side of the transition move to the
/// item's *current* primary (so late or out-of-order effects end on the
/// last one); a conversation someone else owns keeps its assignee. The
/// caller publishes each `Assigned`.
pub async fn apply_contact_assignments(
    db: &Db,
    assignments: &[ContactAssignment],
) -> Result<Vec<(i64, crate::conversation_actions::Assigned)>> {
    let mut out = Vec::new();
    for effect in assignments {
        let effect = effect.clone();
        let ws = effect.workspace_id;
        let targets = tx::read(db, move |conn| {
            let Some(current) = primary_in(conn, ws, &current_in(conn, ws, effect.item_id)?)?
            else {
                return Ok(None);
            };
            let from: Vec<Option<String>> = [
                None,
                effect.previous_assigned_id.clone(),
                Some(effect.assigned_id.clone()),
            ]
            .into_iter()
            .filter(|a| a.as_deref() != Some(current.as_str()))
            .collect();
            let mut stmt = conn.prepare(
                r#"SELECT "id", "assignedUserId", "assignedInboxTeamId" FROM "Conversation"
                   WHERE "workspaceId" = ?1 AND "contactId" = ?2"#,
            )?;
            let rows = stmt.query_map([ws, effect.contact_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?;
            let mut ids = Vec::new();
            for row in rows {
                let (id, user, team) = row?;
                let assignee = user
                    .map(|u| format!("u_{u}"))
                    .or_else(|| team.map(|t| format!("t_{t}")));
                if from.contains(&assignee) {
                    ids.push(id);
                }
            }
            Ok(Some((current, ids)))
        })
        .await?;
        let Some((current, ids)) = targets else {
            continue;
        };
        for id in ids {
            let assigned =
                crate::conversation_actions::assign_one(db, ws, id, Some(current.clone())).await?;
            out.push((ws, assigned));
        }
    }
    Ok(out)
}
