//! `installation-request.service.ts` (`request`, `retry`, `getDetail`,
//! `listResources`, `preview`, `previewWithCollisions`), `installability.ts`,
//! `install-preview.ts` (`previewInstallCollisions`), `warning-groups.ts` and
//! the install job (`apps/worker` `installTemplate`, Q.D4 slice 3).

use std::collections::BTreeMap;

use chatbotx_core::flow_config::import_export::parse_template_export;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Template as TemplateRow, TemplateInstallation};
use chatbotx_db::repo::platform_admin::ROOT_TENANT_ID;
use chatbotx_db::repo::template_installation as installs;
use chatbotx_db::repo::template_installation::TEMPLATE_INSTALLER_WARNING_CODES;
use chatbotx_db::repo::template_installed_resource as installed;
use chatbotx_db::repo::template_installed_resource_meta::load_resource_labels;
use chatbotx_db::repo::{iso_millis, now_ms, template as templates, template_version};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _};
use chatbotx_jobs::Enqueuer;
use serde_json::{Value, json};

use super::install::{self, Ctx, INSTALL_LEASE_MS, RunInput, db_err};
use super::permissions::{Actor, assert_can_use_categories, assert_can_write_installs};
use super::save::workspace_tenant_id;
use super::snapshot::category_counts;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `PUBLISHER_UNINSTALL_DISCLOSURE_VERSION`.
pub const PUBLISHER_UNINSTALL_DISCLOSURE_VERSION: i64 = 1;
const RESOURCES_PAGE_SIZE: i64 = 100;
const MAX_DETAIL_WARNINGS: usize = 200;

fn exception_with_data(message: &str, code: &str, status: u16, data: Value) -> BusinessError {
    BusinessError::Exception {
        message: message.to_string(),
        code: code.to_string(),
        status,
        field: None,
        data: Some(data),
    }
}

/// `(manifest, sourceId, name, action, targetId)` of one collision row.
type PushCollision<'a> = dyn FnMut(&str, &str, &Value, &str, Option<i64>) + 'a;

/// `templateInvalidInboxBindingException`.
pub fn invalid_inbox_binding(source_inbox_id: &str) -> BusinessError {
    exception_with_data(
        "The chosen inbox does not match the template's inbox",
        "templateInvalidInboxBinding",
        400,
        json!({ "sourceInboxId": source_inbox_id }),
    )
}

fn already_installed(installation_id: i64) -> BusinessError {
    exception_with_data(
        "This template is already installed in this workspace",
        "templateAlreadyInstalled",
        409,
        json!({ "installationId": installation_id.to_string() }),
    )
}

fn not_retryable() -> BusinessError {
    BusinessError::exception(
        "Only a failed installation can be retried",
        "templateInstallationNotRetryable",
        409,
    )
}

fn source_missing() -> BusinessError {
    BusinessError::exception(
        "The template this was installed from no longer exists",
        "templateSourceMissing",
        400,
    )
}

fn installation_not_found() -> BusinessError {
    BusinessError::not_found("Template installation not found")
}

/// `isShareLive`.
fn is_share_live(template: &TemplateRow, now: i64) -> bool {
    template.share_enabled && template.share_expires_at.is_none_or(|at| at > now)
}

/// `canInstallFromTenant`.
fn can_install_from_tenant(template_tenant: i64, target_tenant: i64) -> bool {
    target_tenant == template_tenant || template_tenant == ROOT_TENANT_ID
}

/// `InstallableTemplate`.
pub struct Installable {
    pub template: TemplateRow,
    pub payload: Value,
    pub target_tenant_id: i64,
}

/// `assertTemplateInstallable`.
pub fn assert_installable(
    conn: &Connection,
    template: Option<TemplateRow>,
    target_workspace_id: i64,
    actor: &Actor,
) -> Result<Installable> {
    let template = template
        .filter(|t| is_share_live(t, now_ms()))
        .ok_or_else(|| {
            BusinessError::exception(
                "This share link is no longer available",
                "templateShareDisabled",
                400,
            )
        })?;
    if template.workspace_id == target_workspace_id {
        return Err(BusinessError::exception(
            "A template can not be installed into the workspace it was created in",
            "templateInstallIntoSource",
            400,
        ));
    }
    let target_tenant_id = workspace_tenant_id(conn, target_workspace_id)?;
    if !can_install_from_tenant(template.tenant_id, target_tenant_id) {
        return Err(BusinessError::exception(
            "This template is not available for your workspace",
            "templateCrossTenantInstall",
            400,
        ));
    }
    let payload = parse_template_export(&template.payload.0).map_err(|reason| {
        BusinessError::exception(
            format!("This template's saved data is no longer compatible: {reason}"),
            "templatePayloadIncompatible",
            400,
        )
    })?;
    let counts = category_counts(&payload);
    let used = counts
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, n)| n.as_u64().unwrap_or(0) > 0)
        .map(|(k, _)| k.as_str());
    assert_can_use_categories(actor, used)?;
    Ok(Installable {
        template,
        payload,
        target_tenant_id,
    })
}

