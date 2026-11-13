//! Port of `repositories/template-installation/` (`templateInstallationRepository`: the
//! request, lease, staged-cleanup and publish-effects parts, and `shared.ts`); no TS
//! tests, smoke test `tests/repo_p33_smoke.rs`.
//!
//! - `lockInstallRequest` (`pg_advisory_xact_lock`) is a no-op under the one SQLite writer.
//! - Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`); "now" (`new Date()`) is
//!   `now_ms()`.
//! - `pendingPublishEffects` items are `{ "flowId": <id>, "versionId": <id> }` with integer
//!   ids. `clearPendingPublishEffects` (jsonb `IN`) is a read-modify-write with JSON value
//!   equality.
//! - `holdsCategoryOutside`: `jsonb_each_text(categoryCounts)` = `json_each`,
//!   `total::numeric > 0` = `CAST(value AS REAL) > 0`.

use std::collections::BTreeMap;

use rusqlite::Connection;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use super::models::TemplateInstallation;
use super::patch::{self, Patch};
use super::sql::{Sql, opt_int};
use super::template::parse_template_uninstall_counts;
use super::{Error, Json, Model, Result, id_list, json_text, new_id, now_ms, text_list};

/// `TEMPLATE_INSTALLER_WARNING_CODES`.
pub const TEMPLATE_INSTALLER_WARNING_CODES: [&str; 24] = [
    "aiFileTooLarge",
    "aiFileUnavailable",
    "assigneeRequired",
    "backupTooLarge",
    "backupUnavailable",
    "botFieldRenamed",
    "categoryPermissionDenied",
    "couponTopicExpired",
    "credentialRequired",
    "customFieldRenamed",
    "enqueueFailed",
    "flowInvalidStructure",
    "flowNotPublished",
    "flowSkippedRequiredReference",
    "inboxBindingRequired",
    "localModifiedOverwritten",
    "name",
    "noEmbeddingProvider",
    "notUpdatable",
    "postSelectionRequired",
    "questionImage",
    "quota",
    "savedReplyImage",
    "workspaceUnavailable",
];

const TI: &str = "\"TemplateInstallation\"";

// ---------------------------------------------------------------------------
// shared.ts
// ---------------------------------------------------------------------------

/// `holdsCategoryOutside(visible)` over the row alias `ti`.
pub fn holds_category_outside(visible: &[String]) -> Sql {
    let not_in = |col: &str| -> Sql {
        if visible.is_empty() {
            Sql::default()
        } else {
            Sql::bind(
                format!(" AND {col} NOT IN (SELECT value FROM json_each("),
                text_list(visible),
            )
            .closed("))")
        }
    };
    let mut sql = Sql::raw(
        "(EXISTS (SELECT 1 FROM \"TemplateInstalledResource\" r WHERE r.\"installationId\" = ti.\"id\"",
    );
    sql.append(not_in("r.\"category\""));
    sql.push(
        ") OR (NOT EXISTS (SELECT 1 FROM \"TemplateInstalledResource\" r WHERE r.\"installationId\" = ti.\"id\") AND (NOT EXISTS (SELECT 1 FROM \"Template\" t WHERE t.\"id\" = ti.\"sourceTemplateId\") OR EXISTS (SELECT 1 FROM \"Template\" t, json_each(t.\"categoryCounts\") AS counts WHERE t.\"id\" = ti.\"sourceTemplateId\" AND CAST(counts.value AS REAL) > 0",
    );
    sql.append(not_in("counts.key"));
    sql.push("))))");
    sql
}

/// `stagedCleanupOwed` over the row alias `ti`.
const STAGED_CLEANUP_OWED: &str = "((ti.\"status\" = 'failed' AND ti.\"stagedCleanedGeneration\" < ti.\"stagingGeneration\") OR (ti.\"status\" <> 'failed' AND ti.\"stagedCleanedGeneration\" < ti.\"stagingGeneration\" - 1))";

/// `hasActiveSync()` over the row alias `ti`.
const HAS_ACTIVE_SYNC: &str = "EXISTS (SELECT 1 FROM \"TemplateInstallationSync\" s WHERE s.\"installationId\" = ti.\"id\" AND s.\"status\" IN ('pending', 'running'))";

