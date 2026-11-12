//! Port of `repositories/media-library-file/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::Connection;

use super::models::MediaLibraryFile;
use super::sql::Sql;
use super::{Model, Result, id_list, new_id, now_ms};

/// `MediaLibraryFileFolderCount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCount {
    pub folder_id: Option<i64>,
    pub count: i64,
}

/// Input of [`list`]. `filter`: `favourite` | `recent` | `all` | `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilesInput {
    pub workspace_id: i64,
    pub filter: Option<String>,
    pub folder_id: Option<i64>,
    pub search: Option<String>,
    /// `Some("image")` keeps `image/*` files only.
    pub file_type: Option<String>,
    pub page: Option<i64>,
    pub per_page: i64,
}

/// Result of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct FilePage {
    pub data: Vec<MediaLibraryFile>,
    pub total: i64,
}

/// `$inferInsert` of `MediaLibraryFile` (`id` = `None` makes a new snowflake).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewMediaLibraryFile {
    pub id: Option<i64>,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub size: i64,
    pub is_favourite: bool,
    pub workspace_id: i64,
    pub folder_id: Option<i64>,
}

/// `findByPath`.
pub fn find_by_path(
    conn: &Connection,
    workspace_id: i64,
    path: &str,
) -> Result<Option<MediaLibraryFile>> {
    todo!("P3.3")
}

/// `findById`.
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<MediaLibraryFile>> {
    todo!("P3.3")
}

/// `findByIds` (no statement for an empty list).
pub fn find_by_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<MediaLibraryFile>> {
    todo!("P3.3")
}

fn list_where(input: &ListFilesInput) -> Sql {
    let filter = input.filter.as_deref().filter(|f| !f.is_empty());
    let folder = if filter == Some("favourite") {
        Some(Sql::raw("\"isFavourite\" = 1"))
    } else if let Some(folder_id) = input.folder_id {
        Some(Sql::bind("\"folderId\" = ", folder_id))
    } else if filter.is_none() {
        Some(Sql::raw("\"folderId\" IS NULL"))
    } else {
        None
    };
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        folder,
        input
            .search
            .as_deref()
            .filter(|s| !s.is_empty())
            // `%${search}%` without escaping, as the TS.
            .map(|s| Sql::bind("ilike(\"name\", ", format!("%{s}%")).closed(")")),
        (input.file_type.as_deref() == Some("image"))
            .then(|| Sql::raw("ilike(\"mimeType\", 'image/%')")),
    ])
}

/// `list`: one page and the total of the same filter. `recent` sorts by
/// `lastAccessedAt` (newest first; NULLs first, as Postgres `DESC`), the rest by `createdAt`.
pub fn list(conn: &Connection, input: &ListFilesInput) -> Result<FilePage> {
    todo!("P3.3")
}

/// `countByFolder`.
pub fn count_by_folder(conn: &Connection, workspace_id: i64) -> Result<Vec<FolderCount>> {
    todo!("P3.3")
}

/// `listByFolder`: `(id, path)` of the folder's files.
pub fn list_by_folder(
    conn: &Connection,
    workspace_id: i64,
    folder_id: i64,
) -> Result<Vec<(i64, String)>> {
    todo!("P3.3")
}

/// `create`.
pub fn create(conn: &Connection, values: &NewMediaLibraryFile) -> Result<MediaLibraryFile> {
    todo!("P3.3")
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `deleteByFolder`.
pub fn delete_by_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `moveToFolder` (`None` = root).
pub fn move_to_folder(
    conn: &Connection,
    workspace_id: i64,
    file_ids: &[i64],
    folder_id: Option<i64>,
) -> Result<()> {
    todo!("P3.3")
}

/// `setFavourite`.
pub fn set_favourite(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    is_favourite: bool,
) -> Result<()> {
    todo!("P3.3")
}

/// `touchLastAccessedAt` (wall clock, as `CURRENT_TIMESTAMP`).
pub fn touch_last_accessed_at(conn: &Connection, workspace_id: i64, file_id: i64) -> Result<()> {
    todo!("P3.3")
}
