//! Port of `repositories/template-installation-sync/repository.ts`
//! (`templateInstallationSyncRepository`). Every UPDATE sets `"updatedAt"` (Drizzle
//! `$onUpdate`). The installed-resource readers / writers have a ported test; the sync job
//! functions (no TS tests) have the smoke test `tests/repo_p33_smoke.rs`. `now()` /
//! `new Date()` = `now_ms()`; `GREATEST` = `max`; an empty `permanentErrorCodes` list
//! matches no failed sync (Postgres would reject `IN ()`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{TemplateInstallationSync, TemplateInstalledResource};
use super::sql::{Sql, opt_int, opt_text};
use super::template_installed_resource::keep_known_resource_kinds;
use super::{Model, Result, id_list, now_ms, text_list};

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

const SYNC: &str = "\"TemplateInstallationSync\"";

/// `settlesAutoUpdate(permanentErrorCodes)` over the sync alias `s`.
fn settles_auto_update(permanent_error_codes: &[String]) -> Sql {
    Sql::bind(
        "(s.\"status\" IN ('pending', 'running', 'completed', 'partial', 'skipped') OR (s.\"status\" = 'failed' AND s.\"errorCode\" IN (SELECT value FROM json_each(",
        text_list(permanent_error_codes),
    )
    .closed("))))")
}

/// `NewTemplateSync`.
#[derive(Debug, Clone, Default)]
pub struct NewTemplateSync {
    pub id: i64,
    pub workspace_id: i64,
    pub installation_id: i64,
    pub trigger: String,
    pub requested_version: i64,
    pub requested_by: Option<i64>,
    /// `None` = `apiToken`.
    pub requested_by_kind: Option<String>,
    pub idempotency_key: Option<String>,
    pub request_fingerprint: Option<String>,
}

/// `createManyIfAbsent`: inserts the syncs, skipping conflicts; returns the inserted rows.
pub fn create_many_if_absent(
    conn: &Connection,
    values: &[NewTemplateSync],
) -> Result<Vec<TemplateInstallationSync>> {
    let mut out = Vec::new();
    for v in values {
        let mut sql = Sql::new(
            format!(
                "INSERT INTO {SYNC} (\"id\", \"workspaceId\", \"installationId\", \"trigger\", \"requestedVersion\", \"requestedBy\", \"requestedByKind\", \"idempotencyKey\", \"requestFingerprint\") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING"
            ),
            vec![
                Value::Integer(v.id),
                Value::Integer(v.workspace_id),
                Value::Integer(v.installation_id),
                Value::Text(v.trigger.clone()),
                Value::Integer(v.requested_version),
                opt_int(v.requested_by),
                Value::Text(
                    v.requested_by_kind
                        .clone()
                        .unwrap_or_else(|| "apiToken".to_owned()),
                ),
                opt_text(v.idempotency_key.as_deref()),
                opt_text(v.request_fingerprint.as_deref()),
            ],
        );
        sql.push(&format!(
            " RETURNING {}",
            TemplateInstallationSync::columns_sql(None)
        ));
        out.extend(sql.models::<TemplateInstallationSync>(conn)?);
    }
    Ok(out)
}

/// `createIfAbsent`.
pub fn create_if_absent(
    conn: &Connection,
    value: &NewTemplateSync,
) -> Result<Option<TemplateInstallationSync>> {
    Ok(create_many_if_absent(conn, std::slice::from_ref(value))?
        .into_iter()
        .next())
}

fn sync_where(tail: Sql) -> Sql {
    let mut sql = Sql::raw(format!(
        "SELECT {} FROM {SYNC} s WHERE ",
        TemplateInstallationSync::columns_sql(Some("s"))
    ));
    sql.append(tail);
    sql
}

