//! Port of `repositories/template/` (`templateRepository`, `templateSnapshotRepository`,
//! `templatePreviewRepository`); no TS tests, smoke test `tests/repo_p33_smoke.rs`.
//!
//! - Narrow Drizzle `columns` projections of one row return the whole [`Template`] row.
//!   The list rows ([`list_with_install_counts`]) clear the omitted columns (`payload`,
//!   `selection` = JSON `null`, `shareToken` = `""`, `payloadHash` = `None`), and
//!   `installCount` holds the computed count.
//! - The `(createdAt, id) < (key::timestamptz, id)` page key is the ISO millisecond text
//!   (`repo::iso`), read back with `parse_iso`.
//! - Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`).

use std::collections::BTreeMap;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::Template;
use super::patch::{self, Patch};
use super::sql::{Sql, opt_int};
use super::template_version::{self, NewTemplateVersion};
use super::{Error, Json, Model, Result, id_list, iso, new_id, now_ms, parse_iso, text_list};

/// `ACTIVE_INSTALLATION_STATUSES`.
pub const ACTIVE_INSTALLATION_STATUSES: [&str; 2] = ["completed", "partial"];

// ---------------------------------------------------------------------------
// templateSnapshotRepository
// ---------------------------------------------------------------------------

/// `ManifestLookup`: ids, names, or neither (every row, up to `limit`).
#[derive(Debug, Clone, Default)]
pub struct ManifestLookup {
    pub workspace_id: i64,
    pub ids: Option<Vec<i64>>,
    pub names: Option<Vec<String>>,
    pub limit: Option<i64>,
}

impl ManifestLookup {
    fn matches_nothing(&self) -> bool {
        self.ids.as_ref().is_some_and(Vec::is_empty)
            || self.names.as_ref().is_some_and(Vec::is_empty)
    }

    fn filter(&self, sql: &mut Sql) {
        if let Some(ids) = &self.ids {
            sql.push(" AND \"id\" IN (SELECT value FROM json_each(");
            sql.push_bind(id_list(ids));
            sql.push("))");
        }
        if let Some(names) = &self.names {
            sql.push(" AND \"name\" IN (SELECT value FROM json_each(");
            sql.push_bind(text_list(names));
            sql.push("))");
        }
        sql.push(" ORDER BY \"createdAt\" ASC, \"id\" ASC");
        if let Some(limit) = self.limit {
            sql.push(" LIMIT ");
            sql.push_bind(limit);
        }
    }
}

/// A tag manifest row.
#[derive(Debug, Clone, PartialEq)]
pub struct TagManifest {
    pub id: i64,
    pub name: String,
    pub folder_id: Option<i64>,
}

/// A custom field manifest row.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomFieldManifest {
    pub id: i64,
    pub name: String,
    pub r#type: String,
    pub folder_id: Option<i64>,
}

/// `templateSnapshotRepository.findTags`: live tags, oldest first.
pub fn snapshot_find_tags(conn: &Connection, lookup: &ManifestLookup) -> Result<Vec<TagManifest>> {
    if lookup.matches_nothing() {
        return Ok(Vec::new());
    }
    let mut sql = Sql::new(
        "SELECT \"id\", \"name\", \"folderId\" FROM \"Tag\" WHERE \"workspaceId\" = ? AND \"deletedAt\" IS NULL",
        vec![Value::Integer(lookup.workspace_id)],
    );
    lookup.filter(&mut sql);
    sql.query_map(conn, |row| {
        Ok(TagManifest {
            id: row.get(0)?,
            name: row.get(1)?,
            folder_id: row.get(2)?,
        })
    })
}

