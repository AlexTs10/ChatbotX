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
    todo!("P3.3")
}

/// `listInstalledResourcesByInstallationIds`.
pub fn list_installed_resources_by_installation_ids(
    conn: &Connection,
    installation_ids: &[i64],
) -> Result<Vec<TemplateInstalledResource>> {
    todo!("P3.3")
}

/// `markSynced`: records what the sync wrote.
pub fn mark_synced(
    conn: &Connection,
    id: i64,
    synced_hash: Option<&str>,
    synced_version: i64,
) -> Result<()> {
    todo!("P3.3")
}

/// `detachResources`: the template no longer ships these; they stop being governed.
pub fn detach_resources(conn: &Connection, ids: &[i64]) -> Result<()> {
    todo!("P3.3")
}
