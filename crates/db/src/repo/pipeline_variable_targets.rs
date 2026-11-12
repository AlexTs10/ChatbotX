//! Port of `repositories/pipeline/variable-targets.ts`
//! (`pipelineVariableTargetRepository`). `DISTINCT ON (pipelineId)` is a
//! `row_number() OVER (PARTITION BY ...)` window; `array_agg` is `json_group_array` over an
//! ordered subquery (`None` when the item has no assignee, like Postgres `NULL`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Json, Result};

/// One row of `listForContact`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineVariableTarget {
    pub item_id: i64,
    pub title: String,
    pub value: Option<i64>,
    pub status: String,
    pub priority: String,
    pub due_date: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub pipeline_id: i64,
    pub reference_name: String,
    pub currency: Option<String>,
    pub settings: Json,
    pub stage_name: String,
    pub target_country: Option<String>,
    /// Display names (member name, else team name), oldest assignee first.
    pub assignees: Option<Vec<Option<String>>>,
}

/// `listForContact`: one row per pipeline the contact has items in (its most recently
/// created open item, else its most recently created item), in pipeline id order.
pub fn list_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<PipelineVariableTarget>> {
    let sql = "WITH ranked AS (
        SELECT i.\"id\" AS item_id, i.\"title\", i.\"value\", i.\"status\", i.\"priority\", i.\"dueDate\",
               i.\"createdAt\", i.\"updatedAt\", i.\"pipelineId\", p.\"referenceName\", p.\"currency\",
               p.\"settings\", s.\"name\" AS stage_name, w.\"targetCountry\",
               row_number() OVER (PARTITION BY i.\"pipelineId\"
                   ORDER BY (i.\"status\" = 'open') DESC, i.\"createdAt\" DESC, i.\"id\" DESC) AS rn
        FROM \"PipelineItem\" i
        JOIN \"Pipeline\" p ON p.\"id\" = i.\"pipelineId\" AND p.\"workspaceId\" = i.\"workspaceId\"
        JOIN \"PipelineStage\" s ON s.\"id\" = i.\"stageId\"
        JOIN \"Workspace\" w ON w.\"id\" = i.\"workspaceId\"
        WHERE i.\"workspaceId\" = ? AND i.\"contactId\" = ?
    )
    SELECT item_id, \"title\", \"value\", \"status\", \"priority\", \"dueDate\", \"createdAt\", \"updatedAt\",
           \"pipelineId\", \"referenceName\", \"currency\", \"settings\", stage_name, \"targetCountry\",
           (SELECT json_group_array(n) FROM (
                SELECT coalesce(u.\"name\", t.\"name\") AS n
                FROM \"PipelineItemAssignee\" a
                LEFT JOIN \"WorkspaceMember\" m ON m.\"id\" = a.\"workspaceMemberId\"
                LEFT JOIN \"User\" u ON u.\"id\" = m.\"userId\"
                LEFT JOIN \"InboxTeam\" t ON t.\"id\" = a.\"inboxTeamId\"
                WHERE a.\"itemId\" = ranked.item_id AND a.\"workspaceId\" = ?
                ORDER BY a.\"createdAt\", a.\"id\")) AS assignees
    FROM ranked WHERE rn = 1 ORDER BY \"pipelineId\"";
    Sql::new(
        sql,
        vec![
            Value::Integer(workspace_id),
            Value::Integer(contact_id),
            Value::Integer(workspace_id),
        ],
    )
    .query_map(conn, |row| {
        let names: String = row.get(14)?;
        let parsed: Vec<Option<String>> = serde_json::from_str(&names).unwrap_or_default();
        Ok(PipelineVariableTarget {
            item_id: row.get(0)?,
            title: row.get(1)?,
            value: row.get(2)?,
            status: row.get(3)?,
            priority: row.get(4)?,
            due_date: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            pipeline_id: row.get(8)?,
            reference_name: row.get(9)?,
            currency: row.get(10)?,
            settings: row.get(11)?,
            stage_name: row.get(12)?,
            target_country: row.get(13)?,
            assignees: (!parsed.is_empty()).then_some(parsed),
        })
    })
}

/// `findLastTouchedItemId`: the contact's most recently created or updated item in any
/// pipeline.
pub fn find_last_touched_item_id(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<i64>> {
    Ok(Sql::new(
        "SELECT \"id\" FROM \"PipelineItem\" WHERE \"workspaceId\" = ? AND \"contactId\" = ? ORDER BY \"updatedAt\" DESC, \"id\" DESC LIMIT 1",
        vec![Value::Integer(workspace_id), Value::Integer(contact_id)],
    )
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .into_iter()
    .next())
}