/// `findById`.
pub fn find_by_id(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<TemplateInstallationSync>> {
    sync_where(Sql::new(
        "s.\"id\" = ? AND s.\"workspaceId\" = ? LIMIT 1",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    ))
    .model(conn)
}

/// `findByIdempotencyKey`.
pub fn find_by_idempotency_key(
    conn: &Connection,
    installation_id: i64,
    key: &str,
) -> Result<Option<TemplateInstallationSync>> {
    sync_where(Sql::new(
        "s.\"installationId\" = ? AND s.\"idempotencyKey\" = ? LIMIT 1",
        vec![Value::Integer(installation_id), Value::Text(key.to_owned())],
    ))
    .model(conn)
}

fn update_sync(conn: &Connection, set: Sql, cond: Sql) -> Result<usize> {
    let mut sql = Sql::raw(format!("UPDATE {SYNC} AS s SET "));
    sql.append(set);
    sql.push(", \"updatedAt\" = ");
    sql.push_bind(now_ms());
    sql.push(" WHERE ");
    sql.append(cond);
    sql.execute(conn)
}

/// `adoptIdempotencyKey`: sets the key on a sync without one.
pub fn adopt_idempotency_key(
    conn: &Connection,
    id: i64,
    key: &str,
    request_fingerprint: Option<&str>,
) -> Result<bool> {
    Ok(update_sync(
        conn,
        Sql::new(
            "\"idempotencyKey\" = ?, \"requestFingerprint\" = ?",
            vec![Value::Text(key.to_owned()), opt_text(request_fingerprint)],
        ),
        Sql::bind("s.\"idempotencyKey\" IS NULL AND s.\"id\" = ", id),
    )? > 0)
}

/// `findActiveByInstallation` (`pending` / `running`).
pub fn find_active_by_installation(
    conn: &Connection,
    installation_id: i64,
) -> Result<Option<TemplateInstallationSync>> {
    sync_where(Sql::bind(
        "s.\"status\" IN ('pending', 'running') AND s.\"installationId\" = ",
        installation_id,
    ))
    .closed(" LIMIT 1")
    .model(conn)
}

/// `listByInstallation`: newest first.
pub fn list_by_installation(
    conn: &Connection,
    installation_id: i64,
    workspace_id: i64,
    limit: i64,
) -> Result<Vec<TemplateInstallationSync>> {
    sync_where(Sql::new(
        "s.\"installationId\" = ? AND s.\"workspaceId\" = ? ORDER BY s.\"id\" DESC LIMIT ?",
        vec![
            Value::Integer(installation_id),
            Value::Integer(workspace_id),
            Value::Integer(limit),
        ],
    ))
    .models(conn)
}

/// `claim`: a pending (or lease-expired running) sync becomes `running`.
pub fn claim(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    claim_token: &str,
    lease_expired_before: i64,
) -> Result<Option<TemplateInstallationSync>> {
    let now = now_ms();
    let mut sql = Sql::new(
        format!(
            "UPDATE {SYNC} AS s SET \"status\" = 'running', \"claimToken\" = ?, \"claimedAt\" = ?, \"startedAt\" = COALESCE(\"startedAt\", ?), \"attempts\" = \"attempts\" + 1, \"updatedAt\" = ? WHERE s.\"id\" = ? AND s.\"workspaceId\" = ? AND (s.\"status\" = 'pending' OR (s.\"status\" = 'running' AND s.\"claimedAt\" < ?))"
        ),
        vec![
            Value::Text(claim_token.to_owned()),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(now),
            Value::Integer(id),
            Value::Integer(workspace_id),
            Value::Integer(lease_expired_before),
        ],
    );
    sql.push(&format!(
        " RETURNING {}",
        TemplateInstallationSync::columns_sql(None)
    ));
    sql.model(conn)
}

fn owned_by_claim(id: i64, claim_token: &str) -> Sql {
    Sql::new(
        "s.\"id\" = ? AND s.\"status\" = 'running' AND s.\"claimToken\" = ?",
        vec![Value::Integer(id), Value::Text(claim_token.to_owned())],
    )
}

/// `release`.
pub fn release(conn: &Connection, id: i64, claim_token: &str) -> Result<()> {
    update_sync(
        conn,
        Sql::raw("\"status\" = 'pending', \"claimToken\" = NULL, \"claimedAt\" = NULL"),
        owned_by_claim(id, claim_token),
    )?;
    Ok(())
}

/// `TemplateSyncOutcome`.
#[derive(Debug, Clone, Default)]
pub struct TemplateSyncOutcome {
    pub status: String,
    pub error_code: Option<String>,
    pub from_version: Option<i64>,
    pub applied_version: Option<i64>,
    pub applied_hash: Option<String>,
    pub counts: Option<serde_json::Value>,
    /// `None` = `[]`.
    pub warnings: Option<serde_json::Value>,
    pub warning_summary: Option<serde_json::Value>,
}

/// `finish`: the claim holder records the outcome.
pub fn finish(
    conn: &Connection,
    id: i64,
    claim_token: &str,
    outcome: &TemplateSyncOutcome,
) -> Result<bool> {
    let json_or_null = |v: &Option<serde_json::Value>| {
        v.as_ref()
            .map_or(Value::Null, |v| Value::Text(v.to_string()))
    };
    Ok(update_sync(
        conn,
        Sql::new(
            "\"status\" = ?, \"errorCode\" = ?, \"fromVersion\" = ?, \"appliedVersion\" = ?, \"appliedHash\" = ?, \"counts\" = ?, \"warnings\" = ?, \"warningSummary\" = ?, \"finishedAt\" = ?",
            vec![
                Value::Text(outcome.status.clone()),
                opt_text(outcome.error_code.as_deref()),
                opt_int(outcome.from_version),
                opt_int(outcome.applied_version),
                opt_text(outcome.applied_hash.as_deref()),
                json_or_null(&outcome.counts),
                Value::Text(
                    outcome
                        .warnings
                        .as_ref()
                        .map_or_else(|| "[]".to_owned(), |w| w.to_string()),
                ),
                json_or_null(&outcome.warning_summary),
                Value::Integer(now_ms()),
            ],
        ),
        owned_by_claim(id, claim_token),
    )? > 0)
}

/// `findStatusFinishedByClaim`: the status of a finished sync of this claim.
pub fn find_status_finished_by_claim(
    conn: &Connection,
    id: i64,
    claim_token: &str,
) -> Result<Option<String>> {
    Ok(Sql::new(
        format!(
            "SELECT s.\"status\" FROM {SYNC} s WHERE s.\"id\" = ? AND s.\"claimToken\" = ? AND s.\"status\" NOT IN ('pending', 'running') LIMIT 1"
        ),
        vec![Value::Integer(id), Value::Text(claim_token.to_owned())],
    )
    .query_map(conn, |row| row.get(0))?
    .into_iter()
    .next())
}

/// `closeUnclaimed`: closes a pending (or, with a lease bound, lease-expired) sync.
pub fn close_unclaimed(
    conn: &Connection,
    id: i64,
    status: &str,
    error_code: &str,
    lease_expired_before: Option<i64>,
) -> Result<()> {
    let mut cond = Sql::bind("s.\"id\" = ", id);
    cond.push(" AND (s.\"status\" = 'pending'");
    if let Some(before) = lease_expired_before {
        cond.push(" OR (s.\"status\" = 'running' AND s.\"claimedAt\" < ");
        cond.push_bind(before);
        cond.push(")");
    }
    cond.push(")");
    update_sync(
        conn,
        Sql::new(
            "\"status\" = ?, \"errorCode\" = ?, \"finishedAt\" = ?, \"claimToken\" = NULL",
            vec![
                Value::Text(status.to_owned()),
                Value::Text(error_code.to_owned()),
                Value::Integer(now_ms()),
            ],
        ),
        cond,
    )?;
    Ok(())
}

/// `hasRunningSync`.
pub fn has_running_sync(conn: &Connection, installation_id: i64) -> Result<bool> {
    Ok(!Sql::bind(
        format!(
            "SELECT 1 FROM {SYNC} s WHERE s.\"status\" = 'running' AND s.\"installationId\" = "
        ),
        installation_id,
    )
    .closed(" LIMIT 1")
    .query_map(conn, |row| row.get::<_, i64>(0))?
    .is_empty())
}

/// `cancelPendingSyncs`: the installation was uninstalled.
pub fn cancel_pending_syncs(conn: &Connection, installation_id: i64) -> Result<()> {
    update_sync(
        conn,
        Sql::bind(
            "\"status\" = 'cancelled', \"errorCode\" = 'templateUninstalled', \"claimToken\" = NULL, \"finishedAt\" = ",
            now_ms(),
        ),
        Sql::bind(
            "s.\"status\" = 'pending' AND s.\"installationId\" = ",
            installation_id,
        ),
    )?;
    Ok(())
}

/// `listUnfinished`: `(id, workspaceId)` of syncs to recover.
pub fn list_unfinished(
    conn: &Connection,
    pending_before: i64,
    lease_expired_before: i64,
    created_after: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    Sql::new(
        format!(
            "SELECT s.\"id\", s.\"workspaceId\" FROM {SYNC} s WHERE s.\"createdAt\" > ? AND ((s.\"status\" = 'pending' AND s.\"createdAt\" < ?) OR (s.\"status\" = 'running' AND s.\"claimedAt\" < ?)) LIMIT ?"
        ),
        vec![
            Value::Integer(created_after),
            Value::Integer(pending_before),
            Value::Integer(lease_expired_before),
            Value::Integer(limit),
        ],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))
}