/// `installerWarningCount()` over the row alias `ti`.
fn installer_warning_count() -> Sql {
    Sql::bind(
        "(SELECT count(*) FROM json_each(ti.\"warnings\") AS w WHERE json_extract(w.value, '$.entityKind') IN (SELECT value FROM json_each(",
        text_list(&TEMPLATE_INSTALLER_WARNING_CODES),
    )
    .closed(")))")
}

/// `unfinishedInstallation`.
fn unfinished_installation(pending_before: i64, lease_expired_before: i64) -> Sql {
    Sql::new(
        "((ti.\"status\" = 'pending' AND ti.\"createdAt\" < ?) OR (ti.\"status\" = 'installing' AND ti.\"claimedAt\" < ?))",
        vec![
            Value::Integer(pending_before),
            Value::Integer(lease_expired_before),
        ],
    )
}

/// `stagedCleanupDue`.
fn staged_cleanup_due(completed_before: i64) -> Sql {
    Sql::bind(
        format!("({STAGED_CLEANUP_OWED} AND (ti.\"status\" <> 'failed' OR ti.\"completedAt\" < "),
        completed_before,
    )
    .closed("))")
}

fn select_ti(tail: Sql) -> Sql {
    let mut sql = Sql::raw(format!(
        "SELECT {} FROM {TI} ti ",
        TemplateInstallation::columns_sql(Some("ti"))
    ));
    sql.append(tail);
    sql
}

/// An UPDATE of `TemplateInstallation ti` with `set` (plus `updatedAt`) where `cond`.
fn update_ti(set: Sql, cond: Sql) -> Sql {
    let mut sql = Sql::raw(format!("UPDATE {TI} AS ti SET "));
    sql.append(set);
    sql.push(", \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE ");
    sql.append(cond);
    sql
}

fn update_returning(
    conn: &Connection,
    set: Sql,
    cond: Sql,
) -> Result<Option<TemplateInstallation>> {
    let mut sql = update_ti(set, cond);
    sql.push(&format!(
        " RETURNING {}",
        TemplateInstallation::columns_sql(None)
    ));
    sql.model(conn)
}

fn update_count(conn: &Connection, set: Sql, cond: Sql) -> Result<usize> {
    update_ti(set, cond).execute(conn)
}

fn by_source(workspace_id: i64, source_template_id: i64) -> Sql {
    Sql::new(
        "ti.\"workspaceId\" = ? AND ti.\"sourceTemplateId\" = ?",
        vec![
            Value::Integer(workspace_id),
            Value::Integer(source_template_id),
        ],
    )
}

/// `ownedByClaim`.
fn owned_by_claim(id: i64, claim_token: &str) -> Sql {
    Sql::new(
        "ti.\"id\" = ? AND ti.\"status\" = 'installing' AND ti.\"claimToken\" = ?",
        vec![Value::Integer(id), Value::Text(claim_token.to_owned())],
    )
}

/// `findNewestBySourceTemplate`.
pub fn find_newest_by_source_template(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
    statuses: &[&str],
) -> Result<Option<TemplateInstallation>> {
    let mut tail = Sql::raw("WHERE ");
    tail.append(by_source(workspace_id, source_template_id));
    tail.push(" AND ti.\"status\" IN (SELECT value FROM json_each(");
    tail.push_bind(text_list(statuses));
    tail.push(")) ORDER BY ti.\"id\" DESC LIMIT 1");
    select_ti(tail).model(conn)
}

// ---------------------------------------------------------------------------
// request-repository.ts
// ---------------------------------------------------------------------------

/// `lockInstallRequest`: no-op (one SQLite writer).
pub fn lock_install_request(
    _conn: &Connection,
    _workspace_id: i64,
    _source_template_id: i64,
) -> Result<()> {
    Ok(())
}

