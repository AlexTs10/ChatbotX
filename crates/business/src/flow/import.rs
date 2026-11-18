//! Flow import (P3.6-D2 slice flow import): `importService.startFlowImport`,
//! `flowService.importFlowExport` (+ `customFieldService` /
//! `botFieldService.resolveByNameAndType`, `field-token-binder.ts`,
//! `sanitizeUnresolvedReferences`, `createFromImport`) and the worker
//! `runFlowImport` (`imports/flow-import.ts`).
//!
//! No default-queue worker is ported: the API runs [`run_flow_import`] in
//! the request after [`start_flow_import`] (the `bulkTagContacts`
//! precedent). There are no custom-field / bot-field caches to invalidate.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chatbotx_core::flow_config::authoring::FlowAuthoringError;
use chatbotx_core::flow_config::import_export::{
    RemapOptions, UnresolvedReference, collect_flow_reference_warnings,
    is_workspace_bound_reference_value, map_string_leaves, parse_flow_export,
    reference_kind_policy, reference_kinds_with_policy, remap_flow_graph_references,
    remove_unresolved_references, split_prefixed_token,
};
use chatbotx_core::utils::custom_field::{
    CustomFieldRequest, CustomFieldResolutionPlan, CustomFieldRow, CustomFieldType,
    custom_field_resolution_key, normalize_custom_field_name, pick_custom_field_row,
    plan_custom_field_resolution,
};
use chatbotx_core::utils::variables::variable_placeholder_re;
use chatbotx_core::variables::system_field::SystemFieldType;
use chatbotx_db::Db;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::reference_existence::{find_existing_ids, is_verifiable_reference_kind};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_files::Uploader;
use serde_json::{Value, json};

use super::{
    FlowGraph, assert_flow_graph_publishable, insert_analytics_session, insert_version, new_id,
    repo_err,
};
use crate::import::contacts::GENERIC_IMPORT_FAILURE;
use crate::import::{BYTES_PER_MB, ImportType, config};
use crate::{BusinessError, Result, tx};

/// The id of the root folder (`rootFolderId`).
const ROOT_FOLDER_ID: &str = "0";
const MAX_WARNING_SAMPLE: usize = 50;
const RAW_PREFIX: &str = "raw:";
const BOT_FIELD_PREFIX: &str = "bot_field:";

// --- importService.startFlowImport ------------------------------------

/// Why [`start_flow_import`] refused the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFlowImportRefusal {
    FileNotFound,
    NotAFlowImport,
}

/// `importService.startFlowImport`: marks the upload `uploaded` and inserts
/// the pending `flow` / `json` Import row (meta `{ folderId }`).
pub async fn start_flow_import(
    db: &Db,
    workspace_id: i64,
    user_id: Option<i64>,
    file_id: i64,
    folder_id: Option<i64>,
) -> Result<std::result::Result<i64, StartFlowImportRefusal>> {
    tx::write(db, move |conn| {
        let file: Option<(String, Option<String>)> = conn
            .query_row(
                r#"SELECT "contextType", "subType" FROM "File" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![file_id, workspace_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(repo_err)?;
        let Some((context, sub_type)) = file else {
            return Ok(Err(StartFlowImportRefusal::FileNotFound));
        };
        if context != "import" || sub_type.as_deref() != Some("flow") {
            return Ok(Err(StartFlowImportRefusal::NotAFlowImport));
        }
        let now = now_ms();
        conn.execute(
            r#"UPDATE "File" SET "status" = 'uploaded', "uploadedAt" = ?1, "updatedAt" = ?1 WHERE "id" = ?2 AND "workspaceId" = ?3"#,
            params![now, file_id, workspace_id],
        )
        .map_err(repo_err)?;
        let import_id = new_id();
        let meta = json!({ "folderId": folder_id.map(|id| id.to_string()) }).to_string();
        conn.execute(
            r#"INSERT INTO "Import" ("id", "workspaceId", "userId", "fileId", "type", "format", "status", "meta", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, 'flow', 'json', 'pending', ?5, ?6, ?6)"#,
            params![import_id, workspace_id, user_id, file_id, meta, now],
        )
        .map_err(repo_err)?;
        Ok(Ok(import_id))
    })
    .await
}

// --- resolveByNameAndType ----------------------------------------------

/// A manifest entry (`FlowExportCustomField` / `FlowExportBotField`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldManifestEntry {
    pub name: String,
    pub kind: CustomFieldType,
}