/// `templateInstallationRequestService.request` input.
#[derive(Debug, Clone, Default)]
pub struct InstallRequest {
    pub share_token: String,
    pub workspace_id: i64,
    pub installed_by: Option<i64>,
    pub allow_duplicate: bool,
    pub accept_publisher_uninstall: Option<bool>,
    pub auto_update: Option<bool>,
    pub idempotency_key: Option<String>,
    pub inbox_bindings: BTreeMap<String, String>,
}

/// What a request queued: an install, or (for a template installed here
/// already) a reinstall sync.
#[derive(Debug, Clone)]
pub struct Requested {
    pub installation_id: i64,
    pub workspace_id: i64,
    pub created: bool,
    pub sync_id: Option<i64>,
}

enum Outcome {
    Install(TemplateInstallation, bool),
    Reinstall(TemplateInstallation),
}

/// `validateInboxBindings`.
fn validate_inbox_bindings(
    conn: &Connection,
    workspace_id: i64,
    bindings: &BTreeMap<String, String>,
    manifest: &Value,
) -> Result<Option<Value>> {
    if bindings.is_empty() {
        return Ok(None);
    }
    let ids: Vec<i64> = bindings.values().filter_map(|id| id.parse().ok()).collect();
    let targets = chatbotx_db::repo::inbox::list_identities_by_ids(conn, workspace_id, &ids)?;
    for (source, target) in bindings {
        let wanted = &manifest[source];
        let found = targets.iter().find(|t| t.id.to_string() == *target);
        let ok = !wanted.is_null()
            && found.is_some_and(|t| t.status == "connected" && wanted["channelType"] == t.channel);
        if !ok {
            return Err(invalid_inbox_binding(source));
        }
    }
    Ok(Some(json!(bindings)))
}

