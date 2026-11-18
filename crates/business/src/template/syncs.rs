//! `sync-request.service.ts` (`request`, `fanOutAutoUpdate`),
//! `sync-execute.service.ts` (the sync job), `sync-restore.service.ts` and the
//! worker handlers `syncTemplateInstallation` / `fanOutTemplateUpdate`
//! (Q.D4 slice 4). One SQLite write transaction replaces the install-request
//! advisory lock and the resource locks.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{TemplateInstallation, TemplateInstallationSync};
use chatbotx_db::repo::template::ACTIVE_INSTALLATION_STATUSES;
use chatbotx_db::repo::template_installation as installs;
use chatbotx_db::repo::template_installation_sync as syncs;
use chatbotx_db::repo::template_resource_backup as backups;
use chatbotx_db::repo::template_uninstall;
use chatbotx_db::repo::{new_id, now_ms, template as templates, template_version};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _};
use chatbotx_jobs::Enqueuer;
use serde_json::{Value, json};

use super::install::sync::{SyncInput, apply_version, has_lagging_updatable_rows, restore};
use super::install::{Ctx, db_err};
use super::permissions::{Actor, assert_can_use_categories, assert_can_write_installs};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `SYNC_LEASE_MS`.
pub const SYNC_LEASE_MS: i64 = 15 * 60 * 1000;
/// `TEMPLATE_FAN_OUT_BATCH_SIZE`.
pub const FAN_OUT_BATCH_SIZE: i64 = 500;
/// `PERMANENT_TEMPLATE_ERROR_CODES`.
const PERMANENT_ERROR_CODES: [&str; 8] = [
    "templatePayloadIncompatible",
    "templateInstallationSourceMissing",
    "templateCrossTenantInstall",
    "templateInstallIntoSource",
    "templateCategoryPermissionDenied",
    "templateInvalidInboxBinding",
    "templateUninstallNotGranted",
    "templateUninstallNoHandler",
];

fn not_syncable() -> BusinessError {
    BusinessError::exception(
        "This template installation can not be synced",
        "templateInstallationNotSyncable",
        409,
    )
}

fn sync_in_progress() -> BusinessError {
    BusinessError::exception(
        "A sync of this installation is in progress",
        "templateSyncInProgress",
        409,
    )
}

fn uninstall_in_progress() -> BusinessError {
    BusinessError::exception(
        "This template is being uninstalled",
        "templateUninstallInProgress",
        409,
    )
}

fn is_active(row: &TemplateInstallation) -> bool {
    ACTIVE_INSTALLATION_STATUSES.contains(&row.status.as_str())
}

/// The categories of a template's `categoryCounts` with a non-zero count.
fn used_categories(counts: &Value) -> Vec<String> {
    counts
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, n)| n.as_u64().unwrap_or(0) > 0)
        .map(|(k, _)| k.clone())
        .collect()
}

/// A sync request (`RequestSyncInput`).
pub struct SyncRequest {
    pub workspace_id: i64,
    pub installation_id: i64,
    /// `manual` or `reinstall`.
    pub trigger: &'static str,
    pub requested_by: Option<i64>,
    /// For a reinstall: the installer's choices.
    pub auto_update: Option<bool>,
    pub accept_publisher_uninstall: Option<bool>,
    pub inbox_bindings: Option<Value>,
}

/// The sync a request created or joined.
pub struct RequestedSync {
    pub sync_id: i64,
    pub workspace_id: i64,
    pub created: bool,
}

/// `templateSyncRequestService.request` + `enqueueSync`: queues a sync of one
/// installed copy to the template's newest version, or returns the one in flight.
pub async fn request(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    input: SyncRequest,
    actor: &Actor,
) -> Result<RequestedSync> {
    assert_can_write_installs(actor)?;
    let actor = actor.clone();
    let requested = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        request_in(conn, &input, &actor)
    })
    .await?;
    if requested.created {
        queue_sync(jobs, requested.sync_id, requested.workspace_id).await;
    }
    Ok(requested)
}