/// `ResolvedCustomField`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField {
    pub id: String,
    /// The target row's exact stored name.
    pub name: String,
    /// The requested name was taken by a field of another type.
    pub renamed: bool,
}

/// `resolveByNameAndType`'s result, keyed by `customFieldResolutionKey`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldResolution {
    pub id_map: BTreeMap<String, String>,
    pub created_ids: Vec<String>,
    pub resolved: HashMap<String, ResolvedField>,
}

/// Which field table a resolution writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldTable {
    CustomField,
    BotField,
}

/// `customFieldService` / `botFieldService.resolveByNameAndType`: a
/// same-typed field (by case-folded name) is reused, otherwise one is
/// created, as `<name>_<n>` when another type holds the name. The single
/// SQLite writer stands in for the name lock.
pub fn resolve_by_name_and_type(
    conn: &Connection,
    workspace_id: i64,
    table: FieldTable,
    fields: &[FieldManifestEntry],
) -> Result<FieldResolution> {
    let requests: Vec<CustomFieldRequest> = fields
        .iter()
        .map(|f| CustomFieldRequest {
            name: f.name.clone(),
            kind: f.kind,
        })
        .collect();
    if requests.is_empty() {
        return Ok(FieldResolution::default());
    }
    let table_name = match table {
        FieldTable::CustomField => "CustomField",
        FieldTable::BotField => "BotField",
    };
    let existing: Vec<CustomFieldRow> = {
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT "id", "name", "type" FROM "{table_name}" WHERE "workspaceId" = ?1"#
            ))
            .map_err(repo_err)?;
        stmt.query_map([workspace_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?
        .into_iter()
        .filter_map(|(id, name, kind)| {
            Some(CustomFieldRow {
                id: id.to_string(),
                name,
                kind: CustomFieldType::parse(&kind)?,
            })
        })
        .collect()
    };
    let plan = plan_custom_field_resolution(&existing, &requests);
    let requested_name = |key: &str| {
        plan.requests
            .iter()
            .find(|r| custom_field_resolution_key(&r.name, r.kind) == key)
            .map(|r| chatbotx_core::js::trim(&r.name).to_owned())
    };
    let mut by_key: HashMap<String, CustomFieldRow> = plan.winners.clone();
    let mut planned_name_by_key: HashMap<String, String> = HashMap::new();
    let mut created_ids = Vec::new();
    for field in &plan.requests {
        let key = custom_field_resolution_key(&field.name, field.kind);
        let planned = match plan.plans.get(&key) {
            Some(
                CustomFieldResolutionPlan::Create { planned_name }
                | CustomFieldResolutionPlan::Rename { planned_name },
            ) => planned_name.clone(),
            _ => continue,
        };
        let id = new_id();
        let inserted = match table {
            FieldTable::CustomField => conn.execute(
                r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type", "showInInbox") VALUES (?1, ?2, ?3, ?4, 1) ON CONFLICT DO NOTHING"#,
                params![id, workspace_id, planned, field.kind.as_str()],
            ),
            FieldTable::BotField => conn.execute(
                r#"INSERT INTO "BotField" ("id", "workspaceId", "name", "type") VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING"#,
                params![id, workspace_id, planned, field.kind.as_str()],
            ),
        }
        .map_err(repo_err)?;
        if inserted > 0 {
            created_ids.push(id.to_string());
            let row = CustomFieldRow {
                id: id.to_string(),
                name: planned.clone(),
                kind: field.kind,
            };
            let row_key = custom_field_resolution_key(&row.name, row.kind);
            let wanted = requested_name(&row_key);
            let picked =
                pick_custom_field_row(by_key.get(&row_key), &row, wanted.as_deref()).clone();
            by_key.insert(row_key, picked);
        }
        planned_name_by_key.insert(key, planned);
    }
    let mut out = FieldResolution {
        created_ids,
        ..FieldResolution::default()
    };
    for field in &plan.requests {
        let key = custom_field_resolution_key(&field.name, field.kind);
        let planned = planned_name_by_key.get(&key);
        let renamed = planned.is_some_and(|p| {
            normalize_custom_field_name(p) != normalize_custom_field_name(&field.name)
        });
        let row_key = planned.map_or_else(
            || key.clone(),
            |p| custom_field_resolution_key(p, field.kind),
        );
        if let Some(row) = by_key.get(&row_key) {
            out.id_map.insert(key.clone(), row.id.clone());
            out.resolved.insert(
                key,
                ResolvedField {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    renamed,
                },
            );
        }
    }
    Ok(out)
}

