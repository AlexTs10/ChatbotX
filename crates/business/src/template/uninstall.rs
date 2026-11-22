//! `uninstall.service.ts`, `uninstall-decide.ts`, `uninstall-execute.service.ts`,
//! `uninstall-cancel.service.ts` and the publisher reads of `service.ts`
//! (`listPublisherInstallations`, `getInstallerIssues`; Q.D4 slice 5).
//!
//! ponytail: one SQLite write transaction per installation decides and
//! deletes (the original's Phase A freeze + Phase B drain exist to keep
//! Postgres locks short). A failed run rolls back completely, so a cancel
//! never has anything to give back. Dependents are found by direct column
//! references and by the copy's id inside the JSON columns that can hold one
//! (`uninstall-references.ts`, as a text match).

use std::collections::{BTreeMap, HashMap, HashSet};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{TemplateInstallation, TemplateInstalledResource};
use chatbotx_db::repo::template::{self as templates, ListPublisherRows};
use chatbotx_db::repo::template_installation as installs;
use chatbotx_db::repo::template_installation_sync as syncs;
use chatbotx_db::repo::template_installed_resource_uninstall as resource_uninstall;
use chatbotx_db::repo::template_uninstall as states;
use chatbotx_db::repo::{iso_millis, now_ms, parse_iso};
use chatbotx_db::rusqlite::{Connection, params_from_iter};
use chatbotx_jobs::Enqueuer;
use serde_json::{Value, json};

use super::install::sync::UPDATABLE;
use super::install::{MANIFEST_KINDS, db_err, project_hash};
use super::permissions::{
    Actor, assert_can_manage, assert_can_read, assert_can_use_categories, assert_can_write_installs,
};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `MAX_UNINSTALL_COPIES`: per installation.
const MAX_UNINSTALL_COPIES: usize = 200;
const PUBLISHER_PAGE_SIZE: i64 = 25;

/// The table of each copied kind, in delete order (holders before what they hold).
const DELETE_ORDER: &[(&str, &str)] = &[
    ("trigger", "Trigger"),
    ("automatedResponse", "AutomatedResponse"),
    ("reflink", "Reflink"),
    ("fbCommentAutomation", "CommentAutomation"),
    ("sequence", "Sequence"),
    ("aiAgent", "AIAgent"),
    ("aiFunction", "AIFunction"),
    ("calendar", "AppointmentCalendar"),
    ("integrationWebchat", "IntegrationWebchat"),
    ("questionnaire", "Questionnaire"),
    ("flow", "Flow"),
    ("product", "Product"),
    ("pipeline", "Pipeline"),
    ("couponTopic", "CouponTopic"),
    ("savedReply", "SavedReply"),
    ("aiMcpServer", "AIMCPServer"),
    ("aiFile", "AIFile"),
];

/// `(target kind, table, column, holder kind, holder id column)`: a row of
/// `table` whose `column` is the copy's id (`INBOUND_REFERENCE_SLOTS`).
type ColumnProbe = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
);
const COLUMN_PROBES: &[ColumnProbe] = &[
    ("flow", "Reflink", "flowId", Some("reflink"), "id"),
    (
        "flow",
        "AutomatedResponse",
        "flowId",
        Some("automatedResponse"),
        "id",
    ),
    (
        "flow",
        "IntegrationWebchat",
        "welcomeFlowId",
        Some("integrationWebchat"),
        "id",
    ),
    (
        "flow",
        "AIFunction",
        "triggerFlowId",
        Some("aiFunction"),
        "id",
    ),
    (
        "flow",
        "SequenceStep",
        "flowId",
        Some("sequence"),
        "sequenceId",
    ),
    (
        "flow",
        "AppointmentCalendar",
        "confirmationFlowId",
        Some("calendar"),
        "id",
    ),
    (
        "flow",
        "AppointmentCalendar",
        "cancellationFlowId",
        Some("calendar"),
        "id",
    ),
    (
        "flow",
        "AppointmentCalendarReminder",
        "flowId",
        Some("calendar"),
        "calendarId",
    ),
    (
        "flow",
        "Questionnaire",
        "triggerFlowId",
        Some("questionnaire"),
        "id",
    ),
    ("flow", "Broadcast", "flowId", None, "id"),
    ("flow", "BroadcastTarget", "flowId", None, "id"),
    ("flow", "AIHandoverSettings", "gotoFlowId", None, "id"),
    (
        "flow",
        "EcommerceSettings",
        "abandonedCartFlowId",
        None,
        "id",
    ),
    ("flow", "EcommerceSettings", "checkoutFlowId", None, "id"),
    (
        "flow",
        "EcommerceSettings",
        "orderConfirmationFlowId",
        None,
        "id",
    ),
    ("flow", "IntegrationInstagram", "welcomeFlowId", None, "id"),
    ("flow", "IntegrationMessenger", "welcomeFlowId", None, "id"),
    (
        "flow",
        "IntegrationMessenger",
        "handoverResumeFlowId",
        None,
        "id",
    ),
    (
        "flow",
        "IntegrationWhatsapp",
        "handoverResumeFlowId",
        None,
        "id",
    ),
    ("flow", "IntegrationZalo", "fallbackFlowId", None, "id"),
    ("flow", "Leaderboard", "referralWelcomeFlowId", None, "id"),
    ("flow", "Poll", "afterVoteFlowId", None, "id"),
    ("aiAgent", "IntegrationOpenai", "aiAgentId", None, "id"),
    ("product", "OrderItem", "productId", None, "id"),
    ("product", "MetaCatalogItem", "productId", None, "id"),
    ("pipeline", "PipelineItem", "pipelineId", None, "id"),
];