/// `templateInstallationRequestService.request` + `requestTemplateInstall`:
/// the pending installation (or the reinstall sync), then the queued job.
pub async fn request(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    input: InstallRequest,
    actor: Actor,
) -> Result<Requested> {
    assert_can_write_installs(&actor)?;
    let actor_in = actor.clone();
    let outcome = tx::write(db, move |conn| {
        let template = templates::find_live_by_share_token(conn, &input.share_token)?;
        let Installable {
            template,
            payload,
            target_tenant_id,
        } = assert_installable(conn, template, input.workspace_id, &actor_in)?;
        let cross_tenant = target_tenant_id != template.tenant_id;
        let accept = if cross_tenant {
            Some(false)
        } else {
            input.accept_publisher_uninstall
        };
        let bindings = validate_inbox_bindings(
            conn,
            input.workspace_id,
            &input.inbox_bindings,
            &payload["manifests"]["inboxes"],
        )?;
        let (ws, source) = (input.workspace_id, template.id);
        let active = installs::find_active_by_source_template(conn, ws, source)?;
        if active.is_none()
            && let Some(installed) = installs::find_installed_by_source_template(conn, ws, source)?
            && !input.allow_duplicate
        {
            return Ok(Outcome::Reinstall(installed));
        }
        if let Some(accepted) = accept {
            installs::set_publisher_uninstall_grant(conn, ws, source, accepted)?;
        }
        if let Some(active) = active {
            return Ok(Outcome::Install(active, false));
        }
        let version = template_version::pin_version_number(conn, template.id, template.version)?;
        let publisher_ref = installs::find_publisher_ref(conn, ws, source)?
            .unwrap_or_else(|| chatbotx_db::repo::random_uuid().replace('-', ""));
        let grant = match accept {
            Some(true) => PUBLISHER_UNINSTALL_DISCLOSURE_VERSION,
            Some(false) => 0,
            None => installs::find_publisher_uninstall_grant(conn, ws, source)?
                .map_or(PUBLISHER_UNINSTALL_DISCLOSURE_VERSION, |p| {
                    p.min(PUBLISHER_UNINSTALL_DISCLOSURE_VERSION)
                }),
        };
        let kind = if input.installed_by.is_some() {
            "member"
        } else {
            "apiToken"
        };
        let row = installs::create(
            conn,
            &vec![
                ("workspaceId", Sql::Integer(ws)),
                ("templateId", Sql::Integer(template.id)),
                ("sourceTemplateId", Sql::Integer(template.id)),
                ("templateName", Sql::Text(template.name.clone())),
                ("sourceWorkspaceId", Sql::Integer(template.workspace_id)),
                ("formatVersion", Sql::Integer(template.format_version)),
                ("status", Sql::Text("pending".into())),
                (
                    "permissions",
                    Sql::Text(template.default_permissions.0.to_string()),
                ),
                (
                    "autoUpdate",
                    Sql::Integer(i64::from(
                        input.auto_update.unwrap_or(template.default_auto_update),
                    )),
                ),
                ("sourceUpdatedAt", Sql::Integer(template.updated_at)),
                ("sourceVersion", Sql::Integer(version)),
                ("publisherRef", Sql::Text(publisher_ref)),
                ("publisherUninstallGrant", Sql::Integer(grant)),
                (
                    "installedBy",
                    input.installed_by.map_or(Sql::Null, Sql::Integer),
                ),
                ("installedByKind", Sql::Text(kind.into())),
                ("installSource", Sql::Text("manual".into())),
                (
                    "inboxBindings",
                    bindings.map_or(Sql::Null, |b| Sql::Text(b.to_string())),
                ),
            ],
        )?;
        Ok(Outcome::Install(row, true))
    })
    .await?;
    match outcome {
        Outcome::Install(row, created) => {
            queue_install(jobs, row.id, row.workspace_id).await;
            Ok(Requested {
                installation_id: row.id,
                workspace_id: row.workspace_id,
                created,
                sync_id: None,
            })
        }
        Outcome::Reinstall(row) => {
            // ponytail: the reinstall sync is slice 4 (`templateSyncRequestService`).
            let _ = row;
            Err(BusinessError::exception(
                "Reinstalling an installed template is not available yet",
                "notPorted",
                501,
            ))
        }
    }
}

/// `retry`: a failed installation goes back to `pending` on the newest version.
pub async fn retry(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    workspace_id: i64,
    installation_id: i64,
    actor: Actor,
    retried_by: Option<i64>,
) -> Result<i64> {
    assert_can_write_installs(&actor)?;
    let row = tx::write(db, move |conn| {
        let existing = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .ok_or_else(installation_not_found)?;
        if existing.status != "failed" {
            return Err(not_retryable());
        }
        let template_id = existing.template_id.ok_or_else(source_missing)?;
        let template = templates::find_live_by_id(conn, template_id)?;
        assert_installable(conn, template, workspace_id, &actor)?;
        let source = existing.source_template_id.unwrap_or(template_id);
        if installs::find_active_by_source_template(conn, workspace_id, source)?.is_some() {
            return Err(not_retryable());
        }
        if let Some(installed) =
            installs::find_installed_by_source_template(conn, workspace_id, source)?
        {
            return Err(already_installed(installed.id));
        }
        let latest = template_version::find_latest(conn, template_id)?;
        installs::reset_failed(
            conn,
            &installs::ResetFailed {
                id: installation_id,
                workspace_id,
                installed_by: retried_by,
                installed_by_kind: if retried_by.is_some() {
                    "member"
                } else {
                    "apiToken"
                }
                .into(),
                source_version: latest.map(|v| v.version),
            },
        )?
        .ok_or_else(not_retryable)
    })
    .await?;
    queue_install(jobs, row.id, row.workspace_id).await;
    Ok(row.id)
}

/// `enqueueInstall`: best effort; the sweep retries a miss.
async fn queue_install(jobs: Option<&dyn Enqueuer>, installation_id: i64, workspace_id: i64) {
    let Some(jobs) = jobs else { return };
    if let Err(error) = super::save::enqueue_install(jobs, installation_id, workspace_id).await {
        tracing::error!(%error, installation_id, "Queuing a template install failed; the sweep will retry it");
    }
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// `assertCanViewInstallation`.
fn assert_can_view(conn: &Connection, row: &TemplateInstallation, actor: &Actor) -> Result<()> {
    let Some(visible) = super::permissions::visible_install_categories(actor) else {
        return Ok(());
    };
    let copies =
        chatbotx_db::repo::template_installation_sync::list_installed_resources(conn, row.id)?;
    let categories: Option<Vec<String>> = if !copies.is_empty() {
        Some(copies.iter().map(|c| c.category.clone()).collect())
    } else {
        match row.source_template_id {
            Some(id) => templates::find_category_counts(conn, id)?.map(|counts| {
                counts
                    .0
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, n)| n.as_i64().unwrap_or(0) > 0)
                    .map(|(c, _)| c.clone())
                    .collect()
            }),
            None => None,
        }
    };
    match categories {
        Some(c) if c.iter().all(|c| visible.contains(&c.as_str())) => Ok(()),
        _ => Err(installation_not_found()),
    }
}