// --- field-token-binder.ts ---------------------------------------------

/// `customFieldNameFromToken`.
fn custom_field_name_from_token(token: &str) -> Option<&str> {
    let name = chatbotx_core::js::trim(token);
    if name.starts_with("coupon:")
        || name.starts_with(BOT_FIELD_PREFIX)
        || name.starts_with("pipelines.")
    {
        return None;
    }
    let custom = name.strip_prefix(RAW_PREFIX).unwrap_or(name);
    if custom.is_empty() || SystemFieldType::parse(name).is_some() {
        return None;
    }
    Some(custom)
}

/// `rewriteFieldTokensDeep`: `{{name}}` (and `{{raw:name}}`) follow the
/// bound target name.
pub fn rewrite_field_tokens_deep(value: &Value, bindings: &HashMap<String, String>) -> Value {
    if bindings.is_empty() {
        return value.clone();
    }
    map_string_leaves(value, &|text| {
        variable_placeholder_re()
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let inner = &caps[1];
                match custom_field_name_from_token(inner).and_then(|n| bindings.get(n)) {
                    Some(target) => {
                        let prefix = if chatbotx_core::js::trim(inner).starts_with(RAW_PREFIX) {
                            RAW_PREFIX
                        } else {
                            ""
                        };
                        format!("{{{{{prefix}{target}}}}}")
                    }
                    None => caps[0].to_owned(),
                }
            })
            .into_owned()
    })
}

/// `rewriteBotFieldTokensDeep`: `{{bot_field:<id>}}` follows the mapped id.
pub fn rewrite_bot_field_tokens_deep(value: &Value, ids: &BTreeMap<String, String>) -> Value {
    if ids.is_empty() {
        return value.clone();
    }
    map_string_leaves(value, &|text| {
        variable_placeholder_re()
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let inner = chatbotx_core::js::trim(&caps[1]);
                let target = split_prefixed_token(inner)
                    .filter(|(prefix, _)| format!("{prefix}:") == BOT_FIELD_PREFIX)
                    .and_then(|(_, id)| ids.get(chatbotx_core::js::trim(id)));
                match target {
                    Some(target) => format!("{{{{{BOT_FIELD_PREFIX}{target}}}}}"),
                    None => caps[0].to_owned(),
                }
            })
            .into_owned()
    })
}

/// One field of `computeCustomFieldNameBindings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingInput {
    pub source_id: String,
    pub name: String,
    pub kind: String,
    pub target: Option<ResolvedField>,
}

/// `{ from, to, type }` of a renamed field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedField {
    pub from: String,
    pub to: String,
    pub kind: String,
}