/// `findPublisherRef`: the stored non-empty publisher ref of the pair.
pub fn find_publisher_ref(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Option<String>> {
    let mut sql = Sql::raw(format!("SELECT ti.\"publisherRef\" FROM {TI} ti WHERE "));
    sql.append(by_source(workspace_id, source_template_id));
    sql.push(" AND ti.\"publisherRef\" <> '' LIMIT 1");
    Ok(sql.query_map(conn, |row| row.get(0))?.into_iter().next())
}

/// `listInstalledByShareToken`: workspace id -> newest active install `(id, autoUpdate)`.
pub fn list_installed_by_share_token(
    conn: &Connection,
    share_token: &str,
    workspace_ids: &[i64],
) -> Result<BTreeMap<i64, (i64, bool)>> {
    if workspace_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows: Vec<(i64, i64, bool)> = Sql::new(
        format!(
            "SELECT ti.\"workspaceId\", ti.\"id\", ti.\"autoUpdate\" FROM {TI} ti JOIN \"Template\" t ON t.\"id\" = ti.\"sourceTemplateId\" WHERE t.\"shareToken\" = ? AND ti.\"workspaceId\" IN (SELECT value FROM json_each(?)) AND ti.\"status\" IN ('completed', 'partial') ORDER BY ti.\"workspaceId\", ti.\"id\" DESC"
        ),
        vec![Value::Text(share_token.to_owned()), Value::Text(id_list(workspace_ids))],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let mut out = BTreeMap::new();
    for (ws, id, auto) in rows {
        out.entry(ws).or_insert((id, auto));
    }
    Ok(out)
}

/// `findPublisherUninstallGrant`: the grant of the newest install of the pair.
pub fn find_publisher_uninstall_grant(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Option<i64>> {
    let mut sql = Sql::raw(format!(
        "SELECT ti.\"publisherUninstallGrant\" FROM {TI} ti WHERE "
    ));
    sql.append(by_source(workspace_id, source_template_id));
    sql.push(" ORDER BY ti.\"id\" DESC LIMIT 1");
    Ok(sql.query_map(conn, |row| row.get(0))?.into_iter().next())
}

/// `recordInstallEvent`.
pub fn record_install_event(conn: &Connection, installation_id: i64) -> Result<()> {
    update_count(
        conn,
        Sql::bind(
            "\"successfulInstallEvents\" = \"successfulInstallEvents\" + 1, \"completedAt\" = ",
            now_ms(),
        ),
        Sql::bind("ti.\"id\" = ", installation_id),
    )?;
    Ok(())
}

/// `markUninstalled`: an active install becomes `uninstalled`.
pub fn mark_uninstalled(
    conn: &Connection,
    id: i64,
    requested_by: &str,
    counts: &serde_json::Value,
) -> Result<bool> {
    let n = update_count(
        conn,
        Sql::new(
            "\"status\" = 'uninstalled', \"autoUpdate\" = 0, \"uninstalledAt\" = ?, \"uninstalledByKind\" = ?, \"uninstallCounts\" = ?",
            vec![
                Value::Integer(now_ms()),
                Value::Text(requested_by.to_owned()),
                patch::json(counts),
            ],
        ),
        Sql::bind(
            "ti.\"status\" IN ('completed', 'partial') AND ti.\"id\" = ",
            id,
        ),
    )?;
    Ok(n > 0)
}

/// `listActiveByPublisherRef`.
pub fn list_active_by_publisher_ref(
    conn: &Connection,
    template_id: i64,
    publisher_ref: &str,
) -> Result<Vec<TemplateInstallation>> {
    select_ti(Sql::new(
        "WHERE ti.\"templateId\" = ? AND ti.\"publisherRef\" = ? AND ti.\"status\" IN ('completed', 'partial')",
        vec![Value::Integer(template_id), Value::Text(publisher_ref.to_owned())],
    ))
    .models(conn)
}

/// `findLatestByPublisherRef` (newest `createdAt`).
pub fn find_latest_by_publisher_ref(
    conn: &Connection,
    template_id: i64,
    publisher_ref: &str,
) -> Result<Option<TemplateInstallation>> {
    select_ti(Sql::new(
        "WHERE ti.\"templateId\" = ? AND ti.\"publisherRef\" = ? ORDER BY ti.\"createdAt\" DESC LIMIT 1",
        vec![Value::Integer(template_id), Value::Text(publisher_ref.to_owned())],
    ))
    .model(conn)
}

/// `setPublisherUninstallGrant`: on every install of the pair.
pub fn set_publisher_uninstall_grant(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
    granted: bool,
) -> Result<()> {
    update_count(
        conn,
        Sql::bind("\"publisherUninstallGrant\" = ", i64::from(granted)),
        by_source(workspace_id, source_template_id),
    )?;
    Ok(())
}

/// `findLatestBySourceTemplate`.
pub fn find_latest_by_source_template(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Option<TemplateInstallation>> {
    let mut tail = Sql::raw("WHERE ");
    tail.append(by_source(workspace_id, source_template_id));
    tail.push(" ORDER BY ti.\"id\" DESC LIMIT 1");
    select_ti(tail).model(conn)
}

/// `findActiveBySourceTemplate` (`pending` / `installing`).
pub fn find_active_by_source_template(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Option<TemplateInstallation>> {
    find_newest_by_source_template(
        conn,
        workspace_id,
        source_template_id,
        &["pending", "installing"],
    )
}

/// `findInstalledBySourceTemplate` (`completed` / `partial`).
pub fn find_installed_by_source_template(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Option<TemplateInstallation>> {
    find_newest_by_source_template(
        conn,
        workspace_id,
        source_template_id,
        &["completed", "partial"],
    )
}

/// `create`: inserts the values (`id` defaults to a new id).
pub fn create(conn: &Connection, values: &Patch) -> Result<TemplateInstallation> {
    let mut all = values.clone();
    if !all.iter().any(|(c, _)| *c == "id") {
        all.insert(0, ("id", Value::Integer(new_id())));
    }
    let mut sql = Sql::raw(format!("INSERT INTO {TI} "));
    sql.append(patch::insert_clause(TemplateInstallation::COLUMNS, &all)?);
    sql.push(&format!(
        " RETURNING {}",
        TemplateInstallation::columns_sql(None)
    ));
    sql.model(conn)?
        .ok_or_else(|| Error::msg("template installation insert returned no row"))
}

/// `listPendingIds`.
pub fn list_pending_ids(conn: &Connection, workspace_id: i64) -> Result<Vec<i64>> {
    Sql::bind(
        format!("SELECT ti.\"id\" FROM {TI} ti WHERE ti.\"status\" = 'pending' AND ti.\"workspaceId\" = "),
        workspace_id,
    )
    .query_map(conn, |row| row.get(0))
}

/// `findCopyTenants`: `(templateTenantId, targetTenantId)`.
pub fn find_copy_tenants(
    conn: &Connection,
    source_template_id: i64,
    workspace_id: i64,
) -> Result<Option<(i64, i64)>> {
    Ok(Sql::new(
        "SELECT t.\"tenantId\", w.\"tenantId\" FROM \"Template\" t JOIN \"Workspace\" w ON w.\"id\" = ? WHERE t.\"id\" = ? LIMIT 1",
        vec![Value::Integer(workspace_id), Value::Integer(source_template_id)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?
    .into_iter()
    .next())
}

/// `findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<TemplateInstallation>> {
    select_ti(Sql::new(
        "WHERE ti.\"id\" = ? AND ti.\"workspaceId\" = ? LIMIT 1",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    ))
    .model(conn)
}

/// `listByIdsAndWorkspace`.
pub fn list_by_ids_and_workspace(
    conn: &Connection,
    ids: &[i64],
    workspace_id: i64,
) -> Result<Vec<TemplateInstallation>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    select_ti(Sql::new(
        "WHERE ti.\"id\" IN (SELECT value FROM json_each(?)) AND ti.\"workspaceId\" = ?",
        vec![Value::Text(id_list(ids)), Value::Integer(workspace_id)],
    ))
    .models(conn)
}

/// `resetFailed` input.
#[derive(Debug, Clone)]
pub struct ResetFailed {
    pub id: i64,
    pub workspace_id: i64,
    pub installed_by: Option<i64>,
    pub installed_by_kind: String,
    pub source_version: Option<i64>,
}

/// `resetFailed`: a failed install goes back to `pending` with a new staging generation.
pub fn reset_failed(
    conn: &Connection,
    input: &ResetFailed,
) -> Result<Option<TemplateInstallation>> {
    let now = now_ms();
    let mut set = Sql::default();
    if let Some(v) = input.source_version {
        set.push("\"sourceVersion\" = ");
        set.push_bind(v);
        set.push(", ");
    }
    set.append(Sql::new(
        "\"status\" = 'pending', \"claimToken\" = NULL, \"claimedAt\" = NULL, \"errorCode\" = NULL, \"errorMessage\" = NULL, \"completedAt\" = NULL, \"stagingGeneration\" = \"stagingGeneration\" + 1, \"stagedCleanupClaimedAt\" = NULL, \"installedBy\" = ?, \"installedByKind\" = ?, \"createdAt\" = ?",
        vec![
            opt_int(input.installed_by),
            Value::Text(input.installed_by_kind.clone()),
            Value::Integer(now),
        ],
    ));
    update_returning(
        conn,
        set,
        Sql::new(
            "ti.\"id\" = ? AND ti.\"workspaceId\" = ? AND ti.\"status\" = 'failed'",
            vec![Value::Integer(input.id), Value::Integer(input.workspace_id)],
        ),
    )
}

/// `markSourceDeleted`: every install of the template not marked yet.
pub fn mark_source_deleted(conn: &Connection, source_template_id: i64) -> Result<()> {
    update_count(
        conn,
        Sql::bind("\"sourceDeletedAt\" = ", now_ms()),
        Sql::bind(
            "ti.\"sourceDeletedAt\" IS NULL AND ti.\"sourceTemplateId\" = ",
            source_template_id,
        ),
    )?;
    Ok(())
}

/// `listPage` input.
#[derive(Debug, Clone, Default)]
pub struct ListPage {
    pub workspace_id: i64,
    pub status: Option<String>,
    pub active_only: bool,
    pub before_id: Option<i64>,
    pub limit: i64,
    pub visible_categories: Option<Vec<String>>,
}

/// `TemplateInstallationPageRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateInstallationPageRow {
    pub id: i64,
    pub template_name: String,
    pub status: String,
    pub error_code: Option<String>,
    pub resource_count: i64,
    pub warning_count: i64,
    pub installer_warning_count: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub auto_update: bool,
    pub install_source: String,
    pub active_sync: bool,
    pub uninstalled_by_kind: Option<String>,
    pub uninstall_state: String,
    pub uninstall_counts: Option<serde_json::Value>,
    pub update_available: bool,
    pub source_deleted: bool,
}

/// `listPage`: newest first, `limit + 1` rows (the caller cuts the extra row).
pub fn list_page(conn: &Connection, input: &ListPage) -> Result<Vec<TemplateInstallationPageRow>> {
    let mut sql = Sql::raw(
        "SELECT ti.\"id\", ti.\"templateName\", ti.\"status\", ti.\"errorCode\", ti.\"resourceCount\", ti.\"warningCount\", ",
    );
    sql.append(installer_warning_count());
    sql.push(&format!(
        ", ti.\"createdAt\", ti.\"completedAt\", ti.\"autoUpdate\", ti.\"installSource\", {HAS_ACTIVE_SYNC}, ti.\"uninstalledByKind\", ti.\"uninstallState\", ti.\"uninstallCounts\", COALESCE(ti.\"sourceDeletedAt\" IS NULL AND ti.\"sourceVersion\" < t.\"version\", 0), (ti.\"sourceDeletedAt\" IS NOT NULL OR ti.\"templateId\" IS NULL) FROM {TI} ti LEFT JOIN \"Template\" t ON t.\"id\" = ti.\"templateId\" WHERE ti.\"workspaceId\" = "
    ));
    sql.push_bind(input.workspace_id);
    if let Some(status) = &input.status {
        sql.push(" AND ti.\"status\" = ");
        sql.push_bind(status.clone());
    }
    if input.active_only {
        sql.push(&format!(
            " AND (ti.\"status\" IN ('pending', 'installing') OR {HAS_ACTIVE_SYNC})"
        ));
    }
    if let Some(before) = input.before_id {
        sql.push(" AND ti.\"id\" < ");
        sql.push_bind(before);
    }
    if let Some(visible) = &input.visible_categories {
        sql.push(" AND NOT ");
        sql.append(holds_category_outside(visible));
    }
    sql.push(" ORDER BY ti.\"id\" DESC LIMIT ");
    sql.push_bind(input.limit + 1);
    sql.query_map(conn, |row| {
        let counts: Option<Json> = row.get(14)?;
        Ok(TemplateInstallationPageRow {
            id: row.get(0)?,
            template_name: row.get(1)?,
            status: row.get(2)?,
            error_code: row.get(3)?,
            resource_count: row.get(4)?,
            warning_count: row.get(5)?,
            installer_warning_count: row.get(6)?,
            created_at: row.get(7)?,
            completed_at: row.get(8)?,
            auto_update: row.get(9)?,
            install_source: row.get(10)?,
            active_sync: row.get(11)?,
            uninstalled_by_kind: row.get(12)?,
            uninstall_state: row.get(13)?,
            uninstall_counts: counts.and_then(|c| parse_template_uninstall_counts(&c.0)),
            update_available: row.get(15)?,
            source_deleted: row.get(16)?,
        })
    })
}

// ---------------------------------------------------------------------------
// lease-repository.ts
// ---------------------------------------------------------------------------

/// `claim`: a pending (or lease-expired installing) install becomes `installing`.
pub fn claim(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    claim_token: &str,
    lease_expired_before: i64,
) -> Result<Option<TemplateInstallation>> {
    update_returning(
        conn,
        Sql::new(
            "\"status\" = 'installing', \"claimToken\" = ?, \"claimedAt\" = ?, \"attempts\" = \"attempts\" + 1",
            vec![
                Value::Text(claim_token.to_owned()),
                Value::Integer(now_ms()),
            ],
        ),
        Sql::new(
            "ti.\"id\" = ? AND ti.\"workspaceId\" = ? AND (ti.\"status\" = 'pending' OR (ti.\"status\" = 'installing' AND ti.\"claimedAt\" < ?))",
            vec![
                Value::Integer(id),
                Value::Integer(workspace_id),
                Value::Integer(lease_expired_before),
            ],
        ),
    )
}

/// `release`: the claim holder gives the install back to `pending`.
pub fn release(conn: &Connection, id: i64, claim_token: &str) -> Result<bool> {
    Ok(update_count(
        conn,
        Sql::raw("\"status\" = 'pending', \"claimToken\" = NULL, \"claimedAt\" = NULL"),
        owned_by_claim(id, claim_token),
    )? > 0)
}

/// `complete` completion columns.
#[derive(Debug, Clone)]
pub struct Completion {
    pub status: String,
    pub warnings: Option<serde_json::Value>,
    pub warning_count: Option<i64>,
    pub resource_count: Option<i64>,
    /// `None` leaves the column; `Some(None)` clears it.
    pub install_folder_id: Option<Option<i64>>,
    pub pending_publish_effects: Option<Option<serde_json::Value>>,
}

/// `complete`: the claim holder finishes the install.
pub fn complete(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    completion: &Completion,
) -> Result<bool> {
    let mut set = Sql::bind("\"status\" = ", completion.status.clone());
    if let Some(w) = &completion.warnings {
        set.push(", \"warnings\" = ");
        set.push_bind(patch::json(w));
    }
    if let Some(n) = completion.warning_count {
        set.push(", \"warningCount\" = ");
        set.push_bind(n);
    }
    if let Some(n) = completion.resource_count {
        set.push(", \"resourceCount\" = ");
        set.push_bind(n);
    }
    if let Some(f) = completion.install_folder_id {
        set.push(", \"installFolderId\" = ");
        set.push_bind(opt_int(f));
    }
    if let Some(e) = &completion.pending_publish_effects {
        set.push(", \"pendingPublishEffects\" = ");
        set.push_bind(e.as_ref().map_or(Value::Null, patch::json));
    }
    set.push(", \"claimToken\" = NULL, \"errorCode\" = NULL, \"errorMessage\" = NULL, \"successfulInstallEvents\" = 1, \"completedAt\" = ");
    set.push_bind(now_ms());
    Ok(update_count(conn, set, owned_by_claim(id, claim_token))? > 0)
}

fn fail_set(error_code: &str, error_message: &str) -> Sql {
    Sql::new(
        "\"status\" = 'failed', \"errorCode\" = ?, \"errorMessage\" = ?, \"claimToken\" = NULL, \"completedAt\" = ?",
        vec![
            Value::Text(error_code.to_owned()),
            Value::Text(error_message.to_owned()),
            Value::Integer(now_ms()),
        ],
    )
}

/// `fail`: the claim holder fails the install.
pub fn fail(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    error_code: &str,
    error_message: &str,
) -> Result<bool> {
    Ok(update_count(
        conn,
        fail_set(error_code, error_message),
        owned_by_claim(id, claim_token),
    )? > 0)
}

/// `failUnclaimed`: fails a pending or lease-expired install.
pub fn fail_unclaimed(
    conn: &Connection,
    id: i64,
    error_code: &str,
    error_message: &str,
    lease_expired_before: i64,
) -> Result<bool> {
    Ok(update_count(
        conn,
        fail_set(error_code, error_message),
        Sql::new(
            "ti.\"id\" = ? AND (ti.\"status\" = 'pending' OR (ti.\"status\" = 'installing' AND ti.\"claimedAt\" < ?))",
            vec![Value::Integer(id), Value::Integer(lease_expired_before)],
        ),
    )? > 0)
}

/// `listStale`: `(id, workspaceId)` of unfinished installs, oldest first.
pub fn list_stale(
    conn: &Connection,
    pending_before: i64,
    lease_expired_before: i64,
    created_after: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let mut sql = Sql::bind(
        format!("SELECT ti.\"id\", ti.\"workspaceId\" FROM {TI} ti WHERE ti.\"createdAt\" > "),
        created_after,
    );
    sql.push(" AND ");
    sql.append(unfinished_installation(
        pending_before,
        lease_expired_before,
    ));
    sql.push(" ORDER BY ti.\"createdAt\" ASC LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// `failAbandoned`: fails unfinished installs created before `created_before`.
pub fn fail_abandoned(
    conn: &Connection,
    created_before: i64,
    pending_before: i64,
    lease_expired_before: i64,
    error_code: &str,
    error_message: &str,
) -> Result<usize> {
    let mut cond = Sql::bind("ti.\"createdAt\" < ", created_before);
    cond.push(" AND ");
    cond.append(unfinished_installation(
        pending_before,
        lease_expired_before,
    ));
    update_count(conn, fail_set(error_code, error_message), cond)
}

// ---------------------------------------------------------------------------
// staged-cleanup-repository.ts
// ---------------------------------------------------------------------------

fn cleanup_claimable(completed_before: i64, lease_expired_before: i64) -> Sql {
    let mut sql = staged_cleanup_due(completed_before);
    sql.push(" AND (ti.\"stagedCleanupClaimedAt\" IS NULL OR ti.\"stagedCleanupClaimedAt\" < ");
    sql.push_bind(lease_expired_before);
    sql.push(")");
    sql
}

/// `listWithStagedObjectsToClean`: `(id, workspaceId)`, least recently updated first.
pub fn list_with_staged_objects_to_clean(
    conn: &Connection,
    completed_before: i64,
    lease_expired_before: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let mut sql = Sql::raw(format!(
        "SELECT ti.\"id\", ti.\"workspaceId\" FROM {TI} ti WHERE "
    ));
    sql.append(cleanup_claimable(completed_before, lease_expired_before));
    sql.push(" ORDER BY ti.\"updatedAt\" ASC LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// A staged-cleanup claim.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedCleanupClaim {
    pub workspace_id: i64,
    pub from: i64,
    pub to: i64,
    pub claimed_at: i64,
}

/// `claimStagedObjectsCleanup`.
pub fn claim_staged_objects_cleanup(
    conn: &Connection,
    id: i64,
    completed_before: i64,
    lease_expired_before: i64,
) -> Result<Option<StagedCleanupClaim>> {
    let claimed_at = now_ms();
    let mut cond = Sql::bind("ti.\"id\" = ", id);
    cond.push(" AND ");
    cond.append(cleanup_claimable(completed_before, lease_expired_before));
    let mut sql = update_ti(Sql::bind("\"stagedCleanupClaimedAt\" = ", claimed_at), cond);
    sql.push(" RETURNING \"workspaceId\", \"status\", \"stagingGeneration\", \"stagedCleanedGeneration\"");
    let rows: Vec<(i64, String, i64, i64)> = sql.query_map(conn, |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    Ok(rows.into_iter().next().map(
        |(workspace_id, status, staging, cleaned)| StagedCleanupClaim {
            workspace_id,
            claimed_at,
            from: cleaned + 1,
            to: if status == "failed" {
                staging
            } else {
                staging - 1
            },
        },
    ))
}

/// `advanceStagedCleanedGeneration`.
pub fn advance_staged_cleaned_generation(
    conn: &Connection,
    id: i64,
    from: i64,
    to: i64,
) -> Result<()> {
    update_count(
        conn,
        Sql::bind("\"stagedCleanedGeneration\" = ", to),
        Sql::new(
            "ti.\"id\" = ? AND ti.\"stagedCleanedGeneration\" = ?",
            vec![Value::Integer(id), Value::Integer(from - 1)],
        ),
    )?;
    Ok(())
}

/// `releaseStagedObjectsCleanup`.
pub fn release_staged_objects_cleanup(conn: &Connection, id: i64, claimed_at: i64) -> Result<()> {
    update_count(
        conn,
        Sql::raw("\"stagedCleanupClaimedAt\" = NULL"),
        Sql::new(
            "ti.\"id\" = ? AND ti.\"stagedCleanupClaimedAt\" = ?",
            vec![Value::Integer(id), Value::Integer(claimed_at)],
        ),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// publish-effects-repository.ts
// ---------------------------------------------------------------------------

/// A publish effect owed after an install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishEffect {
    pub flow_id: i64,
    pub version_id: i64,
}

fn read_effects(conn: &Connection, id: i64) -> Result<Option<Vec<PublishEffect>>> {
    let rows: Vec<Option<String>> = Sql::bind(
        format!("SELECT ti.\"pendingPublishEffects\" FROM {TI} ti WHERE ti.\"id\" = "),
        id,
    )
    .query_map(conn, |row| row.get(0))?;
    match rows.into_iter().next().flatten() {
        Some(text) => Ok(Some(
            serde_json::from_str(&text).map_err(|e| Error::msg(e.to_string()))?,
        )),
        None => Ok(None),
    }
}

/// `addPendingPublishEffects`: appends to the list. Run it in one write job.
pub fn add_pending_publish_effects(
    conn: &Connection,
    id: i64,
    effects: &[PublishEffect],
) -> Result<()> {
    if effects.is_empty() {
        return Ok(());
    }
    let mut all = read_effects(conn, id)?.unwrap_or_default();
    all.extend(effects.iter().cloned());
    update_count(
        conn,
        Sql::bind("\"pendingPublishEffects\" = ", json_text(&all)?),
        Sql::bind("ti.\"id\" = ", id),
    )?;
    Ok(())
}

/// `findPendingPublishEffects`.
pub fn find_pending_publish_effects(conn: &Connection, id: i64) -> Result<Vec<PublishEffect>> {
    Ok(read_effects(conn, id)?.unwrap_or_default())
}

/// `clearPendingPublishEffects`: drops the done effects (an empty rest stores `NULL`).
/// Run it in one write job.
pub fn clear_pending_publish_effects(
    conn: &Connection,
    id: i64,
    done: &[PublishEffect],
) -> Result<()> {
    if done.is_empty() {
        return Ok(());
    }
    let Some(current) = read_effects(conn, id)? else {
        return Ok(());
    };
    let rest: Vec<&PublishEffect> = current.iter().filter(|e| !done.contains(e)).collect();
    let value = if rest.is_empty() {
        Value::Null
    } else {
        Value::Text(json_text(&rest)?)
    };
    update_count(
        conn,
        Sql::bind("\"pendingPublishEffects\" = ", value),
        Sql::bind("ti.\"id\" = ", id),
    )?;
    Ok(())
}

/// `deferPendingPublishEffects`: bumps `updatedAt` while effects are owed.
pub fn defer_pending_publish_effects(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE \"TemplateInstallation\" SET \"updatedAt\" = ?1 WHERE \"id\" = ?2 AND \"pendingPublishEffects\" IS NOT NULL",
        (now_ms(), id),
    )?;
    Ok(())
}

/// `listWithPendingPublishEffects`: `(id, workspaceId)`, least recently updated first.
pub fn list_with_pending_publish_effects(
    conn: &Connection,
    settled_before: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    Sql::new(
        format!(
            "SELECT ti.\"id\", ti.\"workspaceId\" FROM {TI} ti WHERE ti.\"pendingPublishEffects\" IS NOT NULL AND ti.\"updatedAt\" < ? ORDER BY ti.\"updatedAt\" ASC LIMIT ?"
        ),
        vec![Value::Integer(settled_before), Value::Integer(limit)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}