/// `sourceIdOfPath`: the id after the first `.` of a warning path.
fn source_id_of_path(path: &str) -> Option<String> {
    let rest = path.split_once('.')?.1;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

fn resource_kind_of_category(category: &str) -> Option<&'static str> {
    Some(match category {
        "flows" => "flow",
        "triggers" => "trigger",
        "keywords" => "automatedResponse",
        "sequences" => "sequence",
        "aiAgents" => "aiAgent",
        "aiFunctions" => "aiFunction",
        "aiMcpServers" => "aiMcpServer",
        "aiFiles" => "aiFile",
        "fbCommentAutomations" => "fbCommentAutomation",
        "calendars" => "calendar",
        "webchats" => "integrationWebchat",
        "products" => "product",
        "questionnaires" => "questionnaire",
        "couponTopics" => "couponTopic",
        "pipelines" => "pipeline",
        "entryPointLinks" => "reflink",
        _ => return None,
    })
}

fn is_installer_code(code: &str) -> bool {
    TEMPLATE_INSTALLER_WARNING_CODES.contains(&code)
}

/// `loadInstallationWarningGroups(…, "installer")`.
pub fn installer_warning_groups(
    conn: &Connection,
    source_workspace_id: i64,
    warnings: &[Value],
) -> Result<Vec<Value>> {
    let warnings: Vec<&Value> = warnings
        .iter()
        .filter(|w| is_installer_code(w["entityKind"].as_str().unwrap_or_default()))
        .collect();
    let mut resources: Vec<(String, i64)> = Vec::new();
    for w in &warnings {
        let category = w["category"].as_str().unwrap_or_default();
        if let (Some(id), Some(kind)) = (
            source_id_of_path(w["path"].as_str().unwrap_or_default()),
            resource_kind_of_category(category),
        ) && let Ok(id) = id.parse()
        {
            resources.push((kind.to_string(), id));
        }
    }
    let labels = load_resource_labels(conn, source_workspace_id, &resources)?;
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for w in warnings {
        let source = source_id_of_path(w["path"].as_str().unwrap_or_default());
        let key = format!(
            "{}|{}",
            w["category"].as_str().unwrap_or_default(),
            source
                .clone()
                .unwrap_or_else(|| w["value"].as_str().unwrap_or_default().to_string())
        );
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows.push(w),
            None => groups.push((key, vec![w])),
        }
    }
    let mut out: Vec<Value> = groups
        .into_iter()
        .map(|(_, rows)| {
            let first = rows[0];
            let category = first["category"].as_str().unwrap_or_default();
            let source = source_id_of_path(first["path"].as_str().unwrap_or_default());
            let label = source.as_ref().and_then(|s| {
                labels
                    .get(&format!("{}:{s}", resource_kind_of_category(category)?))
                    .cloned()
            });
            let value = first["value"].as_str().unwrap_or_default();
            let readable = (!value.is_empty() && !value.chars().all(|c| c.is_ascii_digit())).then(|| value.to_string());
            let mut counts: Vec<(String, i64)> = Vec::new();
            for row in &rows {
                let code = row["entityKind"].as_str().unwrap_or_default().to_string();
                match counts.iter_mut().find(|(c, _)| *c == code) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((code, 1)),
                }
            }
            let skipped = rows.iter().any(|r| {
                matches!(r["entityKind"].as_str(), Some("flowSkippedRequiredReference" | "flowInvalidStructure"))
            });
            json!({
                "category": category,
                "sourceId": source,
                "name": label.or(readable),
                "skipped": skipped,
                "issues": counts.into_iter().map(|(code, count)| json!({ "code": code, "count": count })).collect::<Vec<_>>(),
            })
        })
        .collect();
    out.sort_by_key(|g| g["skipped"] != true);
    Ok(out)
}

fn opt_iso(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, |m| json!(iso_millis(m)))
}