fn request_in(conn: &Connection, input: &SyncRequest, actor: &Actor) -> Result<RequestedSync> {
    let installation =
        installs::find_by_id_and_workspace(conn, input.installation_id, input.workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Template installation not found"))?;
    let Some(template_id) = installation.template_id else {
        return Err(not_syncable());
    };
    if installation.source_deleted_at.is_some() || !is_active(&installation) {
        return Err(not_syncable());
    }
    let template = templates::find_sync_source(conn, template_id)?.ok_or_else(not_syncable)?;
    // The installer must be allowed to use what the sync will write.
    let used = used_categories(&template.category_counts.0);
    assert_can_use_categories(actor, used.iter().map(String::as_str))?;
    if input.trigger == "reinstall"
        && let Some(granted) = input.accept_publisher_uninstall
    {
        installs::set_publisher_uninstall_grant(
            conn,
            input.workspace_id,
            installation.source_template_id.unwrap_or(template_id),
            granted,
        )?;
    }
    // An uninstall owns the copies until it is final.
    if template_uninstall::is_in_progress(conn, installation.id)? {
        return Err(uninstall_in_progress());
    }
    let active = syncs::find_active_by_installation(conn, installation.id)?;
    if input.trigger == "reinstall"
        && active
            .as_ref()
            .is_some_and(|a| a.trigger == "autoUpdate" && a.status == "running")
    {
        return Err(sync_in_progress());
    }
    if input.trigger == "reinstall" {
        let auto_update = input.auto_update.unwrap_or(template.default_auto_update);
        conn.execute(
            r#"UPDATE "TemplateInstallation" SET "autoUpdate" = ?1,
                   "inboxBindings" = COALESCE(?2, "inboxBindings"), "updatedAt" = ?3 WHERE "id" = ?4"#,
            chatbotx_db::rusqlite::params![
                auto_update,
                input.inbox_bindings.as_ref().map(Value::to_string),
                now_ms(),
                installation.id
            ],
        )
        .map_err(db_err)?;
    }
    match active {
        // A reinstall is the installer's own request: a queued auto-update gives way to it.
        Some(a)
            if a.status == "pending"
                && a.trigger == "autoUpdate"
                && input.trigger == "reinstall" =>
        {
            syncs::close_unclaimed(conn, a.id, "cancelled", "templateSyncSuperseded", None)?;
        }
        Some(a) => {
            return Ok(RequestedSync {
                sync_id: a.id,
                workspace_id: a.workspace_id,
                created: false,
            });
        }
        None => {}
    }
    let latest =
        template_version::find_latest_number(conn, template_id)?.ok_or_else(not_syncable)?;
    let row = syncs::create_if_absent(
        conn,
        &syncs::NewTemplateSync {
            id: new_id(),
            workspace_id: input.workspace_id,
            installation_id: installation.id,
            trigger: input.trigger.to_string(),
            requested_version: latest,
            requested_by: input.requested_by,
            requested_by_kind: Some(
                if input.requested_by.is_some() {
                    "member"
                } else {
                    "apiToken"
                }
                .into(),
            ),
            idempotency_key: None,
            request_fingerprint: None,
        },
    )?
    .ok_or_else(not_syncable)?;
    Ok(RequestedSync {
        sync_id: row.id,
        workspace_id: row.workspace_id,
        created: true,
    })
}

async fn queue_sync(jobs: Option<&dyn Enqueuer>, sync_id: i64, workspace_id: i64) {
    let Some(jobs) = jobs else { return };
    if let Err(error) = super::save::enqueue_sync(jobs, sync_id, workspace_id, 1).await {
        tracing::error!(%error, sync_id, "Queuing a template sync failed; the sweep will retry it");
    }
}

/// `fanOutTemplateUpdate`: one batch of auto-update syncs for the
/// installations behind `version`, each queued, then the next batch.
pub async fn fan_out(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    template_id: i64,
    workspace_id: i64,
    version: i64,
    after_id: Option<i64>,
) -> Result<()> {
    let (created, next) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        if !template_version::exists(conn, template_id, version)? {
            return Ok((Vec::new(), None));
        }
        let codes: Vec<String> = PERMANENT_ERROR_CODES
            .iter()
            .map(|c| (*c).to_string())
            .collect();
        let candidates = syncs::list_auto_update_candidates(
            conn,
            template_id,
            version,
            after_id,
            FAN_OUT_BATCH_SIZE,
            &codes,
        )?;
        let next = (candidates.len() as i64 == FAN_OUT_BATCH_SIZE)
            .then(|| candidates.last().map(|c| c.id))
            .flatten();
        let rows: Vec<syncs::NewTemplateSync> = candidates
            .iter()
            .map(|c| syncs::NewTemplateSync {
                id: new_id(),
                workspace_id: c.workspace_id,
                installation_id: c.id,
                trigger: "autoUpdate".into(),
                requested_version: version,
                requested_by: None,
                requested_by_kind: Some(c.installed_by_kind.clone()),
                idempotency_key: None,
                request_fingerprint: None,
            })
            .collect();
        let created = syncs::create_many_if_absent(conn, &rows)?;
        Ok((created, next))
    })
    .await?;
    for sync in created {
        queue_sync(jobs, sync.id, sync.workspace_id).await;
    }
    if let (Some(jobs), Some(next)) = (jobs, next) {
        super::save::enqueue_fan_out(jobs, template_id, workspace_id, version, Some(next), 10)
            .await?;
    }
    Ok(())
}

