//! Port of `repositories/workspace/repository.ts` (P3.3).
//!
//! Row locks (`FOR KEY SHARE`, `FOR NO KEY UPDATE`) and the advisory lock of
//! `lockFirstWorkspace` are not needed: the one SQLite writer serializes all write jobs.
//! The functions stay so callers port 1:1.

use rusqlite::Connection;

use super::Result;
use super::models::Workspace;

/// `WorkspaceDeletionRequestState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDeletionRequestState {
    pub id: i64,
    pub name: String,
    pub scheduled_deletion_at: Option<i64>,
    pub deletion_requested_at: Option<i64>,
    pub deletion_requested_by_id: Option<i64>,
    pub purge_started_at: Option<i64>,
}

/// `lockWorkspaceForChildWrite`: `true` while the workspace exists and no purge started.
pub fn lock_workspace_for_child_write(conn: &Connection, workspace_id: i64) -> Result<bool> {
    let _ = (conn, workspace_id);
    todo!("P3.3")
}

/// `lockFirstWorkspace`: a no-op under the single writer (see the module doc).
pub fn lock_first_workspace(conn: &Connection, owner_id: i64) -> Result<()> {
    let _ = (conn, owner_id);
    todo!("P3.3")
}

/// `findByIdForUpdate`: the deletion-request state of one workspace.
pub fn find_by_id_for_update(
    conn: &Connection,
    id: i64,
) -> Result<Option<WorkspaceDeletionRequestState>> {
    let _ = (conn, id);
    todo!("P3.3")
}

/// `findActiveByOwner`: the owner's oldest live (not scheduled for deletion) workspace.
pub fn find_active_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<Workspace>> {
    let _ = (conn, owner_id);
    todo!("P3.3")
}

/// `findEmptyActiveByOwner`: the owner's oldest live workspace that has no Inbox.
pub fn find_empty_active_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<Workspace>> {
    let _ = (conn, owner_id);
    todo!("P3.3")
}
