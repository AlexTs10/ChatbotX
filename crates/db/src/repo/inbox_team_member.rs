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
    todo!("P3.2b")
}

/// `listUserIdsByTeamIds`: one query for all teams (empty list = no query).
pub fn list_user_ids_by_team_ids(
    conn: &Connection,
    workspace_id: i64,
    inbox_team_ids: &[i64],
) -> Result<Vec<i64>> {
    todo!("P3.2b")
}

/// `listTeamNamesByUserId`: by team name.
pub fn list_team_names_by_user_id(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Vec<String>> {
    todo!("P3.2b")
}

/// `listTeamIdsByUserId`.
pub fn list_team_ids_by_user_id(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Vec<i64>> {
    todo!("P3.2b")
}