/// `computeCustomFieldNameBindings`: source name -> target name when they
/// differ (unless two source fields share the name), plus the renames.
pub fn compute_custom_field_name_bindings(
    fields: &[BindingInput],
) -> (HashMap<String, String>, Vec<RenamedField>) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for field in fields {
        *counts.entry(field.name.as_str()).or_default() += 1;
    }
    let mut bindings = HashMap::new();
    let mut renamed = Vec::new();
    for field in fields {
        let Some(target) = &field.target else {
            continue;
        };
        if target.renamed {
            renamed.push(RenamedField {
                from: field.name.clone(),
                to: target.name.clone(),
                kind: field.kind.clone(),
            });
        }
        if target.name == field.name || counts[field.name.as_str()] > 1 {
            continue;
        }
        bindings.insert(field.name.clone(), target.name.clone());
    }
    (bindings, renamed)
}

// --- sanitizeUnresolvedReferences --------------------------------------

/// `FLOW_IMPORT_VERIFIED_KINDS`.
const FLOW_IMPORT_VERIFIED_KINDS: &[&str] = &["tag", "pipeline", "folder", "productCategory"];

/// `flowImportReferenceKinds()`.
pub fn flow_import_reference_kinds() -> Vec<String> {
    let mut kinds: Vec<String> = ["customField", "botField"]
        .iter()
        .chain(FLOW_IMPORT_VERIFIED_KINDS)
        .chain(chatbotx_core::flow_config::import_export::STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS)
        .map(|k| (*k).to_owned())
        .collect();
    kinds.extend(reference_kinds_with_policy("verifyInTarget"));
    kinds
}

/// `sanitizeUnresolvedReferences` (clear mode) with `verifyKinds`: a
/// `naturalKey` / `strip` id no map resolved is removed; a `verifyInTarget`
/// (or verified-kind) id is removed unless it exists in the workspace.
pub(crate) fn sanitize_unresolved(
    conn: &Connection,
    workspace_id: i64,
    graph: &Value,
    unresolved: &[UnresolvedReference],
) -> Result<Value> {
    let ids: Vec<&UnresolvedReference> = unresolved
        .iter()
        .filter(|r| {
            let stripped =
                chatbotx_core::flow_config::import_export::STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS
                    .contains(&r.entity_kind.as_str())
                    && r.path.ends_with(']');
            !stripped && is_workspace_bound_reference_value(&r.entity_kind, &r.value)
        })
        .collect();
    let mut by_kind: BTreeMap<&str, Vec<&UnresolvedReference>> = BTreeMap::new();
    for r in ids {
        by_kind.entry(r.entity_kind.as_str()).or_default().push(r);
    }
    let mut remove: Vec<String> = Vec::new();
    for (kind, refs) in by_kind {
        let policy = if FLOW_IMPORT_VERIFIED_KINDS.contains(&kind) {
            Some("verifyInTarget")
        } else {
            reference_kind_policy().get(kind).and_then(Value::as_str)
        };
        match policy {
            Some("naturalKey" | "strip") => remove.extend(refs.iter().map(|r| r.path.clone())),
            Some("verifyInTarget") if is_verifiable_reference_kind(kind) => {
                let wanted: Vec<String> = refs
                    .iter()
                    .map(|r| r.value.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let existing = find_existing_ids(conn, workspace_id, kind, &wanted)
                    .map_err(BusinessError::Repo)?;
                remove.extend(
                    refs.iter()
                        .filter(|r| !existing.contains(&r.value))
                        .map(|r| r.path.clone()),
                );
            }
            _ => {}
        }
    }
    let paths: Vec<&str> = remove.iter().map(String::as_str).collect();
    Ok(remove_unresolved_references(graph, &paths))
}

// --- flowService.importFlowExport --------------------------------------

/// `importFlowExport` input.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportFlowExport {
    pub workspace_id: i64,
    pub name: String,
    pub active: bool,
    pub enable_in_inbox: bool,
    pub start_node_id: String,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    /// Source id -> manifest entry, in file order.
    pub custom_fields: Vec<(String, FieldManifestEntry)>,
    pub bot_fields: Vec<(String, FieldManifestEntry)>,
    pub folder_id: Option<String>,
}