/// `(table, holder kind, holder id column, JSON columns)`: rows whose JSON can
/// name a copy (`INBOUND_REFERENCE_CONTAINERS`, plus trigger condition sources).
type ContainerProbe = (
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static [&'static str],
);
const CONTAINER_PROBES: &[ContainerProbe] = &[
    ("FlowVersion", Some("flow"), "flowId", &["nodes", "edges"]),
    ("Trigger", Some("trigger"), "id", &["actions"]),
    (
        "Condition",
        Some("trigger"),
        "triggerId",
        &["value", "sourceId"],
    ),
    ("Broadcast", None, "id", &["contactFilter", "templateData"]),
    ("BroadcastTarget", None, "id", &["templateData"]),
    (
        "IntegrationMessenger",
        None,
        "id",
        &["persistentMenus", "conversationStarters"],
    ),
    (
        "IntegrationInstagram",
        None,
        "id",
        &["persistentMenus", "conversationStarters"],
    ),
    ("DiscountCode", None, "id", &["customerFilter"]),
    (
        "CommentAutomation",
        Some("fbCommentAutomation"),
        "id",
        &["privateReply", "publicReply", "options"],
    ),
    (
        "AIAgent",
        Some("aiAgent"),
        "id",
        &["tools", "actionRules", "messages"],
    ),
    (
        "IntegrationWebchat",
        Some("integrationWebchat"),
        "id",
        &["conversationStarters", "persistentMenus"],
    ),
];

/// A copy: `(resource kind, resource id)`.
type Key = (String, i64);
/// `(operation id, uninstall state, (installation id, attempt) to enqueue)`.
type RequestOutcome = (i64, String, Vec<(i64, i64)>);

fn too_large() -> BusinessError {
    BusinessError::exception(
        "This installation tracks too many items to uninstall in one go",
        "templateUninstallTooLarge",
        422,
    )
}

fn in_progress() -> BusinessError {
    BusinessError::exception(
        "This template is being uninstalled",
        "templateUninstallInProgress",
        409,
    )
}

fn not_granted() -> BusinessError {
    BusinessError::exception(
        "The installer did not allow this template to be uninstalled by its publisher",
        "templateUninstallNotGranted",
        403,
    )
}

fn not_cancellable() -> BusinessError {
    BusinessError::exception(
        "This uninstall can not be cancelled in its current state",
        "templateUninstallNotCancellable",
        409,
    )
}

fn table_of(kind: &str) -> Option<&'static str> {
    DELETE_ORDER
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, t)| *t)
}

/// `isSharedRow`: a manifest kind is kept unless the row's own category copies it.
fn is_shared(row: &TemplateInstalledResource) -> bool {
    if row.resource_kind == "pipeline" && row.category == "pipelines" {
        return false;
    }
    MANIFEST_KINDS.contains(&row.resource_kind.as_str()) || table_of(&row.resource_kind).is_none()
}

fn is_install_root(row: &TemplateInstalledResource) -> bool {
    row.resource_kind == "folder"
        && row
            .source_resource_id
            .starts_with(states::INSTALL_ROOT_PREFIX)
}

