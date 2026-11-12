//! Port of `repositories/media-library-folder/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::Connection;

use super::models::MediaLibraryFolder;
use super::{Result, new_id, now_ms};

/// `listByWorkspace`: by name ascending.
pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> Result<Vec<MediaLibraryFolder>> {
    todo!("P3.3")
}

/// `create` (`id` = `None` makes a new snowflake).
pub fn create(
    conn: &Connection,
    id: Option<i64>,
    workspace_id: i64,
    name: &str,
) -> Result<MediaLibraryFolder> {
    todo!("P3.3")
}

/// `rename`.
pub fn rename(conn: &Connection, workspace_id: i64, folder_id: i64, name: &str) -> Result<()> {
    todo!("P3.3")
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    todo!("P3.3")
}
