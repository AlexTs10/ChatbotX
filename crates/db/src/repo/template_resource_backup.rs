//! Port of `repositories/template-resource-backup/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::TemplateResourceBackup;
use super::{Json, Result, id_list, now_ms};

/// `RETAINED_BACKUPS_PER_RESOURCE`.
pub const RETAINED_BACKUPS_PER_RESOURCE: i64 = 5;

/// `NewTemplateResourceBackup`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTemplateResourceBackup {
    pub id: i64,
    pub workspace_id: i64,
    pub installation_id: i64,
    pub sync_id: i64,
    pub installed_resource_id: i64,
    pub resource_kind: String,
    pub resource_id: i64,
    pub format_version: i64,
    pub content: Json,
    pub byte_size: i64,
}

/// The columns `listBySyncs` reads (no content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSummary {
    pub id: i64,
    pub sync_id: i64,
    pub installed_resource_id: i64,
    pub resource_kind: String,
    pub resource_id: i64,
    pub byte_size: i64,
    pub created_at: i64,
}

/// `insert`.
pub fn insert(conn: &Connection, v: &NewTemplateResourceBackup) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"TemplateResourceBackup\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"installationId\", \
         \"syncId\", \"installedResourceId\", \"resourceKind\", \"resourceId\", \"formatVersion\", \"content\", \"byteSize\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            v.id,
            now,
            v.workspace_id,
            v.installation_id,
            v.sync_id,
            v.installed_resource_id,
            v.resource_kind,
            v.resource_id,
            v.format_version,
            v.content,
            v.byte_size
        ],
    )?;
    Ok(())
}

/// `findByIdAndInstallation`.
pub fn find_by_id_and_installation(
    conn: &Connection,
    id: i64,
    installation_id: i64,
    workspace_id: i64,
) -> Result<Option<TemplateResourceBackup>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"installationId\" = ?2 AND \"workspaceId\" = ?3",
        &[&id, &installation_id, &workspace_id],
    )
}

/// `listBySync`.
pub fn list_by_sync(conn: &Connection, sync_id: i64) -> Result<Vec<BackupSummary>> {
    list_by_syncs(conn, &[sync_id])
}

/// `listBySyncs`: by id ascending (no content).
pub fn list_by_syncs(conn: &Connection, sync_ids: &[i64]) -> Result<Vec<BackupSummary>> {
    if sync_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"syncId\", \"installedResourceId\", \"resourceKind\", \"resourceId\", \"byteSize\", \"createdAt\" \
         FROM \"TemplateResourceBackup\" WHERE \"syncId\" IN (SELECT value FROM json_each(?1)) ORDER BY \"id\" ASC",
    )?;
    let rows = stmt
        .query_map([id_list(sync_ids)], |r| {
            Ok(BackupSummary {
                id: r.get(0)?,
                sync_id: r.get(1)?,
                installed_resource_id: r.get(2)?,
                resource_kind: r.get(3)?,
                resource_id: r.get(4)?,
                byte_size: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `pruneForResource`: keeps the newest [`RETAINED_BACKUPS_PER_RESOURCE`] backups.
pub fn prune_for_resource(conn: &Connection, installed_resource_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM \"TemplateResourceBackup\" WHERE \"installedResourceId\" = ?1 AND \"id\" NOT IN ( \
           SELECT \"id\" FROM \"TemplateResourceBackup\" WHERE \"installedResourceId\" = ?1 ORDER BY \"id\" DESC LIMIT ?2)",
        rusqlite::params![installed_resource_id, RETAINED_BACKUPS_PER_RESOURCE],
    )?;
    Ok(())
}

/// `deleteByIds`.
pub fn delete_by_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"TemplateResourceBackup\" WHERE \"id\" IN (SELECT value FROM json_each(?1))",
        [id_list(ids)],
    )?;
    Ok(())
}