fn is_active(row: &TemplateInstallation) -> bool {
    matches!(row.status.as_str(), "completed" | "partial")
}

/// Whether `table` has `column` (probe tables of features a build may lack are skipped).
fn has_column(
    conn: &Connection,
    cache: &mut HashMap<String, HashSet<String>>,
    table: &str,
    column: &str,
) -> Result<bool> {
    if !cache.contains_key(table) {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(db_err)?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(db_err)?
            .collect::<chatbotx_db::rusqlite::Result<HashSet<_>>>()
            .map_err(db_err)?;
        cache.insert(table.to_string(), cols);
    }
    Ok(cache[table].contains(column))
}

/// The holder id column of a probe; `NULL` (installer data, which always
/// protects) when the holder is not a copy kind or the table lacks the column.
fn holder_select(
    conn: &Connection,
    cache: &mut HashMap<String, HashSet<String>>,
    table: &str,
    holder: Option<&str>,
    holder_col: &str,
) -> Result<String> {
    Ok(
        if holder.is_some() && has_column(conn, cache, table, holder_col)? {
            format!("\"{holder_col}\"")
        } else {
            "NULL".to_string()
        },
    )
}

/// The holders of rows referencing `(kind, id)`; `None` = installer data.
fn inbound(
    conn: &Connection,
    cache: &mut HashMap<String, HashSet<String>>,
    kind: &str,
    id: i64,
) -> Result<Vec<Option<(String, i64)>>> {
    let mut out = Vec::new();
    let mut collect = |sql: String, arg: String, holder: Option<&str>| -> Result<()> {
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let ids = stmt
            .query_map([arg], |r| r.get::<_, Option<i64>>(0))
            .map_err(db_err)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        for holder_id in ids {
            out.push(holder.zip(holder_id).map(|(k, h)| (k.to_string(), h)));
        }
        Ok(())
    };
    for (target, table, column, holder, holder_col) in COLUMN_PROBES {
        if *target != kind || !has_column(conn, cache, table, column)? {
            continue;
        }
        let select = holder_select(conn, cache, table, *holder, holder_col)?;
        // Text or integer columns alike: compared as text.
        collect(
            format!("SELECT {select} FROM \"{table}\" WHERE CAST(\"{column}\" AS TEXT) = ?1"),
            id.to_string(),
            *holder,
        )?;
    }
    for (table, holder, holder_col, columns) in CONTAINER_PROBES {
        for column in *columns {
            if !has_column(conn, cache, table, column)? {
                continue;
            }
            let select = holder_select(conn, cache, table, *holder, holder_col)?;
            collect(
                format!(
                    "SELECT {select} FROM \"{table}\" WHERE instr(CAST(\"{column}\" AS TEXT), ?1) > 0"
                ),
                id.to_string(),
                *holder,
            )?;
        }
    }
    Ok(out)
}

/// `decideUninstall`: the outcome of every undecided copy.
fn decide(
    conn: &Connection,
    workspace_id: i64,
    installation_id: i64,
    rows: &[TemplateInstalledResource],
) -> Result<Vec<(TemplateInstalledResource, &'static str)>> {
    let mut decisions = Vec::new();
    let mut candidates: BTreeMap<(String, i64), TemplateInstalledResource> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|r| !r.was_existing && r.detached_at.is_none() && r.uninstall_outcome.is_none())
    {
        if is_install_root(row) {
            decisions.push((row.clone(), "deleting"));
        } else if is_shared(row) {
            decisions.push((row.clone(), "keptShared"));
        } else {
            candidates.insert((row.resource_kind.clone(), row.resource_id), row.clone());
        }
    }
    // Another installation still locking a copy against deletion keeps it.
    let mut by_kind: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (kind, id) in candidates.keys() {
        by_kind.entry(kind.clone()).or_default().push(*id);
    }
    for (kind, ids) in by_kind {
        for id in resource_uninstall::find_delete_blocked_by_others(
            conn,
            workspace_id,
            &kind,
            &ids,
            &[installation_id],
        )? {
            if let Some(row) = candidates.remove(&(kind.clone(), id)) {
                decisions.push((row, "keptLockedByOther"));
            }
        }
    }
    // A kept dependent propagates "keep" to what it references, until stable.
    let mut cache = HashMap::new();
    let mut edges: HashMap<Key, Vec<Option<Key>>> = HashMap::new();
    for key in candidates.keys() {
        edges.insert(key.clone(), inbound(conn, &mut cache, &key.0, key.1)?);
    }
    loop {
        let protected: Vec<(String, i64)> = candidates
            .keys()
            .filter(|key| {
                edges[*key].iter().any(|holder| match holder {
                    None => true,
                    Some(h) => !candidates.contains_key(h),
                })
            })
            .cloned()
            .collect();
        if protected.is_empty() {
            break;
        }
        for key in protected {
            if let Some(row) = candidates.remove(&key) {
                decisions.push((row, "keptHasDependents"));
            }
        }
    }
    for row in candidates.into_values() {
        decisions.push((row.clone(), edit_state(conn, workspace_id, &row)?));
    }
    Ok(decisions)
}