/// `syncTemplateInstallation`: claims the sync, applies the newest version
/// in one write transaction and records the outcome; a failure is recorded on
/// the sync (the transaction rolled back everything else).
pub async fn run_sync_job(db: &Db, workspace_id: i64, sync_id: i64, app_url: &str) -> Result<()> {
    let token = chatbotx_db::repo::random_uuid();
    let claim_token = token.clone();
    let claimed = tx::write(db, move |tx| {
        Ok(syncs::claim(
            tx,
            sync_id,
            workspace_id,
            &claim_token,
            now_ms() - SYNC_LEASE_MS,
        )?)
    })
    .await?;
    let Some(sync) = claimed else {
        return Ok(());
    };
    let app_url = app_url.to_string();
    let run_token = token.clone();
    let applied = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        apply(conn, &sync, &run_token, &app_url)
    })
    .await;
    if let Err(error) = applied {
        let code = match &error {
            BusinessError::Exception { code, .. } => code.clone(),
            _ => "templateSyncFailed".to_string(),
        };
        tracing::warn!(%error, sync_id, "Template sync failed");
        tx::write(db, move |tx| {
            syncs::finish(
                tx,
                sync_id,
                &token,
                &syncs::TemplateSyncOutcome {
                    status: "failed".into(),
                    error_code: Some(code),
                    from_version: None,
                    applied_version: None,
                    applied_hash: None,
                    counts: None,
                    warnings: None,
                    warning_summary: None,
                },
            )?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

fn lease_lost() -> BusinessError {
    BusinessError::exception(
        "The template sync lease was lost",
        "templateSyncLeaseLost",
        500,
    )
}

fn finish_skipped(
    conn: &Connection,
    sync: &TemplateInstallationSync,
    token: &str,
    code: &str,
) -> Result<()> {
    let finished = syncs::finish(
        conn,
        sync.id,
        token,
        &syncs::TemplateSyncOutcome {
            status: "skipped".into(),
            error_code: Some(code.into()),
            from_version: None,
            applied_version: None,
            applied_hash: None,
            counts: None,
            warnings: None,
            warning_summary: None,
        },
    )?;
    if finished { Ok(()) } else { Err(lease_lost()) }
}

/// `resolveInstallerActor`: the member behind the sync (the installer for an
/// auto-update), `None` once they lost access; a token runs with token power.
fn installer_actor(
    conn: &Connection,
    sync: &TemplateInstallationSync,
    installation: &TemplateInstallation,
) -> Result<Option<Actor>> {
    let (user, kind) = if sync.trigger == "autoUpdate" {
        (
            installation.installed_by,
            installation.installed_by_kind.as_str(),
        )
    } else {
        (sync.requested_by, sync.requested_by_kind.as_str())
    };
    if kind != "member" {
        return Ok(Some(Actor::ApiToken { full: true }));
    }
    let Some(user) = user else {
        return Ok(None);
    };
    let permissions: Option<String> = conn
        .query_row(
            r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2 LIMIT 1"#,
            [installation.workspace_id, user],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(permissions.map(|p| Actor::Member(serde_json::from_str(&p).unwrap_or_default())))
}

/// `applyInTransaction`.
fn apply(
    conn: &Connection,
    sync: &TemplateInstallationSync,
    token: &str,
    app_url: &str,
) -> Result<()> {
    let installation =
        installs::find_by_id_and_workspace(conn, sync.installation_id, sync.workspace_id)?;
    let Some(installation) = installation.filter(|i| i.source_deleted_at.is_none() && is_active(i))
    else {
        return finish_skipped(conn, sync, token, "templateInstallationInactive");
    };
    let Some(template_id) = installation.template_id else {
        return finish_skipped(conn, sync, token, "templateInstallationInactive");
    };
    if template_uninstall::is_in_progress(conn, installation.id)? {
        return finish_skipped(conn, sync, token, "templateUninstallInProgress");
    }
    let latest = template_version::find_latest(conn, template_id)?.ok_or_else(|| {
        BusinessError::exception(
            "The template of this installation no longer exists",
            "templateInstallationSourceMissing",
            409,
        )
    })?;
    if sync.trigger == "autoUpdate" {
        // A queued or swept auto-update respects what the installer chose since.
        if !installation.auto_update {
            return finish_skipped(conn, sync, token, "templateAutoUpdateOff");
        }
        if installation.source_version >= latest.version {
            let rows = syncs::list_installed_resources(conn, installation.id)?;
            if !has_lagging_updatable_rows(&rows, installation.source_version) {
                return finish_skipped(conn, sync, token, "templateUpToDate");
            }
        }
    }
    let payload =
        chatbotx_core::flow_config::import_export::parse_template_export(&latest.payload.0)
            .map_err(|reason| {
                BusinessError::exception(
                    format!("Template payload failed validation at sync time: {reason}"),
                    "templatePayloadIncompatible",
                    400,
                )
            })?;
    let Some(actor) = installer_actor(conn, sync, &installation)? else {
        return finish_skipped(conn, sync, token, "templateInstallerAccessLost");
    };
    let mut ctx = Ctx::new(conn, installation.workspace_id, installation.id, app_url);
    if let Some(bindings) = installation
        .inbox_bindings
        .as_ref()
        .and_then(|b| b.0.as_object())
    {
        ctx.inbox_bindings = bindings
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
    }
    let result = apply_version(
        &mut ctx,
        &SyncInput {
            sync_id: sync.id,
            payload: &payload,
            version: latest.version,
            permissions: installation.permissions.0.clone(),
            template_name: installation.template_name.clone(),
        },
        &actor,
    )?;
    let summary = summarize(&result.warnings);
    let finished = syncs::finish(
        conn,
        sync.id,
        token,
        &syncs::TemplateSyncOutcome {
            status: result.status.into(),
            error_code: None,
            from_version: Some(installation.source_version),
            applied_version: Some(latest.version),
            applied_hash: Some(latest.payload_hash.clone()),
            counts: Some(result.counts),
            warnings: Some(Value::Array(result.warnings)),
            warning_summary: Some(summary),
        },
    )?;
    if !finished {
        return Err(lease_lost());
    }
    syncs::complete_installation_sync(conn, installation.id, latest.version)?;
    if sync.trigger == "reinstall" {
        // A reinstall counts as an install; auto and manual syncs do not.
        installs::record_install_event(conn, installation.id)?;
    }
    Ok(())
}

/// `summarize`: `[{ code, count }]` by warning kind, in first-seen order.
fn summarize(warnings: &[Value]) -> Value {
    let mut by_code: Vec<(String, i64)> = Vec::new();
    for warning in warnings {
        let code = warning["entityKind"].as_str().unwrap_or_default();
        match by_code.iter_mut().find(|(c, _)| c == code) {
            Some((_, n)) => *n += 1,
            None => by_code.push((code.to_string(), 1)),
        }
    }
    Value::Array(
        by_code
            .into_iter()
            .map(|(code, count)| json!({ "code": code, "count": count }))
            .collect(),
    )
}

/// `templateSyncRestoreService.restoreBackup`: puts back what a sync
/// overwrote on one copy. Refused while a sync or an uninstall runs.
pub async fn restore_backup(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    backup_id: i64,
    actor: &Actor,
) -> Result<()> {
    assert_can_write_installs(actor)?;
    let actor = actor.clone();
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let installation = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .filter(|i| i.source_template_id.is_some() && is_active(i))
            .ok_or_else(|| BusinessError::not_found("Template installation not found"))?;
        let backup =
            backups::find_by_id_and_installation(conn, backup_id, installation.id, workspace_id)?
                .ok_or_else(|| BusinessError::not_found("Backup not found"))?;
        if syncs::find_active_by_installation(conn, installation.id)?.is_some() {
            return Err(sync_in_progress());
        }
        if template_uninstall::is_in_progress(conn, installation.id)? {
            return Err(uninstall_in_progress());
        }
        let row = syncs::list_installed_resources(conn, installation.id)?
            .into_iter()
            .find(|r| r.id == backup.installed_resource_id)
            .filter(|r| r.detached_at.is_none() && !r.was_existing)
            .ok_or_else(|| BusinessError::not_found("Backup target not found"))?;
        assert_can_use_categories(&actor, [row.category.as_str()])?;
        if row.resource_kind != backup.resource_kind {
            return Err(super::install::sync::not_restorable());
        }
        if super::install::project(conn, workspace_id, &row.category, row.resource_id)?.is_none() {
            return Err(BusinessError::not_found("Backup target not found"));
        }
        let mut ctx = Ctx::new(conn, workspace_id, installation.id, "");
        restore(
            &mut ctx,
            &row.resource_kind,
            row.resource_id,
            &backup.content.0,
            backup.format_version,
        )?;
        Ok(json!({ "ok": true }))
    })
    .await?;
    Ok(())
}
