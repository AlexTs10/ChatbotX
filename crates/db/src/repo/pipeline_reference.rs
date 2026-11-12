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
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"User\" WHERE \"id\" = ?1)",
        [user_id],
        |row| row.get(0),
    )?)
}

/// `listWorkspaceMembers`.
pub fn list_workspace_members(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<MemberRef>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"userId\" FROM \"WorkspaceMember\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?))",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |row| {
        Ok(MemberRef {
            id: row.get(0)?,
            user_id: row.get(1)?,
        })
    })
}

/// `listInboxTeamIds`.
pub fn list_inbox_team_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    ids_in(conn, "InboxTeam", workspace_id, ids)
}

/// `listMediaLibraryFileIds`.
pub fn list_media_library_file_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    ids_in(conn, "MediaLibraryFile", workspace_id, ids)
}

/// `listContactSummaries`: display fields of the contacts on a page of cards.
pub fn list_contact_summaries(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<ContactSummary>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"fullName\", \"avatar\", \"email\", \"phoneNumber\" FROM \"Contact\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?))",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |row| {
        Ok(ContactSummary {
            id: row.get(0)?,
            full_name: row.get(1)?,
            avatar: row.get(2)?,
            email: row.get(3)?,
            phone_number: row.get(4)?,
        })
    })
}

/// `contactExists`.
pub fn contact_exists(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Contact\" WHERE \"workspaceId\" = ?1 AND \"id\" = ?2)",
        (workspace_id, id),
        |row| row.get(0),
    )?)
}
