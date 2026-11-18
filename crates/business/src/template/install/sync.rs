//! `sync-apply.ts`, `sync-pairs.ts` and the adapters' sync handlers
//! (`project`, `serializeBackup`, `restoreBackup`, `update`; Q.D4 slice 4),
//! in the caller's write transaction. New entries go in through the install
//! adapters; installed copies of an updatable category are overwritten in
//! place (an edited copy is backed up first); a copy the installer deleted is
//! detached; a copy the template no longer ships is detached.
//!
//! ponytail: a backup is the template-owned columns of the copy (plus the
//! trigger conditions, the sequence steps or the flow draft), read and written
//! through the install engine's column-typed writer, not per-domain services.

use std::collections::{HashMap, HashSet};

use chatbotx_db::repo::models::TemplateInstalledResource;
use chatbotx_db::repo::template_installation_sync as syncs;
use chatbotx_db::repo::template_resource_backup as backups;
use chatbotx_db::repo::{Json, new_id};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::{
    Ctx, FLOW_DEFERRED_KINDS, FLOW_INSERT_KINDS, INSTALL_ORDER, MANIFEST_KINDS, RunInput,
    agent_fields, apply_deferred, db_err, flow_graph, flush_tracked, insert_bot_fields,
    insert_category, insert_saved_replies, keyword_fields, pick, prepared_entries, project,
    project_hash, resolve_manifests, source_id, stamp_synced_hashes, text_of, trigger_actions,
    write_conditions,
};
use crate::errors::{BusinessError, Result};
use crate::flow::import::rewrite_field_tokens_deep;
use crate::template::permissions::{Actor, assert_can_use_categories};
use crate::template::snapshot::{canonical_text, tracked_kind};

/// The categories whose copies a sync overwrites; the rest only warn `notUpdatable`.
pub const UPDATABLE: &[&str] = &[
    "flows",
    "sequences",
    "triggers",
    "keywords",
    "settings",
    "aiAgents",
    "aiFunctions",
];

/// A backup larger than this is not taken, and the copy is left as it is.
const MAX_BACKUP_BYTES: usize = 1024 * 1024;
const BACKUP_FORMAT_VERSION: i64 = 1;

/// `templateBackupNotRestorableException`.
pub fn not_restorable() -> BusinessError {
    BusinessError::exception(
        "This backup can not be restored",
        "templateBackupNotRestorable",
        409,
    )
}

/// What one sync applies.
pub struct SyncInput<'p> {
    pub sync_id: i64,
    pub payload: &'p Value,
    pub version: i64,
    /// `{ allowEdit, allowDelete }` of the installation.
    pub permissions: Value,
    pub template_name: String,
}

/// What [`apply_version`] did.
pub struct SyncResult {
    pub status: &'static str,
    pub counts: Value,
    pub warnings: Vec<Value>,
}

#[derive(Default)]
struct Counts {
    created: i64,
    updated: i64,
    detached: i64,
    overwritten_modified: i64,
    not_updatable: i64,
}

/// `kindOf`: settings entries carry theirs, the rest have one per category.
fn entry_kind(category: &str, entry: &Value) -> Option<String> {
    tracked_kind(category)
        .map(str::to_string)
        .or_else(|| entry["kind"].as_str().map(str::to_string))
}

fn key(kind: &str, source: &str) -> String {
    format!("{kind}:{source}")
}

/// Whether `row` still has copies a past sync could not update (`hasLaggingUpdatableRows`).
pub fn has_lagging_updatable_rows(rows: &[TemplateInstalledResource], source_version: i64) -> bool {
    rows.iter().any(|row| {
        !row.was_existing
            && row.detached_at.is_none()
            && row.synced_version.is_some_and(|v| v < source_version)
            && UPDATABLE.contains(&row.category.as_str())
    })
}

