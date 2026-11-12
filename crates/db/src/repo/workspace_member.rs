//! Port of `repositories/workspace-member/repository.ts` (P3.3).

use rusqlite::Connection;

use super::{Json, Result, id_list, now_ms};

/// `WorkspaceMemberPermissionsRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMemberPermissionsRow {
    pub user_id: i64,
    pub permissions: Json,
}

/// `listPermissionsByUserIds`: permissions of exactly the requested users of the
/// workspace; an id without a row is absent. Empty `user_ids` reads nothing.
pub fn list_permissions_by_user_ids(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
) -> Result<Vec<WorkspaceMemberPermissionsRow>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"userId\", \"permissions\" FROM \"WorkspaceMember\" \
         WHERE \"workspaceId\" = ?1 AND \"userId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(user_ids)], |row| {
            Ok(WorkspaceMemberPermissionsRow {
                user_id: row.get(0)?,
                permissions: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `markOnlineBulk`: one UPDATE that sets `onlineSince = now` for the users of the
/// workspace. Empty `user_ids` writes nothing; an id without a row is skipped.
pub fn mark_online_bulk(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
    now: i64,
) -> Result<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE \"WorkspaceMember\" SET \"onlineSince\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"workspaceId\" = ?1 AND \"userId\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, id_list(user_ids), now, now_ms()],
    )?;
    Ok(())
}