/// `describeUninstall`.
fn describe_uninstall(row: &TemplateInstallation) -> Value {
    if row.status != "uninstalled" {
        return Value::Null;
    }
    json!({
        "uninstalledAt": opt_iso(row.uninstalled_at),
        "uninstalledBy": row.uninstalled_by_kind,
        "counts": row.uninstall_counts.as_ref().and_then(|c| templates::parse_template_uninstall_counts(&c.0)),
    })
}

/// `getDetail`.
pub async fn get_detail(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    actor: Actor,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let row = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .ok_or_else(installation_not_found)?;
        assert_can_view(conn, &row, &actor)?;
        let installer: Option<String> = match row.installed_by {
            Some(user) => conn
                .query_row(r#"SELECT "name" FROM "User" WHERE "id" = ?1"#, [user], |r| r.get(0))
                .optional()
                .map_err(db_err)?,
            None => None,
        };
        let latest = match row.template_id {
            Some(id) if row.source_deleted_at.is_none() => templates::find_live_version(conn, id)?,
            _ => None,
        };
        let active_sync = chatbotx_db::repo::template_installation_sync::find_active_by_installation(conn, row.id)?;
        let resources = installed::list_categories(conn, row.id, workspace_id)?;
        let warnings: Vec<Value> = row.warnings.0.as_array().cloned().unwrap_or_default();
        let groups = installer_warning_groups(conn, row.source_workspace_id, &warnings)?;
        let mut summary: Vec<(String, i64)> = Vec::new();
        for w in &warnings {
            let code = w["entityKind"].as_str().unwrap_or_default().to_string();
            match summary.iter_mut().find(|(c, _)| *c == code) {
                Some((_, n)) => *n += 1,
                None => summary.push((code, 1)),
            }
        }
        let mut by_category: Vec<(String, i64, i64)> = Vec::new();
        for r in &resources {
            match by_category.iter_mut().find(|(c, _, _)| *c == r.category) {
                Some((_, total, reused)) => {
                    *total += 1;
                    *reused += i64::from(r.was_existing);
                }
                None => by_category.push((r.category.clone(), 1, i64::from(r.was_existing))),
            }
        }
        let counts = row.uninstall_counts.as_ref().and_then(|c| templates::parse_template_uninstall_counts(&c.0));
        Ok(json!({
            "id": row.id.to_string(),
            "templateName": row.template_name,
            "status": row.status,
            "errorCode": row.error_code,
            "resourceCount": row.resource_count,
            "warningCount": row.warning_count,
            "installerWarningCount": warnings.iter().filter(|w| is_installer_code(w["entityKind"].as_str().unwrap_or_default())).count(),
            "attempts": row.attempts,
            "claimedAt": opt_iso(row.claimed_at),
            "autoUpdate": row.auto_update,
            "sourceVersion": row.source_version,
            "updateAvailable": latest.is_some_and(|v| v > row.source_version),
            "sourceDeleted": row.source_deleted_at.is_some() || row.template_id.is_none(),
            "lastSyncedAt": opt_iso(row.last_synced_at),
            "syncInProgress": active_sync.is_some(),
            "publisherUninstallGrant": row.publisher_uninstall_grant > 0,
            "installedByName": installer,
            "installSource": row.install_source,
            "createdAt": iso_millis(row.created_at),
            "completedAt": opt_iso(row.completed_at),
            "warnings": warnings.iter().take(MAX_DETAIL_WARNINGS).map(|w| json!({
                "category": w["category"], "code": w["entityKind"], "path": w["path"], "value": w["value"],
            })).collect::<Vec<_>>(),
            "warningSummary": summary.into_iter().map(|(code, count)| json!({ "code": code, "count": count })).collect::<Vec<_>>(),
            "warningGroups": groups,
            "resourceSummary": by_category.into_iter().map(|(category, total, reused)| json!({
                "category": category,
                "total": total,
                "created": total - reused,
                "reused": reused,
                "warnings": warnings.iter().filter(|w| w["category"] == category.as_str()).count(),
            })).collect::<Vec<_>>(),
            "uninstallState": row.uninstall_state,
            "uninstallErrorCode": row.uninstall_error_code,
            "uninstallCounts": counts,
            "uninstall": describe_uninstall(&row),
        }))
    })
    .await
}

