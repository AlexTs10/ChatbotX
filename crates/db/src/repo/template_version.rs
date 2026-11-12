//! Port of `repositories/template-version/repository.ts` (P3.3). The advisory locks
//! (`lockVersions`, the shared lock of `pinVersionNumber`) are not needed under the one
//! SQLite writer.

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
    let _ = (conn, template_id);
    Ok(())
}

/// `insert`: on the same `(templateId, version)` it replaces payload and format version.
pub fn insert(conn: &Connection, input: &NewTemplateVersion) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"TemplateVersion\" (\"id\", \"createdAt\", \"updatedAt\", \"templateId\", \"workspaceId\", \"version\", \
         \"payload\", \"payloadHash\", \"formatVersion\", \"changes\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT (\"templateId\", \"version\") DO UPDATE SET \"payload\" = excluded.\"payload\", \
         \"formatVersion\" = excluded.\"formatVersion\", \"updatedAt\" = excluded.\"updatedAt\"",
        rusqlite::params![
            input.id,
            now,
            input.template_id,
            input.workspace_id,
            input.version,
            input.payload,
            input.payload_hash,
            input.format_version,
            input.changes
        ],
    )?;
    Ok(())
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
    changes_rows(
        conn,
        "SELECT \"version\", \"createdAt\", \"changes\" FROM \"TemplateVersion\" \
         WHERE \"templateId\" = ?1 AND \"version\" > ?2 AND \"version\" <= ?3 ORDER BY \"version\" ASC",
        &[&template_id, &after_version, &up_to_version],
    )
}

/// `listHistory`: newest first, at most [`VERSION_HISTORY_LIMIT`].
pub fn list_history(conn: &Connection, template_id: i64) -> Result<Vec<VersionChanges>> {
    changes_rows(
        conn,
        "SELECT \"version\", \"createdAt\", \"changes\" FROM \"TemplateVersion\" \
         WHERE \"templateId\" = ?1 ORDER BY \"version\" DESC LIMIT ?2",
        &[&template_id, &VERSION_HISTORY_LIMIT],
    )
}

/// `find`.
pub fn find(conn: &Connection, template_id: i64, version: i64) -> Result<Option<TemplateVersion>> {
    super::find_one(
        conn,
        "\"templateId\" = ?1 AND \"version\" = ?2",
        &[&template_id, &version],
    )
}

/// `exists` (reads no payload).
pub fn exists(conn: &Connection, template_id: i64, version: i64) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT 1 FROM \"TemplateVersion\" WHERE \"templateId\" = ?1 AND \"version\" = ?2 LIMIT 1",
    )?;
    Ok(stmt.exists(rusqlite::params![template_id, version])?)
}

/// `findLatestNumber`.
pub fn find_latest_number(conn: &Connection, template_id: i64) -> Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"version\" FROM \"TemplateVersion\" WHERE \"templateId\" = ?1 ORDER BY \"version\" DESC LIMIT 1",
    )?;
    Ok(stmt.query_row([template_id], |r| r.get(0)).optional()?)
}

/// `pinVersionNumber`: the requested version when stored, else the newest stored one,
/// else the requested number (reads no payload).
pub fn pin_version_number(
    conn: &Connection,
    template_id: i64,
    requested_version: i64,
) -> Result<i64> {
    if exists(conn, template_id, requested_version)? {
        return Ok(requested_version);
    }
    Ok(find_latest_number(conn, template_id)?.unwrap_or(requested_version))
}

/// `findLatest`.
pub fn find_latest(conn: &Connection, template_id: i64) -> Result<Option<TemplateVersion>> {
    Ok(super::find_many::<TemplateVersion>(
        conn,
        "WHERE \"templateId\" = ?1 ORDER BY \"version\" DESC LIMIT 1",
        &[&template_id],
    )?
    .into_iter()
    .next())
}

/// `prune`: drops versions older than the newest [`RETAINED_VERSIONS`] that no unfinished
/// installation or sync is pinned to.
pub fn prune(conn: &Connection, template_id: i64, latest_version: i64) -> Result<()> {
    let cutoff = latest_version - RETAINED_VERSIONS;
    if cutoff < 1 {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"TemplateVersion\" WHERE \"templateId\" = ?1 AND \"version\" <= ?2 \
         AND NOT EXISTS (SELECT 1 FROM \"TemplateInstallation\" ti \
           WHERE ti.\"sourceTemplateId\" = \"TemplateVersion\".\"templateId\" \
             AND ti.\"sourceVersion\" = \"TemplateVersion\".\"version\" \
             AND ti.\"status\" IN ('pending', 'installing', 'failed')) \
         AND NOT EXISTS (SELECT 1 FROM \"TemplateInstallationSync\" s \
           JOIN \"TemplateInstallation\" ti ON ti.\"id\" = s.\"installationId\" \
           WHERE ti.\"sourceTemplateId\" = \"TemplateVersion\".\"templateId\" \
             AND s.\"requestedVersion\" = \"TemplateVersion\".\"version\" \
             AND s.\"status\" IN ('pending', 'running'))",
        rusqlite::params![template_id, cutoff],
    )?;
    Ok(())
}