/// `templateSnapshotRepository.findCustomFields`, oldest first.
pub fn snapshot_find_custom_fields(
    conn: &Connection,
    lookup: &ManifestLookup,
) -> Result<Vec<CustomFieldManifest>> {
    if lookup.matches_nothing() {
        return Ok(Vec::new());
    }
    let mut sql = Sql::new(
        "SELECT \"id\", \"name\", \"type\", \"folderId\" FROM \"CustomField\" WHERE \"workspaceId\" = ?",
        vec![Value::Integer(lookup.workspace_id)],
    );
    lookup.filter(&mut sql);
    sql.query_map(conn, |row| {
        Ok(CustomFieldManifest {
            id: row.get(0)?,
            name: row.get(1)?,
            r#type: row.get(2)?,
            folder_id: row.get(3)?,
        })
    })
}

// ---------------------------------------------------------------------------
// templatePreviewRepository
// ---------------------------------------------------------------------------

/// `{ id, name }`.
#[derive(Debug, Clone, PartialEq)]
pub struct IdName {
    pub id: i64,
    pub name: String,
}

/// `{ id, name, type }`.
#[derive(Debug, Clone, PartialEq)]
pub struct IdNameType {
    pub id: i64,
    pub name: String,
    pub r#type: String,
}

/// A folder of the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewFolder {
    pub id: i64,
    pub folder_type: String,
    pub name: String,
    pub parent_id: Option<i64>,
}

fn id_names(sql: Sql, conn: &Connection) -> Result<Vec<IdName>> {
    sql.query_map(conn, |row| {
        Ok(IdName {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })
}

fn id_name_types(sql: Sql, conn: &Connection) -> Result<Vec<IdNameType>> {
    sql.query_map(conn, |row| {
        Ok(IdNameType {
            id: row.get(0)?,
            name: row.get(1)?,
            r#type: row.get(2)?,
        })
    })
}

/// `templatePreviewRepository.findTagsByNames`: live tags with exactly these names.
pub fn preview_find_tags_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
) -> Result<Vec<IdName>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    id_names(
        Sql::new(
            "SELECT \"id\", \"name\" FROM \"Tag\" WHERE \"workspaceId\" = ? AND \"deletedAt\" IS NULL AND \"name\" IN (SELECT value FROM json_each(?))",
            vec![Value::Integer(workspace_id), Value::Text(text_list(names))],
        ),
        conn,
    )
}

/// `templatePreviewRepository.listPlaceCategories`.
pub fn preview_list_place_categories(conn: &Connection, workspace_id: i64) -> Result<Vec<IdName>> {
    id_names(
        Sql::new(
            "SELECT \"id\", \"name\" FROM \"PlaceCategory\" WHERE \"workspaceId\" = ?",
            vec![Value::Integer(workspace_id)],
        ),
        conn,
    )
}

/// `templatePreviewRepository.listCustomFields`.
pub fn preview_list_custom_fields(conn: &Connection, workspace_id: i64) -> Result<Vec<IdNameType>> {
    id_name_types(
        Sql::new(
            "SELECT \"id\", \"name\", \"type\" FROM \"CustomField\" WHERE \"workspaceId\" = ?",
            vec![Value::Integer(workspace_id)],
        ),
        conn,
    )
}

/// `templatePreviewRepository.listBotFields`.
pub fn preview_list_bot_fields(conn: &Connection, workspace_id: i64) -> Result<Vec<IdNameType>> {
    id_name_types(
        Sql::new(
            "SELECT \"id\", \"name\", \"type\" FROM \"BotField\" WHERE \"workspaceId\" = ?",
            vec![Value::Integer(workspace_id)],
        ),
        conn,
    )
}

/// `templatePreviewRepository.findFolders`: folders of these types with one of these names.
pub fn preview_find_folders(
    conn: &Connection,
    workspace_id: i64,
    folder_types: &[String],
    names: &[String],
) -> Result<Vec<PreviewFolder>> {
    if folder_types.is_empty() || names.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"folderType\", \"name\", \"parentId\" FROM \"Folder\" WHERE \"workspaceId\" = ? AND \"folderType\" IN (SELECT value FROM json_each(?)) AND \"name\" IN (SELECT value FROM json_each(?))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(text_list(folder_types)),
            Value::Text(text_list(names)),
        ],
    )
    .query_map(conn, |row| {
        Ok(PreviewFolder {
            id: row.get(0)?,
            folder_type: row.get(1)?,
            name: row.get(2)?,
            parent_id: row.get(3)?,
        })
    })
}