/// `listResources`.
pub async fn list_resources(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    after_id: Option<i64>,
    limit: Option<i64>,
    category: Option<String>,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let installation = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .ok_or_else(installation_not_found)?;
        let page = installed::list_page(
            conn,
            &installed::ListPageInput {
                installation_id,
                workspace_id,
                after_id,
                limit: limit.unwrap_or(RESOURCES_PAGE_SIZE),
                category,
            },
        )?;
        let governed = installation.template_id.is_some() && installation.source_deleted_at.is_none();
        let keys: Vec<(String, i64)> = page.rows.iter().map(|r| (r.resource_kind.clone(), r.resource_id)).collect();
        let labels = load_resource_labels(conn, workspace_id, &keys)?;
        let items: Vec<Value> = page
            .rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.id.to_string(),
                    "category": r.category,
                    "resourceKind": r.resource_kind,
                    "resourceId": r.resource_id.to_string(),
                    "label": labels.get(&format!("{}:{}", r.resource_kind, r.resource_id)),
                    "created": !r.was_existing,
                    "locked": governed && !(r.was_existing || r.detached_at.is_some() || r.allow_edit),
                })
            })
            .collect();
        Ok(json!({ "items": items, "nextCursor": page.next_cursor.map(|c| c.to_string()) }))
    })
    .await
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

fn publisher_uninstall_disclosure() -> Value {
    json!({
        "version": PUBLISHER_UNINSTALL_DISCLOSURE_VERSION,
        "publisherSees": ["workspaceName", "installCounts", "uninstallCounts"],
        "publisherCan": ["uninstall"],
        "uninstallDeletes": "Everything the template created in this workspace, including copies you edited; anything you built on top of a copy is kept.",
    })
}

/// `PREVIEW_REASON_BY_CODE`.
fn preview_reason(code: &str) -> &str {
    match code {
        "templateShareDisabled" | "templateCrossTenantInstall" => "templateShareDisabled",
        other => other,
    }
}

/// `loadPreview` (+ `previewInstallCollisions` when `with_collisions`). A
/// rejected link is `{ installable: false, reason }`, not an error.
pub async fn preview(
    db: &Db,
    share_token: String,
    workspace_id: i64,
    actor: Actor,
    with_collisions: bool,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let template = templates::find_live_by_share_token(conn, &share_token)?;
        let source = match assert_installable(conn, template, workspace_id, &actor) {
            Ok(source) => source,
            Err(BusinessError::Exception { code, .. }) => {
                return Ok(json!({ "installable": false, "reason": preview_reason(&code) }));
            }
            Err(other) => return Err(other),
        };
        let required: Vec<Value> = source.payload["manifests"]["inboxes"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, e)| json!({ "sourceId": id, "channelType": e["channelType"], "name": e["name"] }))
            .collect();
        let mut channels: Vec<Value> = Vec::new();
        for inbox in &required {
            if !channels.contains(&inbox["channelType"]) {
                channels.push(inbox["channelType"].clone());
            }
        }
        let latest = installs::find_latest_by_source_template(conn, workspace_id, source.template.id)?;
        let t = &source.template;
        let mut out = json!({
            "installable": true,
            "template": {
                "name": t.name,
                "description": t.description,
                "imageUrl": t.image_url,
                "publisherName": t.publisher_name,
                "youtubeVideoId": t.youtube_video_id,
                "testLink": t.test_link,
                "categoryCounts": t.category_counts.0,
            },
            "requiredInboxes": required,
            "requiredChannels": channels,
            "existingInstallation": latest.map(|l| json!({ "id": l.id.to_string(), "status": l.status })),
            "publisherUninstallDisclosure": publisher_uninstall_disclosure(),
        });
        if with_collisions {
            let (collisions, summary) = preview_collisions(conn, workspace_id, &source.payload, t.create_install_folder)?;
            out["collisions"] = json!(collisions);
            out["collisionSummary"] = summary;
        }
        Ok(out)
    })
    .await
}

fn found(
    conn: &Connection,
    sql: &str,
    values: &[&dyn chatbotx_db::rusqlite::ToSql],
) -> Result<Option<i64>> {
    conn.query_row(sql, values, |r| r.get(0))
        .optional()
        .map_err(db_err)
}

