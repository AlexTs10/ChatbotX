//! Port of `repositories/file/repository.ts` (`fileRepository`, P3.2b).
//!
//! `createdAtCursor` is the `toISOString()` text of `createdAt` (the column holds unix
//! ms, so the Postgres microseconds do not exist here).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::File;
use super::patch::{Patch, insert_clause, set_clause};
use super::sql::Sql;
use super::{Error, Model, Result, iso, new_id, now_ms, parse_iso};

/// `fileContextTypes.enum.export`.
const EXPORT: &str = "export";

/// `(createdAt, id) <op> (cursor)`; an unreadable cursor time is an error.
fn past_cursor(q: &mut Sql, cursor: Option<&ExportFileCursor>, op: &str) -> Result<()> {
    if let Some(cursor) = cursor {
        let at = parse_iso(&cursor.created_at)
            .ok_or_else(|| Error::msg(format!("invalid cursor createdAt {}", cursor.created_at)))?;
        q.push(&format!(" AND (f.\"createdAt\", f.\"id\") {op} ("));
        q.push_bind(at);
        q.push(", ");
        q.push_bind(cursor.id);
        q.push(")");
    }
    Ok(())
}

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
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    values: &Patch,
) -> Result<()> {
    let mut all: Patch = values.clone();
    if !all.iter().any(|(c, _)| *c == "updatedAt") {
        all.push(("updatedAt", Value::Integer(now_ms())));
    }
    let mut q = Sql::raw("UPDATE \"File\" SET ");
    q.append(set_clause(File::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.execute(conn)?;
    Ok(())
}

/// `findByIdForWorkspace` (`user_id` narrows to the uploader).
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    user_id: Option<i64>,
) -> Result<Option<File>> {
    let mut q = Sql::raw(format!("{} WHERE \"id\" = ", File::select_sql()));
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    if let Some(user_id) = user_id {
        q.push(" AND \"userId\" = ");
        q.push_bind(user_id);
    }
    q.push(" LIMIT 1");
    q.model(conn)
}

/// `create` (`id` / timestamps are filled when the patch has none).
pub fn create(conn: &Connection, values: &Patch) -> Result<File> {
    let now = now_ms();
    let mut all: Patch = Vec::new();
    for (col, v) in [
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
    ] {
        if !values.iter().any(|(c, _)| *c == col) {
            all.push((col, v));
        }
    }
    all.extend(values.iter().cloned());
    let mut q = Sql::raw("INSERT INTO \"File\" ");
    q.append(insert_clause(File::COLUMNS, &all)?);
    q.push(&format!(" RETURNING {}", File::columns_sql(None)));
    q.model(conn)?
        .ok_or_else(|| Error::msg("Failed to insert file"))
}

/// `listExportsForWorkspace`: newest first, `limit + 1` rows past `cursor`.
pub fn list_exports_for_workspace(
    conn: &Connection,
    input: &ListExportsForWorkspaceInput,
) -> Result<Vec<ExportFileListItem>> {
    let mut q = Sql::raw(
        "SELECT f.\"id\", f.\"status\", f.\"fileName\", f.\"meta\", f.\"createdAt\", f.\"userId\", u.\"name\" \
         FROM \"File\" AS f LEFT JOIN \"User\" AS u ON u.\"id\" = f.\"userId\" WHERE f.\"workspaceId\" = ",
    );
    q.push_bind(input.workspace_id);
    q.push(" AND f.\"contextType\" = ");
    q.push_bind(EXPORT.to_owned());
    q.push(" AND f.\"subType\" = ");
    q.push_bind(input.sub_type.clone());
    if let Some(user_id) = input.user_id {
        q.push(" AND f.\"userId\" = ");
        q.push_bind(user_id);
    }
    past_cursor(&mut q, input.cursor.as_ref(), "<")?;
    q.push(" ORDER BY f.\"createdAt\" DESC, f.\"id\" DESC LIMIT ");
    q.push_bind(input.limit + 1);
    q.query_map(conn, |r| {
        let created_at: i64 = r.get(4)?;
        Ok(ExportFileListItem {
            id: r.get(0)?,
            status: r.get(1)?,
            file_name: r.get(2)?,
            meta: r.get(3)?,
            created_at,
            user_id: r.get(5)?,
            requested_by_name: r.get(6)?,
            created_at_cursor: iso(created_at),
        })
    })
}

/// `listExportsCreatedBefore`: exports older than `before`, oldest first, past `after`.
pub fn list_exports_created_before(
    conn: &Connection,
    input: &ListExportsCreatedBeforeInput,
) -> Result<Vec<ExportFileSweepRow>> {
    let mut q = Sql::raw(
        "SELECT f.\"id\", f.\"workspaceId\", f.\"path\", f.\"createdAt\" FROM \"File\" AS f \
         WHERE f.\"contextType\" = ",
    );
    q.push_bind(EXPORT.to_owned());
    q.push(" AND f.\"subType\" = ");
    q.push_bind(input.sub_type.clone());
    q.push(" AND f.\"createdAt\" < ");
    q.push_bind(input.before);
    past_cursor(&mut q, input.after.as_ref(), ">")?;
    q.push(" ORDER BY f.\"createdAt\" ASC, f.\"id\" ASC LIMIT ");
    q.push_bind(input.limit);
    q.query_map(conn, |r| {
        Ok(ExportFileSweepRow {
            id: r.get(0)?,
            workspace_id: r.get(1)?,
            path: r.get(2)?,
            created_at_cursor: iso(r.get(3)?),
        })
    })
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, id: i64) -> Result<()> {
    Sql::bind("DELETE FROM \"File\" WHERE \"id\" = ", id).execute(conn)?;
    Ok(())
}
