//! Port of `repositories/file/repository.ts` (`fileRepository`, P3.2b).
//!
//! `createdAtCursor` is the `toISOString()` text of `createdAt` (the column holds unix
//! ms, so the Postgres microseconds do not exist here).

use rusqlite::Connection;

use super::Result;
use super::models::File;
use super::patch::Patch;

/// `ExportFileCursor`: a keyset position in `(createdAt, id)` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFileCursor {
    pub created_at: String,
    pub id: i64,
}

/// `ExportFileListItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportFileListItem {
    pub id: i64,
    pub status: String,
    pub file_name: String,
    pub meta: Option<super::Json>,
    pub created_at: i64,
    pub user_id: Option<i64>,
    pub requested_by_name: Option<String>,
    pub created_at_cursor: String,
}

/// One row of `listExportsCreatedBefore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFileSweepRow {
    pub id: i64,
    pub workspace_id: Option<i64>,
    pub path: String,
    pub created_at_cursor: String,
}

/// Input of `listExportsForWorkspace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListExportsForWorkspaceInput {
    pub workspace_id: i64,
    pub sub_type: String,
    pub user_id: Option<i64>,
    pub limit: i64,
    pub cursor: Option<ExportFileCursor>,
}

/// Input of `listExportsCreatedBefore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListExportsCreatedBeforeInput {
    pub sub_type: String,
    pub before: i64,
    pub limit: i64,
    pub after: Option<ExportFileCursor>,
}

/// `updateForWorkspace`: sets `values` (and `updatedAt`) on the row `(id, workspaceId)`.
pub fn update_for_workspace(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _values: &Patch,
) -> Result<()> {
    todo!("P3.2b")
}

/// `findByIdForWorkspace` (`user_id` narrows to the uploader).
pub fn find_by_id_for_workspace(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _user_id: Option<i64>,
) -> Result<Option<File>> {
    todo!("P3.2b")
}

/// `create` (`id` / timestamps are filled when the patch has none).
pub fn create(_conn: &Connection, _values: &Patch) -> Result<File> {
    todo!("P3.2b")
}

/// `listExportsForWorkspace`: newest first, `limit + 1` rows past `cursor`.
pub fn list_exports_for_workspace(
    _conn: &Connection,
    _input: &ListExportsForWorkspaceInput,
) -> Result<Vec<ExportFileListItem>> {
    todo!("P3.2b")
}

/// `listExportsCreatedBefore`: exports older than `before`, oldest first, past `after`.
pub fn list_exports_created_before(
    _conn: &Connection,
    _input: &ListExportsCreatedBeforeInput,
) -> Result<Vec<ExportFileSweepRow>> {
    todo!("P3.2b")
}

/// `deleteById`.
pub fn delete_by_id(_conn: &Connection, _id: i64) -> Result<()> {
    todo!("P3.2b")
}
