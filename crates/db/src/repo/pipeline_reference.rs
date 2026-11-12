//! Port of `repositories/pipeline/reference.ts` (`pipelineReferenceRepository`):
//! workspace-scoped existence lookups for ids a pipeline mutation references. The
//! `FOR KEY SHARE` lock of `lockUserIfExists` is a plain read under the one SQLite writer.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Result, id_list};

/// One row of `listWorkspaceMembers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberRef {
    pub id: i64,
    pub user_id: i64,
}

/// One row of `listContactSummaries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactSummary {
    pub id: i64,
    pub full_name: Option<String>,
    pub avatar: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
}

fn ids_in(conn: &Connection, table: &str, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        format!(
            "SELECT \"id\" FROM \"{table}\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?))"
        ),
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |row| row.get(0))
}

/// `lockUserIfExists`: whether the user still exists.
pub fn lock_user_if_exists(conn: &Connection, user_id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `listWorkspaceMembers`.
pub fn list_workspace_members(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<MemberRef>> {
    todo!("P3.3")
}

/// `listInboxTeamIds`.
pub fn list_inbox_team_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `listMediaLibraryFileIds`.
pub fn list_media_library_file_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `listContactSummaries`: display fields of the contacts on a page of cards.
pub fn list_contact_summaries(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<ContactSummary>> {
    todo!("P3.3")
}

/// `contactExists`.
pub fn contact_exists(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}
