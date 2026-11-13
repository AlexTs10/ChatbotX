//! Port of `repositories/inbox-team-member/repository.ts` (`inboxTeamMemberRepository`).
//! Every query joins `InboxTeam` and filters on its `workspaceId` (workspace fence).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Result, id_list};

const JOIN: &str = "FROM \"InboxTeamMember\" m JOIN \"InboxTeam\" t ON m.\"inboxTeamId\" = t.\"id\" WHERE t.\"workspaceId\" = ?";

/// `listUserIdsByTeamId`.
pub fn list_user_ids_by_team_id(
    conn: &Connection,
    workspace_id: i64,
    inbox_team_id: i64,
) -> Result<Vec<i64>> {
    Sql::new(
        format!("SELECT m.\"userId\" {JOIN} AND m.\"inboxTeamId\" = ?"),
        vec![Value::Integer(workspace_id), Value::Integer(inbox_team_id)],
    )
    .query_map(conn, |row| row.get(0))
}

/// `listUserIdsByTeamIds`: one query for all teams (empty list = no query).
pub fn list_user_ids_by_team_ids(
    conn: &Connection,
    workspace_id: i64,
    inbox_team_ids: &[i64],
) -> Result<Vec<i64>> {
    if inbox_team_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        format!(
            "SELECT m.\"userId\" {JOIN} AND m.\"inboxTeamId\" IN (SELECT value FROM json_each(?))"
        ),
        vec![
            Value::Integer(workspace_id),
            Value::Text(id_list(inbox_team_ids)),
        ],
    )
    .query_map(conn, |row| row.get(0))
}

/// `listTeamNamesByUserId`: by team name.
pub fn list_team_names_by_user_id(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Vec<String>> {
    Sql::new(
        format!("SELECT t.\"name\" {JOIN} AND m.\"userId\" = ? ORDER BY t.\"name\" ASC"),
        vec![Value::Integer(workspace_id), Value::Integer(user_id)],
    )
    .query_map(conn, |row| row.get(0))
}

/// `listTeamIdsByUserId`.
pub fn list_team_ids_by_user_id(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Vec<i64>> {
    Sql::new(
        format!("SELECT m.\"inboxTeamId\" {JOIN} AND m.\"userId\" = ?"),
        vec![Value::Integer(workspace_id), Value::Integer(user_id)],
    )
    .query_map(conn, |row| row.get(0))
}