/// `applyVersion`: maps what exists, resolves the manifests, inserts what is
/// new, syncs what is installed and detaches what was dropped.
pub fn apply_version(
    ctx: &mut Ctx<'_>,
    input: &SyncInput<'_>,
    actor: &Actor,
) -> Result<SyncResult> {
    let conn = ctx.conn;
    let rows = syncs::list_installed_resources(conn, ctx.installation_id)?;
    let mut by_key: HashMap<String, TemplateInstalledResource> = HashMap::new();
    for row in &rows {
        if row.detached_at.is_none() {
            ctx.map(&row.resource_kind, &row.source_resource_id, row.resource_id);
            if row.resource_kind == "flow" {
                register_draft_node_ids(ctx, row.resource_id)?;
            }
        }
        by_key.insert(
            key(&row.resource_kind, &row.source_resource_id),
            row.clone(),
        );
    }
    let payload = input.payload;
    for flow in payload["resources"]["flows"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for node in flow["nodes"].as_array().into_iter().flatten() {
            if let Some(id) = text_of(&node["id"]) {
                ctx.id_maps
                    .entry("flowNode".into())
                    .or_default()
                    .insert(id.clone(), id);
            }
        }
    }
    // ponytail: no new install root folder on a sync; a root entry a later
    // version adds lands at the workspace root.
    let run_input = RunInput {
        payload,
        template_name: input.template_name.clone(),
        create_install_folder: false,
        permissions: input.permissions.clone(),
        inbox_bindings: ctx.inbox_bindings.clone(),
        source_version: input.version,
    };
    resolve_manifests(ctx, &payload["manifests"], &run_input)?;
    let resources = rewrite_field_tokens_deep(&payload["resources"], &ctx.field_name_bindings);

    let mut counts = Counts::default();
    let mut shipped: HashSet<String> = HashSet::new();
    let mut plans: Vec<(&'static str, Vec<(Value, TemplateInstalledResource)>)> = Vec::new();
    let tracked_before = ctx.tracked.len();
    // Step 5: new items go in first, so an updated copy can point at one this version adds.
    let mut slots: Vec<(&'static str, &'static str)> =
        vec![("couponTopics", "couponTopics"), ("settings", "botFields")];
    for category in INSTALL_ORDER.iter().filter(|c| **c != "couponTopics") {
        let slot = if *category == "settings" {
            "savedReplies"
        } else {
            *category
        };
        slots.push((*category, slot));
    }
    for (category, slot) in slots {
        let entries: Vec<Value> = prepared_entries(ctx, &resources, category)
            .into_iter()
            .filter(|e| match slot {
                "botFields" => e["kind"] == "botField",
                "savedReplies" => e["kind"] != "botField",
                _ => true,
            })
            .collect();
        let allowed = assert_can_use_categories(actor, [category]).is_ok();
        let mut fresh = Vec::new();
        let mut pairs = Vec::new();
        for entry in entries {
            let Some(kind) = entry_kind(category, &entry) else {
                continue;
            };
            let k = key(&kind, &source_id(&entry));
            shipped.insert(k.clone());
            if !allowed {
                continue;
            }
            match by_key.get(&k) {
                None => fresh.push(entry),
                // A detached copy is never recreated; a reused row is never overwritten.
                Some(row) if row.detached_at.is_none() && !row.was_existing => {
                    pairs.push((entry, row.clone()));
                }
                Some(_) => {}
            }
        }
        if !fresh.is_empty() {
            match slot {
                "botFields" => insert_bot_fields(ctx, &fresh)?,
                "savedReplies" => insert_saved_replies(ctx, &fresh)?,
                _ => insert_category(ctx, category, &fresh)?,
            }
        }
        plans.push((category, pairs));
    }
    counts.created = ctx.tracked[tracked_before..]
        .iter()
        .filter(|t| !t.was_existing && !MANIFEST_KINDS.contains(&t.resource_kind.as_str()))
        .count() as i64;

    let mut updated: Vec<TemplateInstalledResource> = Vec::new();
    for (category, pairs) in plans {
        if pairs.is_empty() {
            continue;
        }
        if !UPDATABLE.contains(&category) {
            counts.not_updatable += pairs.len() as i64;
            ctx.warn(category, "notUpdatable", category, pairs.len().to_string());
            continue;
        }
        for (entry, row) in pairs {
            let Some(current) = project(conn, ctx.workspace_id, category, row.resource_id)? else {
                // Deleted by the installer meanwhile: never recreated, no longer governed.
                syncs::detach_resources(conn, &[row.id])?;
                counts.detached += 1;
                continue;
            };
            let hash = chatbotx_core::utils::crypto::sha256_hex(&canonical_text(&current));
            let modified = row.allow_edit && row.synced_hash.as_deref() != Some(hash.as_str());
            if modified && !back_up(ctx, input.sync_id, category, &row, &mut counts)? {
                continue;
            }
            update_entry(ctx, &row.resource_kind, &entry, row.resource_id)?;
            counts.updated += 1;
            updated.push(row);
        }
    }
    apply_deferred(ctx)?;
    flush_tracked(ctx, &input.permissions, payload)?;
    for row in &updated {
        let hash = project_hash(conn, ctx.workspace_id, &row.category, row.resource_id)?;
        syncs::mark_synced(conn, row.id, hash.as_deref(), input.version)?;
    }
    // Rule 6: a copy the template no longer ships stops being governed by it.
    let dropped: Vec<i64> = rows
        .iter()
        .filter(|r| {
            r.detached_at.is_none()
                && !MANIFEST_KINDS.contains(&r.resource_kind.as_str())
                && !shipped.contains(&key(&r.resource_kind, &r.source_resource_id))
        })
        .map(|r| r.id)
        .collect();
    if !dropped.is_empty() {
        syncs::detach_resources(conn, &dropped)?;
        counts.detached += dropped.len() as i64;
    }
    stamp_synced_hashes(conn, ctx.workspace_id, ctx.installation_id, input.version)?;
    let status = if ctx.warnings.is_empty() && counts.not_updatable == 0 {
        "completed"
    } else {
        "partial"
    };
    Ok(SyncResult {
        status,
        counts: json!({
            "created": counts.created,
            "updated": counts.updated,
            "detached": counts.detached,
            "overwrittenModified": counts.overwritten_modified,
            "notUpdatable": counts.not_updatable,
        }),
        warnings: ctx.warnings.clone(),
    })
}

/// `backUpModifiedCopy`: `false` leaves the copy as the installer has it.
fn back_up(
    ctx: &mut Ctx<'_>,
    sync_id: i64,
    category: &str,
    row: &TemplateInstalledResource,
    counts: &mut Counts,
) -> Result<bool> {
    let path = format!("{category}.{}", row.source_resource_id);
    let Some(content) = read_content(ctx, &row.resource_kind, row.resource_id)? else {
        ctx.warn(category, "backupUnavailable", path, row.id.to_string());
        return Ok(false);
    };
    let byte_size = content.to_string().len();
    if byte_size > MAX_BACKUP_BYTES {
        ctx.warn(category, "backupTooLarge", path, byte_size.to_string());
        return Ok(false);
    }
    let id = new_id();
    backups::insert(
        ctx.conn,
        &backups::NewTemplateResourceBackup {
            id,
            workspace_id: ctx.workspace_id,
            installation_id: ctx.installation_id,
            sync_id,
            installed_resource_id: row.id,
            resource_kind: row.resource_kind.clone(),
            resource_id: row.resource_id,
            format_version: BACKUP_FORMAT_VERSION,
            content: Json(content),
            byte_size: byte_size as i64,
        },
    )?;
    backups::prune_for_resource(ctx.conn, row.id)?;
    // The id `restoreBackup` takes, so the warning leads straight to the undo.
    ctx.warn(category, "localModifiedOverwritten", path, id.to_string());
    counts.overwritten_modified += 1;
    Ok(true)
}

/// The main row and the template-owned columns of each updatable kind.
fn row_spec(kind: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match kind {
        "automatedResponse" => (
            "AutomatedResponse",
            &["text", "keywords", "conditions", "flowId"],
        ),
        "savedReply" => ("SavedReply", &["shortcut", "text"]),
        "aiAgent" => (
            "AIAgent",
            &[
                "name",
                "prompt",
                "messages",
                "temperature",
                "maxOutputTokens",
                "isRichResponse",
                "tools",
                "models",
                "webSearchAuthorizedDomains",
            ],
        ),
        "aiFunction" => (
            "AIFunction",
            &[
                "name",
                "purpose",
                "dataCollect",
                "outputMessage",
                "triggerFlowId",
            ],
        ),
        "trigger" => ("Trigger", &["name", "actions"]),
        "sequence" => ("Sequence", &["name"]),
        "flow" => ("Flow", &["name"]),
        _ => return None,
    })
}