/// `previewInstallCollisions` (no inbox bindings).
pub fn preview_collisions(
    conn: &Connection,
    workspace_id: i64,
    payload: &Value,
    create_install_folder: bool,
) -> Result<(Vec<Value>, Value)> {
    let manifests = &payload["manifests"];
    let mut out: Vec<Value> = Vec::new();
    let mut push =
        |manifest: &str, source: &str, name: &Value, action: &str, target: Option<i64>| {
            let mut c =
                json!({ "manifest": manifest, "sourceId": source, "name": name, "action": action });
            if let Some(t) = target {
                c["targetId"] = json!(t.to_string());
            }
            out.push(c);
        };
    let each = |key: &str| -> Vec<(String, Value)> {
        manifests[key]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    for (source, e) in each("folders") {
        let parentless = e["parentSourceId"].is_null();
        let target = if parentless && !create_install_folder {
            found(
                conn,
                r#"SELECT "id" FROM "Folder" WHERE "workspaceId" = ?1 AND "folderType" = ?2 AND "parentId" IS NULL AND "name" = ?3 LIMIT 1"#,
                &[
                    &workspace_id,
                    &e["folderType"].as_str().unwrap_or_default(),
                    &e["name"].as_str().unwrap_or_default(),
                ],
            )?
        } else {
            None
        };
        push(
            "folders",
            &source,
            &e["name"],
            if target.is_some() { "reuse" } else { "create" },
            target,
        );
    }
    let fields = |table: &str,
                  entries: Vec<(String, Value)>,
                  push: &mut PushCollision<'_>,
                  manifest: &str|
     -> Result<()> {
        for (source, e) in entries {
            let name = e["name"].as_str().unwrap_or_default().trim().to_lowercase();
            let same = found(
                conn,
                &format!(
                    r#"SELECT "id" FROM "{table}" WHERE "workspaceId" = ?1 AND lower(trim("name")) = ?2 AND "type" = ?3 LIMIT 1"#
                ),
                &[
                    &workspace_id,
                    &name,
                    &e["type"].as_str().unwrap_or_default(),
                ],
            )?;
            let other = found(
                conn,
                &format!(
                    r#"SELECT "id" FROM "{table}" WHERE "workspaceId" = ?1 AND lower(trim("name")) = ?2 LIMIT 1"#
                ),
                &[&workspace_id, &name],
            )?;
            let action = if same.is_some() {
                "reuse"
            } else if other.is_some() {
                "rename"
            } else {
                "create"
            };
            push(manifest, &source, &e["name"], action, same);
        }
        Ok(())
    };
    fields(
        "CustomField",
        each("customFields"),
        &mut push,
        "customFields",
    )?;
    let bot_fields: Vec<(String, Value)> = payload["resources"]["settings"]["botFields"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            (
                e["sourceId"].as_str().unwrap_or_default().to_string(),
                e.clone(),
            )
        })
        .collect();
    fields("BotField", bot_fields, &mut push, "botFields")?;
    for (source, e) in each("tags") {
        let target = found(
            conn,
            r#"SELECT "id" FROM "Tag" WHERE "workspaceId" = ?1 AND "name" = ?2 AND "deletedAt" IS NULL LIMIT 1"#,
            &[
                &workspace_id,
                &e["name"].as_str().unwrap_or_default().trim(),
            ],
        )?;
        push(
            "tags",
            &source,
            &e["name"],
            if target.is_some() { "reuse" } else { "create" },
            target,
        );
    }
    for (source, e) in each("productCategories") {
        let target = if e["parentSourceId"].is_null() {
            found(
                conn,
                r#"SELECT "id" FROM "ProductCategory" WHERE "workspaceId" = ?1 AND lower("name") = lower(?2) AND "parentId" IS NULL LIMIT 1"#,
                &[&workspace_id, &e["name"].as_str().unwrap_or_default()],
            )?
        } else {
            None
        };
        push(
            "productCategories",
            &source,
            &e["name"],
            if target.is_some() { "reuse" } else { "create" },
            target,
        );
    }
    for (source, e) in each("pipelines") {
        let target = found(
            conn,
            r#"SELECT "id" FROM "Pipeline" WHERE "workspaceId" = ?1 AND "referenceName" = ?2 LIMIT 1"#,
            &[
                &workspace_id,
                &e["referenceName"].as_str().unwrap_or_default(),
            ],
        )?;
        push(
            "pipelines",
            &source,
            &e["name"],
            if target.is_some() { "reuse" } else { "create" },
            target,
        );
    }
    for (source, e) in each("placeCategories") {
        let target = found(
            conn,
            r#"SELECT "id" FROM "PlaceCategory" WHERE "workspaceId" = ?1 AND lower("name") = lower(?2) LIMIT 1"#,
            &[&workspace_id, &e["name"].as_str().unwrap_or_default()],
        )?;
        push(
            "placeCategories",
            &source,
            &e["name"],
            if target.is_some() { "reuse" } else { "create" },
            target,
        );
    }
    for (source, e) in each("inboxes") {
        let target = found(
            conn,
            r#"SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?1 AND "id" = ?2 AND "channel" = ?3 AND "status" = 'connected' LIMIT 1"#,
            &[
                &workspace_id,
                &source.parse::<i64>().unwrap_or(-1),
                &e["channelType"].as_str().unwrap_or_default(),
            ],
        )?;
        push(
            "inboxes",
            &source,
            &e["name"],
            if target.is_some() { "bind" } else { "unbound" },
            target,
        );
    }
    let mut summary = json!({ "create": 0, "reuse": 0, "rename": 0, "bind": 0, "unbound": 0 });
    for c in &out {
        if let Some(action) = c["action"].as_str() {
            summary[action] = json!(summary[action].as_i64().unwrap_or(0) + 1);
        }
    }
    Ok((out, summary))
}

