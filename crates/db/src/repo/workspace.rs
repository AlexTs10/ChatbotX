//! Port of `repositories/workspace/repository.ts` (P3.3).
//!
//! Row locks (`FOR KEY SHARE`, `FOR NO KEY UPDATE`) and the advisory lock of
//! `lockFirstWorkspace` are not needed: the one SQLite writer serializes all write jobs.
//! The functions stay so callers port 1:1.

use rusqlite::{Connection, OptionalExtension};

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
    let mut stmt =
        conn.prepare_cached("SELECT \"purgeStartedAt\" FROM \"Workspace\" WHERE \"id\" = ?1")?;
    let purge_started_at: Option<Option<i64>> = stmt
        .query_row([workspace_id], |row| row.get(0))
        .optional()?;
    Ok(matches!(purge_started_at, Some(None)))
}

/// `lockFirstWorkspace`: a no-op under the single writer (see the module doc).
pub fn lock_first_workspace(conn: &Connection, owner_id: i64) -> Result<()> {
    let _ = (conn, owner_id);
    Ok(())
}

/// `findByIdForUpdate`: the deletion-request state of one workspace.
pub fn find_by_id_for_update(
    conn: &Connection,
    id: i64,
) -> Result<Option<WorkspaceDeletionRequestState>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\", \"scheduledDeletionAt\", \"deletionRequestedAt\", \
         \"deletionRequestedById\", \"purgeStartedAt\" FROM \"Workspace\" WHERE \"id\" = ?1",
    )?;
    Ok(stmt
        .query_row([id], |row| {
            Ok(WorkspaceDeletionRequestState {
                id: row.get(0)?,
                name: row.get(1)?,
                scheduled_deletion_at: row.get(2)?,
                deletion_requested_at: row.get(3)?,
                deletion_requested_by_id: row.get(4)?,
                purge_started_at: row.get(5)?,
            })
        })
        .optional()?)
}

/// `findActiveByOwner`: the owner's oldest live (not scheduled for deletion) workspace.
pub fn find_active_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<Workspace>> {
    Ok(super::find_many::<Workspace>(
        conn,
        "WHERE \"ownerId\" = ?1 AND \"scheduledDeletionAt\" IS NULL ORDER BY \"id\" ASC LIMIT 1",
        &[&owner_id],
    )?
    .into_iter()
    .next())
}

/// `findEmptyActiveByOwner`: the owner's oldest live workspace that has no Inbox.
pub fn find_empty_active_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<Workspace>> {
    Ok(super::find_many::<Workspace>(
        conn,
        "WHERE \"ownerId\" = ?1 AND \"scheduledDeletionAt\" IS NULL \
         AND NOT EXISTS (SELECT 1 FROM \"Inbox\" WHERE \"Inbox\".\"workspaceId\" = \"Workspace\".\"id\") \
         ORDER BY \"id\" ASC LIMIT 1",
        &[&owner_id],
    )?
    .into_iter()
    .next())
}