const STEP_COLUMNS: &[&str] = &[
    "order",
    "delayDays",
    "delayMinutes",
    "delayUnit",
    "specificDateTime",
    "isActive",
    "anytime",
    "sendTimeStart",
    "sendTimeEnd",
    "sendDays",
    "flowId",
];

/// Columns of one row as JSON (JSON columns parsed); `None` = no row.
fn read_columns(
    ctx: &mut Ctx<'_>,
    table: &str,
    cols: &[&str],
    where_sql: &str,
    arg: i64,
) -> Result<Vec<Map<String, Value>>> {
    let json_cols: HashSet<String> = ctx
        .columns(table)?
        .into_iter()
        .filter(|c| c.json)
        .map(|c| c.name)
        .collect();
    let list: Vec<String> = cols.iter().map(|c| format!("\"{c}\"")).collect();
    let mut stmt = ctx
        .conn
        .prepare(&format!(
            "SELECT {} FROM \"{table}\" WHERE {where_sql}",
            list.join(", ")
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([arg], |r| {
            let mut out = Map::new();
            for (i, col) in cols.iter().enumerate() {
                let value = match r.get::<_, Sql>(i)? {
                    Sql::Null => Value::Null,
                    Sql::Integer(n) => json!(n),
                    Sql::Real(f) => json!(f),
                    Sql::Text(t) if json_cols.contains(*col) => {
                        serde_json::from_str(&t).unwrap_or(Value::String(t))
                    }
                    Sql::Text(t) => Value::String(t),
                    Sql::Blob(_) => Value::Null,
                };
                out.insert((*col).to_string(), value);
            }
            Ok(out)
        })
        .map_err(db_err)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// `serializeBackup` content: what `update` is about to overwrite.
pub fn read_content(ctx: &mut Ctx<'_>, kind: &str, id: i64) -> Result<Option<Value>> {
    let Some((table, cols)) = row_spec(kind) else {
        return Ok(None);
    };
    let Some(mut row) = read_columns(ctx, table, cols, "\"id\" = ?1", id)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    match kind {
        "trigger" => {
            let conditions = read_columns(
                ctx,
                "Condition",
                &["type", "operator", "value", "sourceId"],
                "\"triggerId\" = ?1 ORDER BY \"id\"",
                id,
            )?;
            row.insert("conditions".into(), json!(conditions));
        }
        "sequence" => {
            let steps: Vec<Map<String, Value>> = read_columns(
                ctx,
                "SequenceStep",
                STEP_COLUMNS,
                "\"sequenceId\" = ?1 ORDER BY \"order\", \"id\"",
                id,
            )?
            .into_iter()
            .filter(|s| s["isActive"] == json!(1) || !s["flowId"].is_null())
            .collect();
            row.insert("steps".into(), json!(steps));
        }
        "flow" => {
            let Some(draft) = read_columns(
                ctx,
                "FlowVersion",
                &["startNodeId", "nodes", "edges"],
                "\"id\" = (SELECT \"draftVersionId\" FROM \"Flow\" WHERE \"id\" = ?1)",
                id,
            )?
            .into_iter()
            .next() else {
                return Ok(None);
            };
            row.extend(draft);
        }
        _ => {}
    }
    Ok(Some(Value::Object(row)))
}

/// `resolveSyncedName`: `wanted`, unless another row of the workspace has it.
fn synced_name(ctx: &mut Ctx<'_>, table: &str, id: i64, wanted: &Value) -> Result<Option<String>> {
    let Some(wanted) = wanted.as_str() else {
        return Ok(None);
    };
    let taken = ctx
        .conn
        .query_row(
            &format!(
                "SELECT 1 FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND \"name\" = ?2 AND \"id\" <> ?3 LIMIT 1"
            ),
            params![ctx.workspace_id, wanted, id],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_err)?
        .is_some();
    Ok((!taken).then(|| wanted.to_string()))
}

/// `update` of one copy from its new entry (references resolve through `ctx.id_maps`).
fn update_entry(ctx: &mut Ctx<'_>, kind: &str, entry: &Value, id: i64) -> Result<()> {
    let source = source_id(entry);
    match kind {
        "automatedResponse" => {
            let fields = keyword_fields(ctx, entry, &source);
            ctx.update("AutomatedResponse", id, fields)
        }
        "savedReply" => ctx.update("SavedReply", id, pick(entry, &["shortcut", "text"])),
        "aiAgent" => {
            let fields = agent_fields(ctx, entry, &source);
            ctx.update("AIAgent", id, fields)
        }
        "aiFunction" => {
            let mut fields = pick(entry, &["name", "purpose", "dataCollect", "outputMessage"]);
            let flow = ctx.resolve(
                "aiFunctions",
                "flow",
                format!("aiFunctions.{source}.triggerFlowId"),
                &entry["triggerFlowId"],
            );
            fields.insert("triggerFlowId".into(), json!(flow));
            ctx.update("AIFunction", id, fields)
        }
        "trigger" => {
            let (actions, _) = trigger_actions(ctx, entry, &source);
            let mut fields = Map::new();
            if let Some(name) = synced_name(ctx, "Trigger", id, &entry["name"])? {
                fields.insert("name".into(), json!(name));
            }
            fields.insert("actions".into(), actions);
            ctx.conn
                .execute(r#"DELETE FROM "Condition" WHERE "triggerId" = ?1"#, [id])
                .map_err(db_err)?;
            // `active` is the installer's, but a trigger that lost a condition
            // row would fire more broadly than intended.
            if write_conditions(ctx, entry, &source, id)? {
                fields.insert("active".into(), json!(false));
            }
            ctx.update("Trigger", id, fields)
        }
        "sequence" => {
            if let Some(name) = synced_name(ctx, "Sequence", id, &entry["name"])? {
                ctx.update("Sequence", id, pick(&json!({ "name": name }), &["name"]))?;
            }
            let mut steps = Vec::new();
            for step in entry["steps"].as_array().into_iter().flatten() {
                let mut fields = pick(step, STEP_COLUMNS);
                let order = step["order"].to_string();
                let flow = ctx.resolve(
                    "sequences",
                    "flow",
                    format!("sequences.{source}.steps.{order}.flowId"),
                    &step["flowId"],
                );
                fields.insert("flowId".into(), json!(flow));
                steps.push(fields);
            }
            write_steps(ctx, id, steps)
        }
        "flow" => {
            let kinds: Vec<&str> = FLOW_INSERT_KINDS
                .iter()
                .chain(FLOW_DEFERRED_KINDS)
                .copied()
                .collect();
            let (nodes, edges, all_resolved) = flow_graph(ctx, entry, &source, &kinds)?;
            if let Some(name) = entry["name"].as_str() {
                ctx.update("Flow", id, pick(&json!({ "name": name }), &["name"]))?;
            }
            let start = start_node(&entry["startNodeId"]);
            let had_published = write_draft(ctx, id, start, &nodes, &edges)?;
            if had_published {
                let publishable = all_resolved
                    && crate::flow::assert_flow_graph_publishable(&json!(nodes), &json!(edges))
                        .is_ok();
                if publishable {
                    republish(ctx, id, start, &nodes, &edges)?;
                } else {
                    ctx.warn(
                        "flows",
                        "flowNotPublished",
                        format!("flows.{source}"),
                        source,
                    );
                }
            }
            Ok(())
        }
        // Bot fields are reused by name and type; nothing of theirs is overwritten.
        _ => Ok(()),
    }
}

fn start_node(value: &Value) -> i64 {
    text_of(value)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

/// `writeSteps`: content rows first (a contact waiting on one keeps it); a
/// surplus row is switched off rather than deleted.
fn write_steps(ctx: &mut Ctx<'_>, sequence_id: i64, steps: Vec<Map<String, Value>>) -> Result<()> {
    let rows = read_columns(
        ctx,
        "SequenceStep",
        &["id", "isActive", "flowId"],
        "\"sequenceId\" = ?1 ORDER BY \"order\", \"id\"",
        sequence_id,
    )?;
    let is_content = |r: &Map<String, Value>| r["isActive"] == json!(1) || !r["flowId"].is_null();
    let existing: Vec<i64> = rows
        .iter()
        .filter(|r| is_content(r))
        .chain(rows.iter().filter(|r| !is_content(r)))
        .filter_map(|r| r["id"].as_i64())
        .collect();
    let count = steps.len();
    for (i, mut step) in steps.into_iter().enumerate() {
        match existing.get(i) {
            Some(row) => ctx.update("SequenceStep", *row, step)?,
            None => {
                step.insert("sequenceId".into(), json!(sequence_id));
                ctx.insert("SequenceStep", step)?;
            }
        }
    }
    for row in existing.iter().skip(count) {
        ctx.conn
            .execute(
                r#"UPDATE "SequenceStep" SET "isActive" = 0, "flowId" = NULL WHERE "id" = ?1"#,
                [row],
            )
            .map_err(db_err)?;
    }
    ctx.conn
        .execute(
            r#"UPDATE "Sequence" SET "messages" = ?1 WHERE "id" = ?2"#,
            params![count as i64, sequence_id],
        )
        .map_err(db_err)?;
    Ok(())
}

/// `writeDraft`: the flow's draft gets the graph; `true` when the flow has a
/// published version (the sync then republishes it).
fn write_draft(
    ctx: &mut Ctx<'_>,
    flow_id: i64,
    start: i64,
    nodes: &[Value],
    edges: &[Value],
) -> Result<bool> {
    let (draft, current): (Option<i64>, Option<i64>) = ctx
        .conn
        .query_row(
            r#"SELECT "draftVersionId", "currentVersionId" FROM "Flow" WHERE "id" = ?1"#,
            [flow_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_err)?;
    let (nodes, edges) = (Value::from(nodes.to_vec()), Value::from(edges.to_vec()));
    match draft {
        Some(draft) => {
            ctx.conn
                .execute(
                    r#"UPDATE "FlowVersion" SET "nodes" = ?1, "edges" = ?2, "startNodeId" = ?3 WHERE "id" = ?4"#,
                    params![nodes.to_string(), edges.to_string(), start, draft],
                )
                .map_err(db_err)?;
        }
        None => {
            let id = insert_version(ctx, flow_id, start, &nodes, &edges, true)?;
            ctx.conn
                .execute(
                    r#"UPDATE "Flow" SET "draftVersionId" = ?1 WHERE "id" = ?2"#,
                    params![id, flow_id],
                )
                .map_err(db_err)?;
        }
    }
    Ok(current.is_some())
}

fn insert_version(
    ctx: &mut Ctx<'_>,
    flow_id: i64,
    start: i64,
    nodes: &Value,
    edges: &Value,
    is_draft: bool,
) -> Result<i64> {
    let id = new_id();
    ctx.conn
        .execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                id,
                ctx.workspace_id,
                flow_id,
                nodes.to_string(),
                edges.to_string(),
                is_draft,
                !is_draft,
                start
            ],
        )
        .map_err(db_err)?;
    Ok(id)
}

/// `republish`: a new published version with the synced graph.
fn republish(
    ctx: &mut Ctx<'_>,
    flow_id: i64,
    start: i64,
    nodes: &[Value],
    edges: &[Value],
) -> Result<()> {
    ctx.conn
        .execute(
            r#"UPDATE "FlowVersion" SET "isLatest" = 0 WHERE "flowId" = ?1 AND "isDraft" = 0"#,
            [flow_id],
        )
        .map_err(db_err)?;
    let id = insert_version(
        ctx,
        flow_id,
        start,
        &Value::from(nodes.to_vec()),
        &Value::from(edges.to_vec()),
        false,
    )?;
    ctx.conn
        .execute(
            r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
            params![id, flow_id],
        )
        .map_err(db_err)?;
    ctx.published_flows
        .push(json!({ "flowId": flow_id.to_string(), "versionId": id.to_string() }));
    Ok(())
}

/// `registerInstalledCopyNodeIds`: a jump into an installed flow stays valid.
fn register_draft_node_ids(ctx: &mut Ctx<'_>, flow_id: i64) -> Result<()> {
    let nodes: Option<String> = ctx
        .conn
        .query_row(
            r#"SELECT v."nodes" FROM "FlowVersion" v JOIN "Flow" f ON f."draftVersionId" = v."id" WHERE f."id" = ?1"#,
            [flow_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let nodes: Value = nodes
        .and_then(|n| serde_json::from_str(&n).ok())
        .unwrap_or_default();
    for node in nodes.as_array().into_iter().flatten() {
        if let Some(id) = text_of(&node["id"]) {
            ctx.id_maps
                .entry("flowNode".into())
                .or_default()
                .insert(id.clone(), id);
        }
    }
    Ok(())
}

/// A flow id of the workspace, else `Null` (a restored reference may be gone).
fn existing_flow(ctx: &Ctx<'_>, value: &Value) -> Result<Value> {
    let Some(id) = text_of(value).and_then(|s| s.parse::<i64>().ok()) else {
        return Ok(Value::Null);
    };
    let found = ctx
        .conn
        .query_row(
            r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, ctx.workspace_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_err)?;
    Ok(if found.is_some() {
        json!(id)
    } else {
        Value::Null
    })
}

/// `restoreBackup`: puts a backup's content back (a flow into its draft only).
pub fn restore(
    ctx: &mut Ctx<'_>,
    kind: &str,
    id: i64,
    content: &Value,
    format_version: i64,
) -> Result<()> {
    let Some((table, cols)) = row_spec(kind) else {
        return Err(not_restorable());
    };
    if format_version != BACKUP_FORMAT_VERSION || !content.is_object() {
        return Err(not_restorable());
    }
    let mut fields = pick(content, cols);
    for col in ["flowId", "triggerFlowId"] {
        if let Some(value) = fields.get(col).cloned() {
            fields.insert(col.into(), existing_flow(ctx, &value)?);
        }
    }
    if matches!(kind, "trigger" | "sequence") {
        // The old name may have been taken since.
        match synced_name(ctx, table, id, &content["name"])? {
            Some(name) => fields.insert("name".into(), json!(name)),
            None => fields.remove("name"),
        };
    }
    ctx.update(table, id, fields)?;
    match kind {
        "trigger" => {
            ctx.conn
                .execute(r#"DELETE FROM "Condition" WHERE "triggerId" = ?1"#, [id])
                .map_err(db_err)?;
            for condition in content["conditions"].as_array().into_iter().flatten() {
                let mut fields = pick(condition, &["type", "operator", "value", "sourceId"]);
                fields.insert("triggerId".into(), json!(id));
                ctx.insert("Condition", fields)?;
            }
        }
        "sequence" => {
            let mut steps = Vec::new();
            for step in content["steps"].as_array().into_iter().flatten() {
                let mut fields = pick(step, STEP_COLUMNS);
                fields.insert("flowId".into(), existing_flow(ctx, &step["flowId"])?);
                steps.push(fields);
            }
            write_steps(ctx, id, steps)?;
        }
        "flow" => {
            let (Some(nodes), Some(edges)) =
                (content["nodes"].as_array(), content["edges"].as_array())
            else {
                return Err(not_restorable());
            };
            let start = start_node(&content["startNodeId"]);
            write_draft(ctx, id, start, nodes, edges)?;
        }
        _ => {}
    }
    Ok(())
}
