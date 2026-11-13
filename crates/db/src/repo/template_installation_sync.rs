//! Port of `repositories/template-installation-sync/repository.ts`
//! (`templateInstallationSyncRepository`), first part: the installed-resource readers and
//! writers. Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::TemplateInstalledResource;
use super::sql::Sql;
use super::template_installed_resource::keep_known_resource_kinds;
use super::{Model, Result, id_list, now_ms};

/// `listInstalledResources`: an installation's tracked resources of a known kind.
pub fn list_installed_resources(
    conn: &Connection,
    installation_id: i64,
) -> Result<Vec<TemplateInstalledResource>> {
    let rows = Sql::new(
        format!(
            "{} WHERE \"installationId\" = ?",
            TemplateInstalledResource::select_sql()
        ),
        vec![Value::Integer(installation_id)],
    )
    .models::<TemplateInstalledResource>(conn)?;
    Ok(keep_known_resource_kinds(rows))
}

/// `listInstalledResourcesByInstallationIds`.
pub fn list_installed_resources_by_installation_ids(
    conn: &Connection,
    installation_ids: &[i64],
) -> Result<Vec<TemplateInstalledResource>> {
    if installation_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = Sql::new(
        format!(
            "{} WHERE \"installationId\" IN (SELECT value FROM json_each(?))",
            TemplateInstalledResource::select_sql()
        ),
        vec![Value::Text(id_list(installation_ids))],
    )
    .models::<TemplateInstalledResource>(conn)?;
    Ok(keep_known_resource_kinds(rows))
}

/// `markSynced`: records what the sync wrote.
pub fn mark_synced(
    conn: &Connection,
    id: i64,
    synced_hash: Option<&str>,
    synced_version: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE \"TemplateInstalledResource\" SET \"syncedHash\" = ?1, \"syncedVersion\" = ?2, \"updatedAt\" = ?3 WHERE \"id\" = ?4",
        (synced_hash, synced_version, now_ms(), id),
    )?;
    Ok(())
}

/// `detachResources`: the template no longer ships these; they stop being governed.
pub fn detach_resources(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_ms();
    conn.execute(
        "UPDATE \"TemplateInstalledResource\" SET \"detachedAt\" = ?1, \"updatedAt\" = ?1 WHERE \"id\" IN (SELECT value FROM json_each(?2))",
        (now, id_list(ids)),
    )?;
    Ok(())
}
