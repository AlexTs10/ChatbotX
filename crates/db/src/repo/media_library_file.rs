//! Port of `repositories/media-library-file/repository.ts` (P3.3).

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
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"path\" = ?2",
        &[&workspace_id, &path],
    )
}

/// `findById`.
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<MediaLibraryFile>> {
    super::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )
}

/// `findByIds` (no statement for an empty list).
pub fn find_by_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<MediaLibraryFile>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2))",
        &[&workspace_id, &id_list(ids)],
    )
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
    let order = if input.filter.as_deref() == Some("recent") {
        "\"lastAccessedAt\" IS NOT NULL, \"lastAccessedAt\" DESC"
    } else {
        "\"createdAt\" DESC"
    };
    let page = input.page.unwrap_or(1);
    let mut q = Sql::raw(format!("{} WHERE ", MediaLibraryFile::select_sql()));
    q.append(list_where(input));
    q.push(&format!(" ORDER BY {order} LIMIT "));
    q.push_bind(input.per_page);
    q.push(" OFFSET ");
    q.push_bind((page - 1) * input.per_page);
    let data = q.models::<MediaLibraryFile>(conn)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"MediaLibraryFile\" WHERE ");
    c.append(list_where(input));
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok(FilePage { data, total })
}

/// `countByFolder`.
pub fn count_by_folder(conn: &Connection, workspace_id: i64) -> Result<Vec<FolderCount>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"folderId\", count(*) FROM \"MediaLibraryFile\" WHERE \"workspaceId\" = ?1 GROUP BY \"folderId\"",
    )?;
    let rows = stmt
        .query_map([workspace_id], |row| {
            Ok(FolderCount {
                folder_id: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listByFolder`: `(id, path)` of the folder's files.
pub fn list_by_folder(
    conn: &Connection,
    workspace_id: i64,
    folder_id: i64,
) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"path\" FROM \"MediaLibraryFile\" WHERE \"folderId\" = ?1 AND \"workspaceId\" = ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![folder_id, workspace_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `create`.
pub fn create(conn: &Connection, values: &NewMediaLibraryFile) -> Result<MediaLibraryFile> {
    let now = now_ms();
    super::returning::<MediaLibraryFile>(
        conn,
        "INSERT INTO \"MediaLibraryFile\" (\"id\", \"createdAt\", \"updatedAt\", \"name\", \"path\", \"mimeType\", \
         \"size\", \"isFavourite\", \"workspaceId\", \"folderId\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        &[
            &values.id.unwrap_or_else(new_id),
            &now,
            &values.name,
            &values.path,
            &values.mime_type,
            &values.size,
            &values.is_favourite,
            &values.workspace_id,
            &values.folder_id,
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| super::Error::msg("Failed to create media library file"))
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, workspace_id: i64, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MediaLibraryFile\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id],
    )?;
    Ok(())
}

/// `deleteByFolder`.
pub fn delete_by_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"MediaLibraryFile\" WHERE \"folderId\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![folder_id, workspace_id],
    )?;
    Ok(())
}

/// `moveToFolder` (`None` = root).
pub fn move_to_folder(
    conn: &Connection,
    workspace_id: i64,
    file_ids: &[i64],
    folder_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE \"MediaLibraryFile\" SET \"folderId\" = ?3, \"updatedAt\" = ?4 \
         WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![workspace_id, id_list(file_ids), folder_id, now_ms()],
    )?;
    Ok(())
}

/// `setFavourite`.
pub fn set_favourite(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    is_favourite: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE \"MediaLibraryFile\" SET \"isFavourite\" = ?3, \"updatedAt\" = ?4 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![id, workspace_id, is_favourite, now_ms()],
    )?;
    Ok(())
}

/// `touchLastAccessedAt` (wall clock, as `CURRENT_TIMESTAMP`).
pub fn touch_last_accessed_at(conn: &Connection, workspace_id: i64, file_id: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE \"MediaLibraryFile\" SET \"lastAccessedAt\" = ?3, \"updatedAt\" = ?3 WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![file_id, workspace_id, now],
    )?;
    Ok(())
}