/// `importFlowExport` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFlow {
    pub flow_id: i64,
    pub created_custom_field_ids: Vec<String>,
    pub created_bot_field_ids: Vec<String>,
    pub renamed_custom_fields: Vec<RenamedField>,
    pub renamed_bot_fields: Vec<RenamedField>,
}

fn source_id_map(
    entries: &[(String, FieldManifestEntry)],
    resolution: &FieldResolution,
) -> BTreeMap<String, String> {
    entries
        .iter()
        .filter_map(|(source, field)| {
            let key = custom_field_resolution_key(&field.name, field.kind);
            resolution
                .id_map
                .get(&key)
                .map(|target| (source.clone(), target.clone()))
        })
        .collect()
}

/// `flowService.importFlowExport`: resolves both field manifests, rewrites
/// `{{name}}` / `{{bot_field:<id>}}` tokens, remaps and sanitizes the
/// references, resolves the folder (a missing one is `null`) and inserts
/// the flow with its draft (`createFromImport`), all in one transaction.
pub async fn import_flow_export(db: &Db, input: ImportFlowExport) -> Result<ImportedFlow> {
    tx::write(db, move |conn| import_flow_export_tx(conn, &input)).await
}

fn import_flow_export_tx(conn: &Connection, input: &ImportFlowExport) -> Result<ImportedFlow> {
    let ws = input.workspace_id;
    let fields_of = |entries: &[(String, FieldManifestEntry)]| -> Vec<FieldManifestEntry> {
        entries.iter().map(|(_, f)| f.clone()).collect()
    };
    let customs = resolve_by_name_and_type(
        conn,
        ws,
        FieldTable::CustomField,
        &fields_of(&input.custom_fields),
    )?;
    let bots = resolve_by_name_and_type(
        conn,
        ws,
        FieldTable::BotField,
        &fields_of(&input.bot_fields),
    )?;
    let custom_map = source_id_map(&input.custom_fields, &customs);
    let bot_map = source_id_map(&input.bot_fields, &bots);
    let (bindings, renamed) = compute_custom_field_name_bindings(
        &input
            .custom_fields
            .iter()
            .map(|(source, field)| BindingInput {
                source_id: source.clone(),
                name: field.name.clone(),
                kind: field.kind.as_str().to_owned(),
                target: customs
                    .resolved
                    .get(&custom_field_resolution_key(&field.name, field.kind))
                    .cloned(),
            })
            .collect::<Vec<_>>(),
    );
    let graph = json!({ "nodes": input.nodes, "edges": input.edges });
    let graph =
        rewrite_bot_field_tokens_deep(&rewrite_field_tokens_deep(&graph, &bindings), &bot_map);
    let mut unresolved: Vec<UnresolvedReference> = Vec::new();
    let mut collect = |r: &UnresolvedReference| unresolved.push(r.clone());
    let maps = BTreeMap::from([
        ("customField".to_owned(), custom_map),
        ("botField".to_owned(), bot_map),
    ]);
    let remapped = remap_flow_graph_references(
        &graph,
        &maps,
        RemapOptions {
            kinds: Some(flow_import_reference_kinds()),
            on_unresolved: Some(&mut collect),
        },
    );
    let graph = sanitize_unresolved(conn, ws, &remapped, &unresolved)?;
    let nodes = graph["nodes"].clone();
    let edges = graph["edges"].clone();
    let requested_folder = input
        .folder_id
        .as_deref()
        .filter(|id| !id.is_empty() && *id != ROOT_FOLDER_ID)
        .and_then(|id| id.parse::<i64>().ok());
    let folder_id: Option<i64> = match requested_folder {
        Some(id) => conn
            .query_row(
                r#"SELECT "id" FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'flow'"#,
                params![id, ws],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?,
        None => None,
    };
    // `createFromImport` (validatePublishable, draft only).
    assert_flow_graph_publishable(&nodes, &edges)?;
    let flow_id = new_id();
    conn.execute(
        r#"INSERT INTO "Flow" ("id", "name", "active", "enableInInbox", "workspaceId", "folderId", "draftVersionId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)"#,
        params![flow_id, input.name, input.active, input.enable_in_inbox, ws, folder_id],
    )
    .map_err(repo_err)?;
    insert_analytics_session(conn, ws, flow_id)?;
    let draft_id = new_id();
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    insert_version(
        conn,
        draft_id,
        ws,
        flow_id,
        &FlowGraph {
            nodes: list(&nodes),
            edges: list(&edges),
            start_node_id: input.start_node_id.clone(),
            node_ids: None,
        },
        true,
        false,
    )?;
    conn.execute(
        r#"UPDATE "Flow" SET "draftVersionId" = ?1 WHERE "id" = ?2"#,
        params![draft_id, flow_id],
    )
    .map_err(repo_err)?;
    let renamed_bot_fields = input
        .bot_fields
        .iter()
        .filter_map(|(_, field)| {
            let target = bots
                .resolved
                .get(&custom_field_resolution_key(&field.name, field.kind))?;
            target.renamed.then(|| RenamedField {
                from: field.name.clone(),
                to: target.name.clone(),
                kind: field.kind.as_str().to_owned(),
            })
        })
        .collect();
    Ok(ImportedFlow {
        flow_id,
        created_custom_field_ids: customs.created_ids,
        created_bot_field_ids: bots.created_ids,
        renamed_custom_fields: renamed,
        renamed_bot_fields,
    })
}

// --- runFlowImport ------------------------------------------------------

/// `summarizeFlowImportValidationError` + `summarizeSchemaError`.
pub fn summarize_schema_error(errors: &[FlowAuthoringError]) -> String {
    let mut by_channel: Vec<(String, Vec<String>)> = Vec::new();
    for error in errors {
        let Some(cap) = &error.capability else {
            continue;
        };
        if cap.get("code").and_then(Value::as_str) != Some("unsupportedBlock") {
            continue;
        }
        let channel = cap
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let block = cap
            .get("block")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match by_channel.iter_mut().find(|(c, _)| *c == channel) {
            Some((_, steps)) => {
                if !steps.contains(&block) {
                    steps.push(block);
                }
            }
            None => by_channel.push((channel, vec![block])),
        }
    }
    if !by_channel.is_empty() {
        return by_channel
            .iter()
            .map(|(channel, steps)| {
                format!(
                    "Flow uses steps not supported by {channel}: {}.",
                    steps.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    if let Some(cap) = errors
        .iter()
        .filter_map(|e| e.capability.as_ref())
        .find(|c| c.get("code").and_then(Value::as_str) == Some("constraintExceeded"))
    {
        let text = |k: &str| {
            cap.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return format!(
            "Flow uses {}, which exceeds the {} limit.",
            text("block"),
            text("channel")
        );
    }
    match errors.first() {
        None => "The export file does not match the expected format.".to_owned(),
        Some(first) => {
            let path = if first.path.is_empty() {
                String::new()
            } else {
                format!(" at {}", first.path)
            };
            format!("Invalid export file{path}: {}", first.message)
        }
    }
}

fn error_samples(errors: &[FlowAuthoringError]) -> Value {
    Value::Array(
        errors
            .iter()
            .map(|e| {
                let mut sample = json!({ "reason": e.message, "path": e.path, "code": e.code });
                if let Some(cap) = &e.capability {
                    sample["capability"] = cap.clone();
                }
                sample
            })
            .collect(),
    )
}

/// The authoring errors of a `FlowAuthoringException` raised as a
/// [`BusinessError`] (`invalidRequestData`, the errors in `data`).
fn authoring_errors(error: &BusinessError) -> Option<Vec<FlowAuthoringError>> {
    match error {
        BusinessError::Exception {
            code,
            data: Some(data),
            ..
        } if code == "invalidRequestData" => serde_json::from_value(data.clone()).ok(),
        _ => None,
    }
}

/// `importService.fail` with a public message (`toPublicErrorMessage` of a
/// `ChatbotXException`) and optional `{1, 0, 1}` counters / sample.
async fn fail(db: &Db, import_id: i64, message: String, sample: Option<Value>) -> Result<()> {
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'failed', "errorMessage" = ?2, "completedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1"#,
            params![import_id, message, now],
        )
        .map_err(repo_err)?;
        if let Some(sample) = sample {
            conn.execute(
                r#"UPDATE "Import" SET "totalCount" = 1, "processedCount" = 1, "successCount" = 0, "failedCount" = 1, "errorSample" = ?2 WHERE "id" = ?1"#,
                params![import_id, sample.to_string()],
            )
            .map_err(repo_err)?;
        }
        Ok(())
    })
    .await
}

/// `readImportedJson`: the size check, then the JSON body.
async fn read_json(uploader: &Uploader, path: &str) -> std::result::Result<Value, String> {
    let max_mb = config(ImportType::Flow).max_file_size_mb;
    let max = u64::try_from(max_mb * BYTES_PER_MB).unwrap_or(u64::MAX);
    let too_large = || format!("File exceeds {max_mb}MB limit");
    if let Ok(head) = uploader.head_object(path).await
        && head.content_length > max
    {
        return Err(too_large());
    }
    // A read error is not a `ChatbotXException`: the generic message.
    let Ok(body) = uploader.get_object(path).await else {
        return Err(GENERIC_IMPORT_FAILURE.to_owned());
    };
    if body.len() as u64 > max {
        return Err(too_large());
    }
    serde_json::from_slice(&body).map_err(|_| "The file is not valid JSON.".to_owned())
}

fn manifest(value: &Value) -> Vec<(String, FieldManifestEntry)> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(source, field)| {
                    Some((
                        source.clone(),
                        FieldManifestEntry {
                            name: field.get("name")?.as_str()?.to_owned(),
                            kind: CustomFieldType::parse(field.get("type")?.as_str()?)?,
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// `runFlowImport` for the Import row `import_id` (a missing row is a
/// no-op).
pub async fn run_flow_import(db: &Db, uploader: &Uploader, import_id: i64) -> Result<()> {
    let row: Option<(i64, String, Option<String>)> = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT i."workspaceId", i."meta", f."path" FROM "Import" i LEFT JOIN "File" f ON f."id" = i."fileId" WHERE i."id" = ?1"#,
            [import_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let Some((workspace_id, meta, path)) = row else {
        return Ok(());
    };
    // `flowImportMetaSchema`: `{ folderId?: string | null }`.
    let meta: Value = serde_json::from_str(&meta).unwrap_or(Value::Null);
    let folder_id = match meta.get("folderId") {
        None | Some(Value::Null) if meta.is_object() => None,
        Some(Value::String(id)) => Some(id.clone()),
        _ => return fail(db, import_id, "Invalid flow import meta".into(), None).await,
    };
    let Some(path) = path else {
        return fail(db, import_id, "Associated file not found".into(), None).await;
    };
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'processing', "updatedAt" = ?2 WHERE "id" = ?1"#,
            params![import_id, now_ms()],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await?;
    let raw = match read_json(uploader, &path).await {
        Ok(raw) => raw,
        Err(message) => return fail(db, import_id, message, None).await,
    };
    let data = match parse_flow_export(&raw) {
        Ok(data) => data,
        Err(error) => {
            return fail(
                db,
                import_id,
                summarize_schema_error(&error.errors),
                Some(error_samples(&error.errors)),
            )
            .await;
        }
    };
    let flow = data["flows"][0].clone();
    let customs = manifest(&data["customFields"]);
    let bots = manifest(&data["botFields"]);
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let input = ImportFlowExport {
        workspace_id,
        name: flow["name"].as_str().unwrap_or_default().to_owned(),
        active: flow["active"].as_bool().unwrap_or_default(),
        enable_in_inbox: flow["enableInInbox"].as_bool().unwrap_or_default(),
        start_node_id: flow["startNodeId"].as_str().unwrap_or_default().to_owned(),
        nodes: list(&flow["nodes"]),
        edges: list(&flow["edges"]),
        custom_fields: customs.clone(),
        bot_fields: bots.clone(),
        folder_id,
    };
    let imported = match import_flow_export(db, input).await {
        Ok(imported) => imported,
        Err(error) => {
            tracing::error!(%error, import_id, "flow import insert failed");
            return match authoring_errors(&error) {
                Some(errors) => {
                    fail(
                        db,
                        import_id,
                        summarize_schema_error(&errors),
                        Some(error_samples(&errors)),
                    )
                    .await
                }
                None => fail(db, import_id, GENERIC_IMPORT_FAILURE.into(), None).await,
            };
        }
    };
    let custom_ids: HashSet<&str> = customs.iter().map(|(s, _)| s.as_str()).collect();
    let bot_ids: HashSet<&str> = bots.iter().map(|(s, _)| s.as_str()).collect();
    let warnings: Vec<UnresolvedReference> = collect_flow_reference_warnings(&flow)
        .into_iter()
        .filter(|w| {
            !((w.entity_kind == "customField" && custom_ids.contains(w.value.as_str()))
                || (w.entity_kind == "botField" && bot_ids.contains(w.value.as_str())))
        })
        .collect();
    let mut sample: Vec<Value> = imported
        .renamed_custom_fields
        .iter()
        .map(|r| json!({
            "path": "customFields",
            "reason": format!(
                "Custom field \"{}\" was created as \"{}\" ({}) because a field of another type already uses that name; {{{{{}}}}} in this flow now reads \"{}\".",
                r.from, r.to, r.kind, r.from, r.to
            ),
        }))
        .chain(imported.renamed_bot_fields.iter().map(|r| json!({
            "path": "botFields",
            "reason": format!(
                "Bot field \"{}\" was created as \"{}\" ({}) because a field of another type already uses that name.",
                r.from, r.to, r.kind
            ),
        })))
        .chain(warnings.iter().map(|w| json!({
            "path": w.path,
            "reason": format!(
                "{} reference at {} ({}) could not be kept in this workspace — it was cleared or left unset; repoint it manually.",
                w.entity_kind, w.path, w.value
            ),
        })))
        .collect();
    sample.truncate(MAX_WARNING_SAMPLE);
    let (n_custom, n_bot) = (
        imported.renamed_custom_fields.len(),
        imported.renamed_bot_fields.len(),
    );
    let warning = (!warnings.is_empty() || n_custom > 0 || n_bot > 0).then(|| {
        format!(
            "Imported with {} and {}{} — review them.",
            plural(warnings.len(), "unresolved reference"),
            plural(n_custom, "renamed custom field"),
            if n_bot > 0 {
                format!(" and {}", plural(n_bot, "renamed bot field"))
            } else {
                String::new()
            }
        )
    });
    let sample = Value::Array(sample).to_string();
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'completed', "completedAt" = ?2, "updatedAt" = ?2,
               "totalCount" = 1, "processedCount" = 1, "successCount" = 1, "failedCount" = 0,
               "errorSample" = ?3, "errorMessage" = ?4 WHERE "id" = ?1"#,
            params![import_id, now, sample, warning],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}