/// An auto-update candidate installation.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoUpdateCandidate {
    pub id: i64,
    pub workspace_id: i64,
    pub installed_by_kind: String,
}

/// `listAutoUpdateCandidates`: by id.
pub fn list_auto_update_candidates(
    conn: &Connection,
    template_id: i64,
    version: i64,
    after_id: Option<i64>,
    limit: i64,
    permanent_error_codes: &[String],
) -> Result<Vec<AutoUpdateCandidate>> {
    let mut sql = Sql::new(
        "SELECT ti.\"id\", ti.\"workspaceId\", ti.\"installedByKind\" FROM \"TemplateInstallation\" ti WHERE ti.\"sourceTemplateId\" = ? AND ti.\"autoUpdate\" = 1 AND ti.\"status\" IN ('completed', 'partial') AND ti.\"sourceDeletedAt\" IS NULL AND ti.\"sourceVersion\" < ? AND NOT EXISTS (SELECT 1 FROM \"TemplateInstallationSync\" s WHERE s.\"installationId\" = ti.\"id\" AND s.\"trigger\" = 'autoUpdate' AND s.\"requestedVersion\" = ? AND ",
        vec![
            Value::Integer(template_id),
            Value::Integer(version),
            Value::Integer(version),
        ],
    );
    sql.append(settles_auto_update(permanent_error_codes));
    sql.push(") AND NOT EXISTS (SELECT 1 FROM \"TemplateInstallationSync\" a WHERE a.\"installationId\" = ti.\"id\" AND a.\"status\" IN ('pending', 'running'))");
    if let Some(after) = after_id {
        sql.push(" AND ti.\"id\" > ");
        sql.push_bind(after);
    }
    sql.push(" ORDER BY ti.\"id\" LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| {
        Ok(AutoUpdateCandidate {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            installed_by_kind: row.get(2)?,
        })
    })
}