// ---------------------------------------------------------------------------
// The install job
// ---------------------------------------------------------------------------

/// `installTemplate` (worker): claim the lease, run the install in one write
/// transaction and complete it, or record the terminal failure. A job for an
/// installation another worker owns (or a redelivery) does nothing.
pub async fn run_install_job(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    app_url: &str,
) -> Result<()> {
    let claim_token = chatbotx_db::repo::random_uuid();
    let token = claim_token.clone();
    let claimed = tx::write(db, move |conn| {
        Ok(installs::claim(
            conn,
            installation_id,
            workspace_id,
            &token,
            now_ms() - INSTALL_LEASE_MS,
        )?)
    })
    .await?;
    if claimed.is_none() {
        return Ok(());
    }
    let app_url = app_url.to_string();
    let token = claim_token.clone();
    let result = tx::write(db, move |conn| {
        apply(conn, workspace_id, installation_id, &token, &app_url)
    })
    .await;
    let Err(error) = result else { return Ok(()) };
    let (code, message) = match &error {
        BusinessError::Exception { code, message, .. } => (code.clone(), message.clone()),
        _ => (
            "templateInstallFailed".to_string(),
            "Template install failed".to_string(),
        ),
    };
    tracing::warn!(%error, installation_id, "Template install failed");
    tx::write(db, move |conn| {
        Ok(installs::fail(
            conn,
            installation_id,
            &claim_token,
            &code,
            &message,
        )?)
    })
    .await?;
    Ok(())
}

/// `findInstallationByIdOrFail` + `templateInstallService.run` + `complete`.
fn apply(
    conn: &Connection,
    workspace_id: i64,
    installation_id: i64,
    claim_token: &str,
    app_url: &str,
) -> Result<()> {
    let installation = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
        .ok_or_else(installation_not_found)?;
    let template_id = installation.template_id.ok_or_else(source_missing)?;
    let template = templates::find_install_source(conn, template_id)?.ok_or_else(source_missing)?;
    let raw = template_version::find(conn, template_id, installation.source_version)?
        .map(|v| v.payload.0)
        .unwrap_or(template.payload.0.clone());
    let payload = parse_template_export(&raw).map_err(|reason| {
        BusinessError::exception(
            format!("Template payload failed validation at install time: {reason}"),
            "templatePayloadIncompatible",
            400,
        )
    })?;
    let bindings: BTreeMap<String, String> = installation
        .inbox_bindings
        .as_ref()
        .and_then(|b| serde_json::from_value(b.0.clone()).ok())
        .unwrap_or_default();
    let mut ctx = Ctx::new(conn, workspace_id, installation_id, app_url);
    let outcome = install::run(
        &mut ctx,
        &RunInput {
            payload: &payload,
            template_name: template.name.clone(),
            create_install_folder: template.create_install_folder,
            permissions: installation.permissions.0.clone(),
            inbox_bindings: bindings,
            source_version: installation.source_version,
        },
    )?;
    let completed = installs::complete(
        conn,
        installation_id,
        claim_token,
        &installs::Completion {
            status: outcome.status.to_string(),
            warning_count: Some(outcome.warnings.len() as i64),
            warnings: Some(Value::Array(outcome.warnings)),
            resource_count: Some(outcome.resource_count),
            install_folder_id: Some(outcome.install_folder_id),
            pending_publish_effects: Some(None),
        },
    )?;
    if !completed {
        return Err(BusinessError::exception(
            "The template install lease was lost",
            "templateInstallLeaseLost",
            409,
        ));
    }
    Ok(())
}