/// `editStateOf` as the tombstone outcome: edits only word the counts.
fn edit_state(
    conn: &Connection,
    workspace_id: i64,
    row: &TemplateInstalledResource,
) -> Result<&'static str> {
    let Some(synced) = row.synced_hash.as_deref() else {
        return Ok("deletingUnverified");
    };
    if !UPDATABLE.contains(&row.category.as_str()) {
        return Ok("deletingUnverified");
    }
    Ok(
        match project_hash(conn, workspace_id, &row.category, row.resource_id)? {
            None => "deletingUnverified",
            Some(hash) if hash == synced => "deleting",
            Some(_) => "deletingModified",
        },
    )
}

/// `countDecisions` (install roots left out).
fn count(decisions: &[(TemplateInstalledResource, &'static str)]) -> Value {
    let mut counts = states::empty_counts();
    for (row, outcome) in decisions {
        if is_install_root(row) {
            continue;
        }
        let (key, extra) = match *outcome {
            "deleting" => ("deleted", None),
            "deletingModified" => ("deleted", Some("deletedModified")),
            "deletingUnverified" => ("deleted", Some("deletedUnverified")),
            other => (other, None),
        };
        for key in std::iter::once(key).chain(extra) {
            if let Some(n) = counts.get_mut(key) {
                *n = json!(n.as_i64().unwrap_or(0) + 1);
            }
        }
    }
    counts
}

fn preview_installation(conn: &Connection, row: &TemplateInstallation) -> Result<Value> {
    let rows = syncs::list_installed_resources(conn, row.id)?;
    let decided = states::count_outcomes(conn, row.id)?;
    let pending = count(&decide(conn, row.workspace_id, row.id, &rows)?);
    Ok(states::sum_counts(&decided, &pending))
}

/// `previewAsInstaller`: advisory counts by outcome.
pub async fn preview_as_installer(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    actor: &Actor,
) -> Result<Value> {
    assert_can_read(actor)?;
    tx::read(db, move |conn| {
        let row = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .filter(is_active)
            .ok_or_else(|| BusinessError::not_found("Template installation not found"))?;
        preview_installation(conn, &row)
    })
    .await
}

/// What an uninstall request recorded.
pub struct Requested {
    pub operation_id: i64,
    pub state: String,
    pub installations: usize,
}

/// `requestUninstall`: one operation for the installations of one
/// (template, workspace) pair; the jobs run after the commit.
fn request_in(
    conn: &Connection,
    workspace_id: i64,
    found: &[TemplateInstallation],
    requested_by_kind: &str,
    actor: &Actor,
) -> Result<RequestOutcome> {
    if requested_by_kind == "publisher" && found.iter().any(|r| r.publisher_uninstall_grant <= 0) {
        return Err(not_granted());
    }
    let in_flight: Vec<&TemplateInstallation> = found
        .iter()
        .filter(|r| matches!(r.uninstall_state.as_str(), "pending" | "running"))
        .collect();
    if let Some(first) = in_flight.first() {
        if let Some(operation) = first.uninstall_operation_id
            && in_flight.len() == found.len()
            && in_flight
                .iter()
                .all(|r| r.uninstall_operation_id == Some(operation))
        {
            return Ok((operation, first.uninstall_state.clone(), Vec::new()));
        }
        return Err(in_progress());
    }
    let ids: Vec<i64> = found.iter().map(|r| r.id).collect();
    let created: Vec<TemplateInstalledResource> =
        syncs::list_installed_resources_by_installation_ids(conn, &ids)?
            .into_iter()
            .filter(|r| !r.was_existing && r.detached_at.is_none())
            .collect();
    let mut copies: HashMap<i64, usize> = HashMap::new();
    for row in created.iter().filter(|r| !is_shared(r)) {
        *copies.entry(row.installation_id).or_default() += 1;
    }
    if copies.values().any(|n| *n > MAX_UNINSTALL_COPIES) {
        return Err(too_large());
    }
    assert_can_manage(actor)?;
    if requested_by_kind == "installer" {
        let categories: HashSet<&str> = created.iter().map(|r| r.category.as_str()).collect();
        assert_can_use_categories(actor, categories)?;
    }
    let first = &found[0];
    let operation = states::create_operation(
        conn,
        workspace_id,
        first.template_id,
        &first.publisher_ref,
        requested_by_kind,
        None,
    )?;
    let mut queued = Vec::new();
    for row in found {
        let (attempt, _) = states::request(conn, row.id, operation.id, requested_by_kind, None)?
            .ok_or_else(in_progress)?;
        states::reset_failed_to_deleting(conn, row.id)?;
        queued.push((row.id, attempt));
    }
    Ok((operation.id, "pending".into(), queued))
}

async fn queue(jobs: Option<&dyn Enqueuer>, workspace_id: i64, queued: Vec<(i64, i64)>) {
    let Some(jobs) = jobs else { return };
    for (id, attempt) in queued {
        if let Err(error) = super::save::enqueue_uninstall(jobs, id, workspace_id, attempt).await {
            tracing::warn!(%error, installation_id = id, "Could not enqueue the template uninstall");
        }
    }
}

/// `uninstallAsInstaller`.
pub async fn uninstall_as_installer(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    workspace_id: i64,
    installation_id: i64,
    actor: &Actor,
) -> Result<Requested> {
    assert_can_write_installs(actor)?;
    let actor = actor.clone();
    let (operation_id, state, queued) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let row = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .filter(is_active)
            .ok_or_else(|| BusinessError::not_found("Template installation not found"))?;
        request_in(conn, workspace_id, &[row], "installer", &actor)
    })
    .await?;
    queue(jobs, workspace_id, queued).await;
    Ok(Requested {
        operation_id,
        state,
        installations: 1,
    })
}

