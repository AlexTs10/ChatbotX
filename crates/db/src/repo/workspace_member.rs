//! Port of `repositories/workspace-member/repository.ts` (P3.3).

use rusqlite::Connection;

use super::{Json, Result};

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
    let _ = (conn, workspace_id, user_ids);
    todo!("P3.3")
}

/// `markOnlineBulk`: one UPDATE that sets `onlineSince = now` for the users of the
/// workspace. Empty `user_ids` writes nothing; an id without a row is skipped.
pub fn mark_online_bulk(
    conn: &Connection,
    workspace_id: i64,
    user_ids: &[i64],
    now: i64,
) -> Result<()> {
    let _ = (conn, workspace_id, user_ids, now);
    todo!("P3.3")
}
