//! Port of `repositories/template-version/repository.ts` (P3.3). The advisory locks
//! (`lockVersions`, the shared lock of `pinVersionNumber`) are not needed under the one
//! SQLite writer.
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::{Connection, OptionalExtension};

use super::models::TemplateVersion;
use super::{Json, Result, now_ms};

/// Versions kept regardless of use.
pub const RETAINED_VERSIONS: i64 = 20;
/// Versions a history lists.
pub const VERSION_HISTORY_LIMIT: i64 = 50;

/// Input of [`insert`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewTemplateVersion {
    pub id: i64,
    pub template_id: i64,
    pub workspace_id: i64,
    pub version: i64,
    pub payload: Json,
    pub payload_hash: String,
    pub format_version: i64,
    pub changes: Option<Json>,
}

/// One row of [`list_history`] / [`list_changes_between`] (no payload).
#[derive(Debug, Clone, PartialEq)]
pub struct VersionChanges {
    pub version: i64,
    pub created_at: i64,
    pub changes: Option<Json>,
}

/// `lockVersions`: a no-op under the single writer.
pub fn lock_versions(conn: &Connection, template_id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `insert`: on the same `(templateId, version)` it replaces payload and format version.
pub fn insert(conn: &Connection, input: &NewTemplateVersion) -> Result<()> {
    todo!("P3.3")
}

fn changes_rows(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<VersionChanges>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(VersionChanges {
                version: row.get(0)?,
                created_at: row.get(1)?,
                changes: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listChangesBetween`: versions in `(after_version, up_to_version]`, ascending.
pub fn list_changes_between(
    conn: &Connection,
    template_id: i64,
    after_version: i64,
    up_to_version: i64,
) -> Result<Vec<VersionChanges>> {
    todo!("P3.3")
}

/// `listHistory`: newest first, at most [`VERSION_HISTORY_LIMIT`].
pub fn list_history(conn: &Connection, template_id: i64) -> Result<Vec<VersionChanges>> {
    todo!("P3.3")
}

/// `find`.
pub fn find(conn: &Connection, template_id: i64, version: i64) -> Result<Option<TemplateVersion>> {
    todo!("P3.3")
}

/// `exists` (reads no payload).
pub fn exists(conn: &Connection, template_id: i64, version: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `findLatestNumber`.
pub fn find_latest_number(conn: &Connection, template_id: i64) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `pinVersionNumber`: the requested version when stored, else the newest stored one,
/// else the requested number (reads no payload).
pub fn pin_version_number(
    conn: &Connection,
    template_id: i64,
    requested_version: i64,
) -> Result<i64> {
    todo!("P3.3")
}

/// `findLatest`.
pub fn find_latest(conn: &Connection, template_id: i64) -> Result<Option<TemplateVersion>> {
    todo!("P3.3")
}

/// `prune`: drops versions older than the newest [`RETAINED_VERSIONS`] that no unfinished
/// installation or sync is pinned to.
pub fn prune(conn: &Connection, template_id: i64, latest_version: i64) -> Result<()> {
    todo!("P3.3")
}