/// The publisher's template and the installer's active copies of it.
fn publisher_copies(
    conn: &Connection,
    workspace_id: i64,
    template_id: i64,
    publisher_ref: &str,
) -> Result<Vec<TemplateInstallation>> {
    templates::find_live(conn, template_id, workspace_id)?
        .ok_or_else(|| BusinessError::not_found("Template not found"))?;
    let rows = installs::list_active_by_publisher_ref(conn, template_id, publisher_ref)?;
    if rows.is_empty() {
        return Err(BusinessError::not_found("Template installer not found"));
    }
    Ok(rows)
}

/// `previewAsPublisher`: `{ installations, counts }` over the installer's copies.
pub async fn preview_as_publisher(
    db: &Db,
    workspace_id: i64,
    template_id: i64,
    publisher_ref: String,
    actor: &Actor,
) -> Result<Value> {
    assert_can_manage(actor)?;
    tx::read(db, move |conn| {
        let rows = publisher_copies(conn, workspace_id, template_id, &publisher_ref)?;
        if rows.iter().any(|r| r.publisher_uninstall_grant <= 0) {
            return Err(not_granted());
        }
        let mut counts = states::empty_counts();
        for row in &rows {
            counts = states::sum_counts(&counts, &preview_installation(conn, row)?);
        }
        Ok(json!({ "installations": rows.len(), "counts": counts }))
    })
    .await
}

/// `uninstallAsPublisher`: the jobs run in the installer's workspace.
pub async fn uninstall_as_publisher(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    workspace_id: i64,
    template_id: i64,
    publisher_ref: String,
    actor: &Actor,
) -> Result<Requested> {
    assert_can_manage(actor)?;
    let actor = actor.clone();
    let (target, installations, (operation_id, state, queued)) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let rows = publisher_copies(conn, workspace_id, template_id, &publisher_ref)?;
        let target = rows[0].workspace_id;
        let outcome = request_in(conn, target, &rows, "publisher", &actor)?;
        Ok((target, rows.len(), outcome))
    })
    .await?;
    queue(jobs, target, queued).await;
    Ok(Requested {
        operation_id,
        state,
        installations,
    })
}