/// `completeInstallationSync`: `sourceVersion = max(sourceVersion, version)`.
pub fn complete_installation_sync(
    conn: &Connection,
    installation_id: i64,
    version: i64,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE \"TemplateInstallation\" SET \"sourceVersion\" = max(\"sourceVersion\", ?1), \"lastSyncedAt\" = ?2, \"updatedAt\" = ?2 WHERE \"id\" = ?3",
        (version, now, installation_id),
    )?;
    Ok(())
}

/// A template with installs behind its version: `(templateId, workspaceId, version)`.
pub fn list_templates_behind(
    conn: &Connection,
    limit: i64,
    permanent_error_codes: &[String],
) -> Result<Vec<(i64, i64, i64)>> {
    let mut sql = Sql::raw(
        "SELECT t.\"id\", t.\"workspaceId\", t.\"version\" FROM \"Template\" t WHERE t.\"deletedAt\" IS NULL AND EXISTS (SELECT 1 FROM \"TemplateInstallation\" ti WHERE ti.\"sourceTemplateId\" = t.\"id\" AND ti.\"autoUpdate\" = 1 AND ti.\"status\" IN ('completed', 'partial') AND ti.\"sourceDeletedAt\" IS NULL AND ti.\"sourceVersion\" < t.\"version\" AND NOT EXISTS (SELECT 1 FROM \"TemplateInstallationSync\" s WHERE s.\"installationId\" = ti.\"id\" AND s.\"trigger\" = 'autoUpdate' AND s.\"requestedVersion\" = t.\"version\" AND ",
    );
    sql.append(settles_auto_update(permanent_error_codes));
    sql.push(")) ORDER BY random() LIMIT ");
    sql.push_bind(limit);
    sql.query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
}

/// `failAbandoned`: fails active syncs created before `created_before`.
pub fn fail_abandoned(conn: &Connection, created_before: i64, error_code: &str) -> Result<usize> {
    update_sync(
        conn,
        Sql::new(
            "\"status\" = 'failed', \"errorCode\" = ?, \"finishedAt\" = ?, \"claimToken\" = NULL",
            vec![Value::Text(error_code.to_owned()), Value::Integer(now_ms())],
        ),
        Sql::bind(
            "s.\"status\" IN ('pending', 'running') AND s.\"createdAt\" < ",
            created_before,
        ),
    )
}

/// `listInstalledCategoriesByPair`: distinct categories of live tracked resources.
pub fn list_installed_categories_by_pair(
    conn: &Connection,
    workspace_id: i64,
    source_template_id: i64,
) -> Result<Vec<String>> {
    Sql::new(
        "SELECT DISTINCT r.\"category\" FROM \"TemplateInstalledResource\" r JOIN \"TemplateInstallation\" ti ON ti.\"id\" = r.\"installationId\" WHERE ti.\"workspaceId\" = ? AND ti.\"sourceTemplateId\" = ? AND ti.\"status\" IN ('completed', 'partial') AND r.\"detachedAt\" IS NULL",
        vec![Value::Integer(workspace_id), Value::Integer(source_template_id)],
    )
    .query_map(conn, |row| row.get(0))
}
