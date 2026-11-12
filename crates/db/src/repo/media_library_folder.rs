//! Port of `repositories/media-library-folder/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::MediaLibraryFolder;
use super::{Result, new_id, now_ms};

/// `listByWorkspace`: by name ascending.
pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<MediaLibraryFolder>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"name\" ASC",
        &[&workspace_id],
    )
}

/// `create` (`id` = `None` makes a new snowflake).
pub fn create(
    conn: &Connection,
    id: Option<i64>,
    workspace_id: i64,
    name: &str,
) -> Result<MediaLibraryFolder> {
    let now = now_ms();
    super::returning::<MediaLibraryFolder>(
        conn,
        "INSERT INTO \"MediaLibraryFolder\" (\"id\", \"createdAt\", \"updatedAt\", \"name\", \"workspaceId\") \
         VALUES (?1, ?2, ?2, ?3, ?4)",
        &[&id.unwrap_or_else(new_id), &now, &name, &workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to create media library folder"))
}

/// `rename`.
pub fn rename(conn: &Connection, workspace_id: i64, folder_id: i64, name: &str) -> Result<()> {
    conn.execute(
        "UPDATE \"MediaLibraryFolder\" SET \"name\" = ?3, \"updatedAt\" = ?4 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![folder_id, workspace_id, name, now_ms()],
    )?;
    Ok(())
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MediaLibraryFolder\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![folder_id, workspace_id],
    )?;
    Ok(())
}