/// `cancelAsInstaller`: a run that stopped (`partial`, `failed`, `skipped`)
/// is cancelled; the single-transaction run committed nothing to give back.
pub async fn cancel_as_installer(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    actor: &Actor,
) -> Result<Value> {
    assert_can_write_installs(actor)?;
    assert_can_manage(actor)?;
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let row = installs::find_by_id_and_workspace(conn, installation_id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Template installation not found"))?;
        if !(is_active(&row)
            && matches!(
                row.uninstall_state.as_str(),
                "partial" | "failed" | "skipped"
            ))
        {
            return Err(not_cancellable());
        }
        states::untombstone(conn, row.id)?;
        states::mark_cancelled(conn, row.id)?;
        Ok(json!({
            "restored": 0,
            "alreadyCommitted": 0,
            "keptPartlyRemoved": 0,
            "keptInactiveForReview": 0,
            "keptHistoryRemoved": 0,
        }))
    })
    .await
}

/// `uninstallTemplate`: claims the installation, decides and deletes in one
/// transaction, then finalizes (a failure is recorded as `failed`).
/// `runTemplateUninstall`: the ids of the deleted flows (the
/// `flow.welcomeCleanup` effect the caller runs after the commit).
pub async fn run_uninstall_job(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
) -> Result<Vec<i64>> {
    let token = chatbotx_db::repo::random_uuid();
    let claim = token.clone();
    let claimed = tx::write(db, move |tx| {
        Ok(states::claim(tx, installation_id, workspace_id, &claim)?)
    })
    .await?;
    if claimed.is_none() {
        return Ok(Vec::new());
    }
    let run_token = token.clone();
    let ran = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        execute(conn, workspace_id, installation_id, &run_token)
    })
    .await;
    let error = match ran {
        Ok(deleted_flows) => return Ok(deleted_flows),
        Err(error) => error,
    };
    let code = match &error {
        BusinessError::Exception { code, .. } => code.clone(),
        _ => "templateUninstallFailed".to_string(),
    };
    tracing::warn!(%error, installation_id, "Template uninstall failed");
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        states::finalize(
            conn,
            installation_id,
            &token,
            "failed",
            &states::empty_counts(),
            Some(code.as_str()),
        )?;
        if let Some(operation) = states::operation_id_of(conn, installation_id)? {
            states::finalize_operation_if_complete(conn, operation)?;
        }
        Ok(())
    })
    .await?;
    Ok(Vec::new())
}

fn execute(
    conn: &Connection,
    workspace_id: i64,
    installation_id: i64,
    token: &str,
) -> Result<Vec<i64>> {
    let rows = syncs::list_installed_resources(conn, installation_id)?;
    let decisions = decide(conn, workspace_id, installation_id, &rows)?;
    let doomed: HashSet<(String, i64)> = decisions
        .iter()
        .filter(|(row, o)| o.starts_with("deleting") && !is_install_root(row))
        .map(|(row, _)| (row.resource_kind.clone(), row.resource_id))
        .collect();
    let now = now_ms();
    let mut deleted_flows = Vec::new();
    for (kind, table) in DELETE_ORDER {
        let ids: Vec<i64> = doomed
            .iter()
            .filter(|(k, _)| k.as_str() == *kind)
            .map(|(_, id)| *id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        if *kind == "flow" {
            deleted_flows.extend(&ids);
        }
        match *kind {
            "integrationWebchat" => {
                for id in &ids {
                    crate::integration_webchat::delete_in(conn, workspace_id, *id, now)?;
                }
            }
            _ => {
                let marks = vec!["?"; ids.len()].join(", ");
                let mut args: Vec<i64> = vec![workspace_id];
                args.extend(&ids);
                conn.execute(
                    &format!(
                        "DELETE FROM \"{table}\" WHERE \"workspaceId\" = ? AND \"id\" IN ({marks})"
                    ),
                    params_from_iter(args),
                )
                .map_err(db_err)?;
            }
        }
    }
    // Install roots go once nothing is left in them.
    for (row, _) in decisions.iter().filter(|(r, _)| is_install_root(r)) {
        states::delete_folder_if_empty(conn, workspace_id, row.resource_id)?;
    }
    let mut by_outcome: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (row, outcome) in &decisions {
        let settled = match *outcome {
            "deleting" => "deleted",
            "deletingModified" => "deletedModified",
            "deletingUnverified" => "deletedUnverified",
            other => other,
        };
        by_outcome.entry(settled).or_default().push(row.id);
    }
    for (outcome, ids) in by_outcome {
        states::set_outcomes(conn, &ids, outcome, None, true)?;
    }
    let counts = states::count_outcomes(conn, installation_id)?;
    if !states::finalize(conn, installation_id, token, "done", &counts, None)? {
        return Err(BusinessError::exception(
            "The template uninstall lease was lost",
            "templateUninstallLeaseLost",
            500,
        ));
    }
    if let Some(operation) = states::operation_id_of(conn, installation_id)? {
        states::finalize_operation_if_complete(conn, operation)?;
    }
    Ok(deleted_flows)
}

/// `skipBlockedTemplateUninstall`.
pub async fn skip_blocked(db: &Db, installation_id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        states::mark_skipped(tx, installation_id, "workspaceOwnerBlocked")?;
        Ok(())
    })
    .await
}