// ---------------------------------------------------------------------------
// templateRepository
// ---------------------------------------------------------------------------

fn one(conn: &Connection, where_sql: &str, params: Vec<Value>) -> Result<Option<Template>> {
    Sql::new(
        format!("{} WHERE {where_sql} LIMIT 1", Template::select_sql()),
        params,
    )
    .model(conn)
}

/// `findLive`: a live template of the workspace.
pub fn find_live(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<Template>> {
    one(
        conn,
        "\"id\" = ? AND \"workspaceId\" = ? AND \"deletedAt\" IS NULL",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    )
}

/// `findLiveDetail` (the TS leaves out `payload`; here the whole row).
pub fn find_live_detail(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<Template>> {
    find_live(conn, id, workspace_id)
}

/// `findLiveById`.
pub fn find_live_by_id(conn: &Connection, id: i64) -> Result<Option<Template>> {
    one(
        conn,
        "\"id\" = ? AND \"deletedAt\" IS NULL",
        vec![Value::Integer(id)],
    )
}

/// `findLiveByShareToken`.
pub fn find_live_by_share_token(conn: &Connection, share_token: &str) -> Result<Option<Template>> {
    one(
        conn,
        "\"shareToken\" = ? AND \"deletedAt\" IS NULL",
        vec![Value::Text(share_token.to_owned())],
    )
}

/// `findInstallSource` (`name`, `createInstallFolder`, `categoryCounts` of the row).
pub fn find_install_source(conn: &Connection, template_id: i64) -> Result<Option<Template>> {
    find_live_by_id(conn, template_id)
}

/// `findLivePayload`.
pub fn find_live_payload(conn: &Connection, template_id: i64) -> Result<Option<Json>> {
    Ok(find_live_by_id(conn, template_id)?.map(|t| t.payload))
}

/// `findCategoryCounts` (deleted templates too).
pub fn find_category_counts(conn: &Connection, template_id: i64) -> Result<Option<Json>> {
    Ok(one(conn, "\"id\" = ?", vec![Value::Integer(template_id)])?.map(|t| t.category_counts))
}

/// `findSyncSource` (`categoryCounts`, `defaultAutoUpdate` of the row).
pub fn find_sync_source(conn: &Connection, template_id: i64) -> Result<Option<Template>> {
    find_live_by_id(conn, template_id)
}

/// `findLiveTenant` (`id`, `tenantId` of the row).
pub fn find_live_tenant(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<Template>> {
    find_live(conn, id, workspace_id)
}

/// `findByIdForTenant` (deleted templates too).
pub fn find_by_id_for_tenant(
    conn: &Connection,
    id: i64,
    tenant_id: i64,
) -> Result<Option<Template>> {
    one(
        conn,
        "\"id\" = ? AND \"tenantId\" = ?",
        vec![Value::Integer(id), Value::Integer(tenant_id)],
    )
}

/// `findForStagingByShareToken`.
pub fn find_for_staging_by_share_token(
    conn: &Connection,
    share_token: &str,
) -> Result<Option<Template>> {
    find_live_by_share_token(conn, share_token)
}

/// `findLiveVersion`.
pub fn find_live_version(conn: &Connection, template_id: i64) -> Result<Option<i64>> {
    Ok(find_live_by_id(conn, template_id)?.map(|t| t.version))
}

fn update_live(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    set: Sql,
) -> Result<Option<Template>> {
    let mut sql = Sql::raw("UPDATE \"Template\" SET ");
    sql.append(set);
    sql.push(", \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE \"id\" = ");
    sql.push_bind(id);
    sql.push(" AND \"workspaceId\" = ");
    sql.push_bind(workspace_id);
    sql.push(&format!(
        " AND \"deletedAt\" IS NULL RETURNING {}",
        Template::columns_sql(None)
    ));
    sql.model(conn)
}

/// `rotateShareToken`.
pub fn rotate_share_token(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    share_token: &str,
) -> Result<Option<Template>> {
    update_live(
        conn,
        id,
        workspace_id,
        Sql::bind("\"shareToken\" = ", share_token.to_owned()),
    )
}

/// `updateShareSettings`: `share_expires_at` `None` leaves the column (TS `undefined`).
pub fn update_share_settings(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    share_enabled: bool,
    share_expires_at: Option<Option<i64>>,
) -> Result<Option<Template>> {
    let mut set = Sql::bind("\"shareEnabled\" = ", i64::from(share_enabled));
    if let Some(at) = share_expires_at {
        set.push(", \"shareExpiresAt\" = ");
        set.push_bind(opt_int(at));
    }
    update_live(conn, id, workspace_id, set)
}

/// `TemplateVersionSnapshot`.
#[derive(Debug, Clone)]
pub struct TemplateVersionSnapshot {
    pub payload: Json,
    pub payload_hash: String,
    pub format_version: i64,
    pub changes: Option<Json>,
}

/// `createWithFirstVersion`: the template and its version 1. Run it in one write job.
pub fn create_with_first_version(
    conn: &Connection,
    values: &Patch,
    version_id: i64,
    snapshot: &TemplateVersionSnapshot,
) -> Result<Template> {
    let mut all = values.clone();
    if !all.iter().any(|(c, _)| *c == "id") {
        all.insert(0, ("id", Value::Integer(new_id())));
    }
    let mut sql = Sql::raw("INSERT INTO \"Template\" ");
    sql.append(patch::insert_clause(Template::COLUMNS, &all)?);
    sql.push(&format!(" RETURNING {}", Template::columns_sql(None)));
    let created = sql
        .model::<Template>(conn)?
        .ok_or_else(|| Error::msg("template insert returned no row"))?;
    template_version::insert(
        conn,
        &NewTemplateVersion {
            id: version_id,
            template_id: created.id,
            workspace_id: created.workspace_id,
            version: 1,
            payload: snapshot.payload.clone(),
            payload_hash: snapshot.payload_hash.clone(),
            format_version: snapshot.format_version,
            changes: snapshot.changes.clone(),
        },
    )?;
    Ok(created)
}

/// `saveWithVersion` input.
#[derive(Debug, Clone)]
pub struct SaveWithVersion {
    pub id: i64,
    pub workspace_id: i64,
    pub version_id: i64,
    /// Columns to set (never `version` / `payloadHash`).
    pub set: Patch,
    pub snapshot: TemplateVersionSnapshot,
    pub legacy_payload_hash: Option<String>,
    pub based_on_version: i64,
}

/// `saveWithVersion`: bumps `version` when the base moved or the hash changed, records
/// the version and prunes old ones. Run it in one write job.
pub fn save_with_version(conn: &Connection, input: &SaveWithVersion) -> Result<Option<Template>> {
    if input
        .set
        .iter()
        .any(|(c, _)| *c == "version" || *c == "payloadHash")
    {
        return Err(Error::msg(
            "saveWithVersion: set must not hold version or payloadHash",
        ));
    }
    template_version::lock_versions(conn, input.id)?;
    let mut set = Sql::default();
    if !input.set.is_empty() {
        set.append(patch::set_clause(Template::COLUMNS, &input.set)?);
        set.push(", ");
    }
    set.push("\"payloadHash\" = ");
    set.push_bind(input.snapshot.payload_hash.clone());
    set.push(", \"version\" = CASE WHEN \"version\" <> ");
    set.push_bind(input.based_on_version);
    set.push(" OR COALESCE(");
    set.push_bind(
        input
            .legacy_payload_hash
            .clone()
            .map_or(Value::Null, Value::Text),
    );
    set.push(", \"payloadHash\") IS NOT ");
    set.push_bind(input.snapshot.payload_hash.clone());
    set.push(" THEN \"version\" + 1 ELSE \"version\" END");
    let Some(updated) = update_live(conn, input.id, input.workspace_id, set)? else {
        return Ok(None);
    };
    template_version::insert(
        conn,
        &NewTemplateVersion {
            id: input.version_id,
            template_id: updated.id,
            workspace_id: updated.workspace_id,
            version: updated.version,
            payload: input.snapshot.payload.clone(),
            payload_hash: input.snapshot.payload_hash.clone(),
            format_version: input.snapshot.format_version,
            changes: input.snapshot.changes.clone(),
        },
    )?;
    template_version::prune(conn, updated.id, updated.version)?;
    Ok(Some(updated))
}

/// `softDelete`: marks the template deleted and its installations source-deleted. Run it
/// in one write job.
pub fn soft_delete(conn: &Connection, id: i64, workspace_id: i64) -> Result<bool> {
    let now = now_ms();
    let deleted = update_live(conn, id, workspace_id, Sql::bind("\"deletedAt\" = ", now))?;
    if deleted.is_none() {
        return Ok(false);
    }
    super::template_installation::mark_source_deleted(conn, id)?;
    Ok(true)
}

/// `findInboxManifestByShareToken` row.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxManifest {
    /// `payload -> 'manifests' -> 'inboxes'` (`None` = SQL null).
    pub inboxes: Option<serde_json::Value>,
    pub share_enabled: bool,
    pub share_expires_at: Option<i64>,
}

/// `findInboxManifestByShareToken`.
pub fn find_inbox_manifest_by_share_token(
    conn: &Connection,
    share_token: &str,
) -> Result<Option<InboxManifest>> {
    let rows = Sql::new(
        "SELECT \"payload\" -> '$.manifests.inboxes', \"shareEnabled\", \"shareExpiresAt\" FROM \"Template\" WHERE \"shareToken\" = ? AND \"deletedAt\" IS NULL LIMIT 1",
        vec![Value::Text(share_token.to_owned())],
    )
    .query_map(conn, |row| {
        let inboxes: Option<String> = row.get(0)?;
        Ok((inboxes, row.get::<_, bool>(1)?, row.get::<_, Option<i64>>(2)?))
    })?;
    rows.into_iter()
        .next()
        .map(|(inboxes, share_enabled, share_expires_at)| {
            let inboxes = match inboxes {
                Some(text) => {
                    Some(serde_json::from_str(&text).map_err(|e| Error::msg(e.to_string()))?)
                }
                None => None,
            };
            Ok(InboxManifest {
                inboxes,
                share_enabled,
                share_expires_at,
            })
        })
        .transpose()
}

fn list_row(mut t: Template) -> Template {
    t.payload = Json(serde_json::Value::Null);
    t.selection = Json(serde_json::Value::Null);
    t.share_token = String::new();
    t.payload_hash = None;
    t
}

/// `withInstallCounts`: `installCount` = sum of `successfulInstallEvents` of active installs.
fn with_install_counts(conn: &Connection, templates: Vec<Template>) -> Result<Vec<Template>> {
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = templates.iter().map(|t| t.id).collect();
    let counts: BTreeMap<i64, i64> = Sql::new(
        "SELECT \"templateId\", COALESCE(SUM(\"successfulInstallEvents\"), 0) FROM \"TemplateInstallation\" WHERE \"templateId\" IN (SELECT value FROM json_each(?)) AND \"status\" IN ('completed', 'partial') GROUP BY \"templateId\"",
        vec![Value::Text(id_list(&ids))],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
    .into_iter()
    .collect();
    Ok(templates
        .into_iter()
        .map(|t| {
            let count = counts.get(&t.id).copied().unwrap_or(0);
            let mut t = list_row(t);
            t.install_count = count;
            t
        })
        .collect())
}

/// `listWithInstallCounts`: live templates, newest first.
pub fn list_with_install_counts(conn: &Connection, workspace_id: i64) -> Result<Vec<Template>> {
    let rows = Sql::new(
        format!(
            "{} WHERE \"workspaceId\" = ? AND \"deletedAt\" IS NULL ORDER BY \"createdAt\" DESC, \"id\" DESC",
            Template::select_sql()
        ),
        vec![Value::Integer(workspace_id)],
    )
    .models::<Template>(conn)?;
    with_install_counts(conn, rows)
}

/// One page of [`list_with_install_counts_page`].
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatePage {
    pub rows: Vec<Template>,
    pub has_more: bool,
    /// The ISO text of the last row's `createdAt`.
    pub last_key: Option<String>,
}

/// `listWithInstallCountsPage`: `after` = `(key, id)` of the previous page.
pub fn list_with_install_counts_page(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    after: Option<(&str, i64)>,
) -> Result<TemplatePage> {
    let mut sql = Sql::new(
        format!(
            "{} WHERE \"workspaceId\" = ? AND \"deletedAt\" IS NULL",
            Template::select_sql()
        ),
        vec![Value::Integer(workspace_id)],
    );
    if let Some((key, id)) = after {
        let key_ms =
            parse_iso(key).ok_or_else(|| Error::msg(format!("invalid page key: {key}")))?;
        sql.push(" AND (\"createdAt\", \"id\") < (");
        sql.push_bind(key_ms);
        sql.push(", ");
        sql.push_bind(id);
        sql.push(")");
    }
    sql.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    sql.push_bind(limit + 1);
    let mut fetched = sql.models::<Template>(conn)?;
    let has_more = fetched.len() as i64 > limit;
    fetched.truncate(usize::try_from(limit.max(0)).unwrap_or(0));
    let last_key = fetched.last().map(|t| iso(t.created_at));
    Ok(TemplatePage {
        rows: with_install_counts(conn, fetched)?,
        has_more,
        last_key,
    })
}

/// `installAnalytics` result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstallAnalytics {
    /// `(status, count)` per status.
    pub by_status: Vec<(String, i64)>,
    pub active_auto_update: i64,
    pub total_install_events: i64,
    /// `(version, count)` of active installs, by version.
    pub by_version: Vec<(i64, i64)>,
}

/// `installAnalytics`.
pub fn install_analytics(conn: &Connection, template_id: i64) -> Result<InstallAnalytics> {
    let by_status: Vec<(String, i64, i64, i64)> = Sql::new(
        "SELECT \"status\", count(*), count(*) FILTER (WHERE \"autoUpdate\"), COALESCE(SUM(\"successfulInstallEvents\"), 0) FROM \"TemplateInstallation\" WHERE \"templateId\" = ? GROUP BY \"status\"",
        vec![Value::Integer(template_id)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
    let by_version: Vec<(i64, i64)> = Sql::new(
        "SELECT \"sourceVersion\", count(*) FROM \"TemplateInstallation\" WHERE \"templateId\" = ? AND \"status\" IN ('completed', 'partial') GROUP BY \"sourceVersion\" ORDER BY \"sourceVersion\"",
        vec![Value::Integer(template_id)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(InstallAnalytics {
        active_auto_update: by_status
            .iter()
            .filter(|r| ACTIVE_INSTALLATION_STATUSES.contains(&r.0.as_str()))
            .map(|r| r.2)
            .sum(),
        total_install_events: by_status.iter().map(|r| r.3).sum(),
        by_status: by_status.into_iter().map(|r| (r.0, r.1)).collect(),
        by_version,
    })
}

/// `listPublisherRows` input.
#[derive(Debug, Clone, Default)]
pub struct ListPublisherRows {
    pub template_id: i64,
    pub search: Option<String>,
    /// `(lastInstalledAt, publisherRef)` of the previous page.
    pub after: Option<(i64, String)>,
    pub limit: i64,
}

/// `PublisherInstallationRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct PublisherInstallationRow {
    pub publisher_ref: String,
    pub workspace_name: Option<String>,
    pub install_count: i64,
    pub total_install_events: i64,
    pub last_installed_at: Option<i64>,
    pub oldest_version: Option<i64>,
    pub auto_update: bool,
    pub active: bool,
    pub uninstall_state: String,
    pub installed_automatically: bool,
    pub eligible_for_uninstall: bool,
    pub uninstall_counts: Option<serde_json::Value>,
}

struct PublisherSource {
    publisher_ref: String,
    workspace_name: String,
    grant: i64,
    events: i64,
    completed_at: Option<i64>,
    created_at: i64,
    source_version: i64,
    auto_update: bool,
    active: bool,
    uninstall_state: String,
    requested_at: Option<i64>,
    install_source: String,
    op_counts: Option<String>,
}

/// The `array_agg(x ORDER BY uninstallRequestedAt DESC NULLS LAST)[1]` pick.
fn latest_request<'a>(rows: &[&'a PublisherSource]) -> Option<&'a PublisherSource> {
    let mut best: Option<&PublisherSource> = None;
    for row in rows {
        best = match best {
            None => Some(row),
            Some(b) => {
                let better = match (row.requested_at, b.requested_at) {
                    (Some(a), Some(c)) => a > c,
                    (Some(_), None) => true,
                    _ => false,
                };
                if better { Some(row) } else { Some(b) }
            }
        };
    }
    best
}

/// `listPublisherRows`: per publisher workspace, the install summary of a template (the
/// Postgres `GROUP BY publisherRef` aggregates run in Rust over the joined rows;
/// `min(workspace name)` compares by code point).
pub fn list_publisher_rows(
    conn: &Connection,
    input: &ListPublisherRows,
) -> Result<Vec<PublisherInstallationRow>> {
    let mut sql = Sql::new(
        "SELECT ti.\"publisherRef\", w.\"name\", ti.\"publisherUninstallGrant\", ti.\"successfulInstallEvents\", ti.\"completedAt\", ti.\"createdAt\", ti.\"sourceVersion\", ti.\"autoUpdate\", ti.\"status\" IN ('completed', 'partial'), ti.\"uninstallState\", ti.\"uninstallRequestedAt\", ti.\"installSource\", op.\"counts\" FROM \"TemplateInstallation\" ti JOIN \"Workspace\" w ON w.\"id\" = ti.\"workspaceId\" LEFT JOIN \"TemplateUninstallOperation\" op ON op.\"id\" = ti.\"uninstallOperationId\" WHERE ti.\"templateId\" = ? AND ti.\"status\" IN ('completed', 'partial', 'failed', 'uninstalled')",
        vec![Value::Integer(input.template_id)],
    );
    if let Some(search) = input.search.as_deref().filter(|s| !s.is_empty()) {
        let mut escaped = String::with_capacity(search.len() + 2);
        for c in search.chars() {
            if matches!(c, '\\' | '%' | '_') {
                escaped.push('\\');
            }
            escaped.push(c);
        }
        sql.push(" AND ti.\"publisherUninstallGrant\" > 0 AND ilike(w.\"name\", ");
        sql.push_bind(format!("%{escaped}%"));
        sql.push(")");
    }
    let rows = sql.query_map(conn, |row| {
        Ok(PublisherSource {
            publisher_ref: row.get(0)?,
            workspace_name: row.get(1)?,
            grant: row.get(2)?,
            events: row.get(3)?,
            completed_at: row.get(4)?,
            created_at: row.get(5)?,
            source_version: row.get(6)?,
            auto_update: row.get(7)?,
            active: row.get(8)?,
            uninstall_state: row.get(9)?,
            requested_at: row.get(10)?,
            install_source: row.get(11)?,
            op_counts: row.get(12)?,
        })
    })?;
    let mut groups: BTreeMap<String, Vec<&PublisherSource>> = BTreeMap::new();
    for row in &rows {
        groups
            .entry(row.publisher_ref.clone())
            .or_default()
            .push(row);
    }
    let mut out = Vec::new();
    for (publisher_ref, group) in groups {
        let active_rows: Vec<&&PublisherSource> = group.iter().filter(|r| r.active).collect();
        let active = !active_rows.is_empty();
        let last_installed_at = group
            .iter()
            .filter_map(|r| r.completed_at)
            .max()
            .or_else(|| group.iter().map(|r| r.created_at).max());
        let latest = latest_request(&group);
        let uninstall_counts = if active {
            None
        } else {
            latest
                .and_then(|r| r.op_counts.as_deref())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                .and_then(|v| parse_template_uninstall_counts(&v))
        };
        out.push(PublisherInstallationRow {
            workspace_name: group
                .iter()
                .filter(|r| r.grant > 0)
                .map(|r| r.workspace_name.clone())
                .min(),
            install_count: active_rows.iter().map(|r| r.events).sum(),
            total_install_events: group.iter().map(|r| r.events).sum(),
            last_installed_at,
            oldest_version: active_rows.iter().map(|r| r.source_version).min(),
            auto_update: active_rows.iter().any(|r| r.auto_update),
            active,
            uninstall_state: latest
                .map_or_else(|| "none".to_owned(), |r| r.uninstall_state.clone()),
            installed_automatically: group.iter().any(|r| r.install_source == "tenantDefault"),
            eligible_for_uninstall: active && active_rows.iter().all(|r| r.grant > 0),
            uninstall_counts,
            publisher_ref,
        });
    }
    if let Some((at, ref_)) = &input.after {
        out.retain(|r| match r.last_installed_at {
            Some(last) => (last, r.publisher_ref.as_str()) < (*at, ref_.as_str()),
            None => false,
        });
    }
    out.sort_by(|a, b| {
        // `ORDER BY lastInstalledAt DESC` (Postgres: NULLs first), then `publisherRef DESC`.
        let ka = (
            a.last_installed_at.is_none(),
            a.last_installed_at,
            &a.publisher_ref,
        );
        let kb = (
            b.last_installed_at.is_none(),
            b.last_installed_at,
            &b.publisher_ref,
        );
        kb.cmp(&ka)
    });
    out.truncate(usize::try_from(input.limit.max(0)).unwrap_or(0));
    Ok(out)
}

/// `TEMPLATE_UNINSTALL_COUNT_KEYS` (`templateUninstallCountsSchema` keys).
pub const TEMPLATE_UNINSTALL_COUNT_KEYS: [&str; 12] = [
    "deleted",
    "deletedModified",
    "deletedUnverified",
    "keptModified",
    "keptShared",
    "keptByPolicy",
    "keptHasDependents",
    "blockedByReference",
    "keptPartlyRemoved",
    "keptLockedByOther",
    "absent",
    "failed",
];

/// `parseTemplateUninstallCounts`: stored counts with every key (default 0); `None` for
/// JSON `null` or an invalid value.
pub fn parse_template_uninstall_counts(value: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = value.as_object()?;
    let mut out = serde_json::Map::new();
    for key in TEMPLATE_UNINSTALL_COUNT_KEYS {
        let n = match obj.get(key) {
            None => 0,
            Some(v) => {
                let f = v.as_f64()?;
                if f < 0.0 || f.fract() != 0.0 || !v.is_number() {
                    return None;
                }
                v.as_i64().unwrap_or(f as i64)
            }
        };
        out.insert(key.to_owned(), serde_json::Value::from(n));
    }
    Some(serde_json::Value::Object(out))
}