/// `encodePublisherCursor` / `decodePublisherCursor`.
fn encode_cursor(last_installed_at: i64, publisher_ref: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{}|{publisher_ref}", iso_millis(last_installed_at)))
}

fn decode_cursor(cursor: &str) -> Option<(i64, String)> {
    let text = String::from_utf8(URL_SAFE_NO_PAD.decode(cursor).ok()?).ok()?;
    let (iso, publisher_ref) = text.split_once('|')?;
    let valid = (16..=64).contains(&publisher_ref.len())
        && publisher_ref
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    valid
        .then(|| parse_iso(iso))
        .flatten()
        .map(|at| (at, publisher_ref.to_string()))
}

/// `listPublisherInstallations`: one row per installer, newest install first.
pub async fn list_installers(
    db: &Db,
    workspace_id: i64,
    template_id: i64,
    actor: &Actor,
    search: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<Value> {
    assert_can_read(actor)?;
    tx::read(db, move |conn| {
        templates::find_live(conn, template_id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Template not found"))?;
        let limit = limit.unwrap_or(PUBLISHER_PAGE_SIZE).min(100);
        let rows = templates::list_publisher_rows(
            conn,
            &ListPublisherRows {
                template_id,
                search: search
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                after: cursor.as_deref().and_then(decode_cursor),
                limit: limit + 1,
            },
        )?;
        let more = rows.len() as i64 > limit;
        let page: Vec<_> = rows.into_iter().take(limit as usize).collect();
        let next = page.last().filter(|_| more).and_then(|r| {
            r.last_installed_at
                .map(|at| encode_cursor(at, &r.publisher_ref))
        });
        let items: Vec<Value> = page
            .iter()
            .map(|r| {
                json!({
                    "publisherRef": r.publisher_ref,
                    "workspaceName": r.workspace_name,
                    "installCount": r.install_count,
                    "totalInstallEvents": r.total_install_events,
                    "lastInstalledAt": r.last_installed_at.map(iso_millis),
                    "oldestVersion": r.oldest_version,
                    "autoUpdate": r.auto_update,
                    "active": r.active,
                    "uninstallState": r.uninstall_state,
                    "installedAutomatically": r.installed_automatically,
                    "eligibleForUninstall": r.eligible_for_uninstall,
                    "uninstallCounts": r.uninstall_counts,
                })
            })
            .collect();
        Ok(json!({ "items": items, "nextCursor": next }))
    })
    .await
}

/// `getInstallerIssues`: the installer's newest copy, its status and warnings
/// named after this workspace's own resources.
pub async fn installer_issues(
    db: &Db,
    workspace_id: i64,
    template_id: i64,
    publisher_ref: String,
    actor: &Actor,
) -> Result<Value> {
    assert_can_read(actor)?;
    tx::read(db, move |conn| {
        templates::find_live(conn, template_id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Template not found"))?;
        let row = installs::find_latest_by_publisher_ref(conn, template_id, &publisher_ref)?
            .ok_or_else(|| BusinessError::not_found("Template installer not found"))?;
        let warnings = row.warnings.0.as_array().cloned().unwrap_or_default();
        let groups = super::request::installer_warning_groups(conn, workspace_id, &warnings)?;
        Ok(json!({ "status": row.status, "errorCode": row.error_code, "warningGroups": groups }))
    })
    .await
}
