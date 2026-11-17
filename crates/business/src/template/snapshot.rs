//! `snapshot.service.ts`, `snapshot-collect.ts`, `snapshot-dropped.ts`,
//! `manifest-references.ts`, `trigger-conditions.ts` (references),
//! `trigger-field-keys.ts`, `item-permissions.ts`, `category-counts.ts`,
//! `payload-hash.ts`, `fingerprint.ts` and `version-changes.ts`: a selection
//! becomes a validated template payload. Sync code on one SQLite connection.

use std::collections::{BTreeMap, BTreeSet};

use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_core::flow_config::import_export::{
    TEMPLATE_EXPORT_FORMAT_VERSION, TEMPLATE_EXPORT_FORMAT_VERSION_V2, collect_references_by_kind,
    for_each_string_leaf, parse_template_export,
};
use chatbotx_core::utils::id::is_numeric_id;
use chatbotx_core::utils::variables::variable_placeholder_re;
use chatbotx_core::variables::system_field::SystemFieldType;
use chatbotx_db::repo::models as m;
use chatbotx_db::repo::template::{
    ManifestLookup, snapshot_find_custom_fields, snapshot_find_tags,
};
use chatbotx_db::repo::{RowJson, find_many, id_list, iso_millis, now_ms};
use chatbotx_db::rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::collect::{
    self, Collected, RESOURCE_CATEGORIES, RESOURCE_CATEGORY_ENUM, V2_CATEGORIES, numeric_ids,
};
use super::{Blocking, blocking_ids_in};
use crate::errors::{BusinessError, Result};

/// `MAX_PAYLOAD_RESOURCES`.
pub const MAX_PAYLOAD_RESOURCES: usize = 200;
/// `MAX_PAYLOAD_BYTES`.
pub const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

pub fn save_validation_error() -> BusinessError {
    BusinessError::exception(
        "One or more selected resources could not be found in this workspace",
        "templateSaveInvalidSelection",
        400,
    )
}

pub fn too_large_error() -> BusinessError {
    BusinessError::exception(
        "This template selection is too large to save",
        "templatePayloadTooLarge",
        400,
    )
}

fn too_many_bytes_error() -> BusinessError {
    BusinessError::exception(
        "This template's data is too large to save",
        "templatePayloadTooManyBytes",
        400,
    )
}

/// `templateIncludesViewOnlyCopyException`.
pub fn view_only_copy_error() -> BusinessError {
    BusinessError::exception(
        "A copy installed from a view-only template can not be included in a new template",
        "templateIncludesViewOnlyCopy",
        403,
    )
}

/// The kind each category's copies are tracked under (`trackedKind`).
pub fn tracked_kind(category: &str) -> Option<&'static str> {
    Some(match category {
        "flows" => "flow",
        "products" => "product",
        "aiFunctions" => "aiFunction",
        "aiAgents" => "aiAgent",
        "calendars" => "calendar",
        "webchats" => "integrationWebchat",
        "keywords" => "automatedResponse",
        "entryPointLinks" => "reflink",
        "triggers" => "trigger",
        "fbCommentAutomations" => "fbCommentAutomation",
        "sequences" => "sequence",
        "aiMcpServers" => "aiMcpServer",
        "aiFiles" => "aiFile",
        "questionnaires" => "questionnaire",
        "couponTopics" => "couponTopic",
        "pipelines" => "pipeline",
        _ => return None,
    })
}

/// `viewOnlyCheckKinds`.
pub fn view_only_check_kinds(category: &str) -> Vec<&'static str> {
    match (category, tracked_kind(category)) {
        (_, Some(kind)) => vec![kind],
        ("settings", None) => vec!["savedReply", "botField"],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// `ManifestReferences`.
#[derive(Debug, Default)]
pub struct ManifestReferences {
    pub tag_ids: BTreeSet<String>,
    pub custom_field_ids: BTreeSet<String>,
    pub custom_field_names: BTreeSet<String>,
    pub tag_names: BTreeSet<String>,
    pub bot_field_ids: BTreeSet<String>,
}

/// `customFieldNameFromToken`.
pub fn custom_field_name_from_token(token: &str) -> Option<String> {
    let name = token.trim();
    if name.starts_with("coupon:")
        || name.starts_with("bot_field:")
        || name.starts_with("pipelines.")
    {
        return None;
    }
    let custom = name.strip_prefix("raw:").unwrap_or(name);
    if custom.is_empty() || SystemFieldType::parse(name).is_some() {
        return None;
    }
    Some(custom.to_string())
}

fn for_each_token(value: &Value, visit: &mut dyn FnMut(&str)) {
    for_each_string_leaf(value, &mut |text| {
        for captures in variable_placeholder_re().captures_iter(text) {
            visit(captures.get(1).map_or("", |m| m.as_str()));
        }
    });
}

/// `collectBotFieldTokenIds`.
pub fn collect_bot_field_token_ids(value: &Value, ids: &mut BTreeSet<String>) {
    for_each_token(value, &mut |token| {
        if let Some(id) = token.trim().strip_prefix("bot_field:") {
            let id = id.trim();
            if is_numeric_id(id) {
                ids.insert(id.to_string());
            }
        }
    });
}

fn collect_step_tag_names(value: &Value, names: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|i| collect_step_tag_names(i, names)),
        Value::Object(record) => {
            let step_type = record.get("stepType").and_then(Value::as_str);
            if matches!(step_type, Some("addContactTag" | "removeContactTag"))
                && let Some(Value::Array(tags)) = record.get("tags")
            {
                for name in tags.iter().filter_map(Value::as_str) {
                    if !name.trim().is_empty() {
                        names.insert(name.trim().to_string());
                    }
                }
            }
            record
                .values()
                .for_each(|child| collect_step_tag_names(child, names));
        }
        _ => {}
    }
}

/// `triggerConditionRowSchema` over a whole list (any bad row = none).
fn condition_rows(value: &Value) -> Vec<&Value> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let valid = rows.iter().all(|row| {
        row.is_object()
            && row["type"].is_string()
            && (row["sourceId"].is_string() || row["sourceId"].is_null())
            && (row["operator"].is_string() || row["operator"].is_null())
            && row.get("sourceId").is_some()
            && row.get("operator").is_some()
    });
    if valid {
        rows.iter().collect()
    } else {
        Vec::new()
    }
}

/// `SOURCE_KIND_BY_EVENT_TYPE`.
fn condition_source_kind(event: &str) -> Option<&'static str> {
    Some(match event {
        "tagApplied" | "tagRemoved" => "tag",
        "customFieldValueChanged" | "dateTimeBasedTrigger" => "customField",
        "subscribedToSequence" | "unsubscribedFromSequence" => "sequence",
        "shoppingCartSent"
        | "userAskedAboutProduct"
        | "productOrdered"
        | "productAddedToCart"
        | "productRemovedFromCart" => "product",
        "categoryAddedToCart" => "productCategory",
        "pipelineItemCreated"
        | "pipelineItemMovedToStage"
        | "pipelineItemValueChanged"
        | "pipelineItemStatusChanged"
        | "pipelineItemPriorityChanged" => "pipeline",
        _ => return None,
    })
}

/// `referencesOf(row, { numericOnly: true })`.
fn condition_references(row: &Value) -> Vec<(&'static str, String)> {
    let usable = |id: &str| !id.is_empty() && is_numeric_id(id);
    let mut out = Vec::new();
    if let (Some(kind), Some(id)) = (
        row["type"].as_str().and_then(condition_source_kind),
        row["sourceId"].as_str(),
    ) && usable(id)
    {
        out.push((kind, id.to_string()));
    }
    if row["type"] == "pipelineItemMovedToStage"
        && let Some(stage) = row["value"]["stageId"].as_str()
        && usable(stage)
    {
        out.push(("pipelineStage", stage.to_string()));
    }
    out
}

/// `collectTriggerConditionReferences`.
pub fn trigger_condition_references(rows: &[Value]) -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut by_kind: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        for (kind, id) in condition_references(row) {
            by_kind.entry(kind).or_default().insert(id);
        }
    }
    by_kind
}

/// `collectManifestReferences`.
pub fn collect_manifest_references<'a>(
    entries: impl IntoIterator<Item = &'a Value>,
) -> ManifestReferences {
    let mut refs = ManifestReferences::default();
    for entry in entries {
        collect_step_tag_names(entry, &mut refs.tag_names);
        let by_kind = collect_references_by_kind(entry, Some(&["tag", "customField", "botField"]));
        for id in by_kind.get("botField").into_iter().flatten() {
            if is_numeric_id(id) {
                refs.bot_field_ids.insert(id.clone());
            }
        }
        collect_bot_field_token_ids(entry, &mut refs.bot_field_ids);
        refs.tag_ids
            .extend(by_kind.get("tag").into_iter().flatten().cloned());
        for id in by_kind.get("customField").into_iter().flatten() {
            if is_numeric_id(id) {
                refs.custom_field_ids.insert(id.clone());
            }
        }
        for_each_token(entry, &mut |token| {
            if let Some(name) = custom_field_name_from_token(token) {
                refs.custom_field_names.insert(name);
            }
        });
        let rows: Vec<Value> = condition_rows(&entry["conditions"])
            .into_iter()
            .cloned()
            .collect();
        for (kind, ids) in trigger_condition_references(&rows) {
            match kind {
                "tag" => refs.tag_ids.extend(ids),
                "customField" => refs.custom_field_ids.extend(ids),
                _ => {}
            }
        }
    }
    refs
}

/// `withTriggerFieldKeysAsIds`: `setCustomField` actions naming a field by
/// name (or by value) name it by id.
pub fn with_trigger_field_keys_as_ids(
    conn: &Connection,
    workspace_id: i64,
    action_lists: Vec<Value>,
) -> Result<Vec<Value>> {
    let named_key = |action: &Value| -> Option<String> {
        if action["type"] != "setCustomField" {
            return None;
        }
        let raw = action["customFieldId"].as_str()?;
        match parse_field_reference(raw) {
            FieldReference::BotField { .. } => None,
            FieldReference::CustomField { key } if chatbotx_db::repo::is_bigint_id(&key) => None,
            FieldReference::CustomField { key } => Some(key),
        }
    };
    let keys: Vec<String> = action_lists
        .iter()
        .flat_map(|list| list.as_array().into_iter().flatten().filter_map(named_key))
        .collect();
    if keys.is_empty() {
        return Ok(action_lists);
    }
    let fields = crate::custom_field::find_many_by_keys_in(conn, workspace_id, &keys)?;
    Ok(action_lists
        .into_iter()
        .map(|list| match list {
            Value::Array(actions) => Value::Array(
                actions
                    .into_iter()
                    .map(|mut action| {
                        if let Some(field) = named_key(&action).and_then(|k| fields.get(&k)) {
                            action["customFieldId"] = json!(field.id.to_string());
                        }
                        action
                    })
                    .collect(),
            ),
            other => other,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Selection and closure
// ---------------------------------------------------------------------------

/// Step 1 (`resolveSnapshotSelection`).
pub struct ResolvedSelection {
    pub ids_by_category: BTreeMap<&'static str, Vec<String>>,
    pub tag_ids: Vec<String>,
    pub custom_field_ids: Vec<String>,
}

fn dedupe(ids: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.iter().filter(|id| seen.insert(*id)).cloned().collect()
}

fn selection_ids(chosen: &Value) -> Vec<String> {
    chosen["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn resolve_category(
    conn: &Connection,
    workspace_id: i64,
    category: &str,
    chosen: &Value,
) -> Result<Vec<String>> {
    match chosen["mode"].as_str() {
        Some("all") => collect::resolve_ids(
            conn,
            workspace_id,
            category,
            MAX_PAYLOAD_RESOURCES as i64 + 1,
        ),
        Some("ids") => {
            let unique = dedupe(&selection_ids(chosen));
            if unique.is_empty() {
                return Ok(Vec::new());
            }
            let verified = collect::verify_ownership(conn, workspace_id, category, &unique)?;
            if verified.len() != unique.len() {
                return Err(save_validation_error());
            }
            // Keep the caller's order (the TS repo returns rows, not ids, in order).
            Ok(unique)
        }
        _ => Ok(Vec::new()),
    }
}

/// `resolveSnapshotSelection`.
pub fn resolve_selection(
    conn: &Connection,
    workspace_id: i64,
    selection: &Value,
) -> Result<ResolvedSelection> {
    let mut ids_by_category = BTreeMap::new();
    for category in RESOURCE_CATEGORIES {
        ids_by_category.insert(
            *category,
            resolve_category(conn, workspace_id, category, &selection[*category])?,
        );
    }
    let tag_ids = resolve_category(conn, workspace_id, "tags", &selection["tags"])?;
    let custom_field_ids = resolve_category(
        conn,
        workspace_id,
        "customFields",
        &selection["customFields"],
    )?;
    let total = ids_by_category.values().map(Vec::len).sum::<usize>()
        + tag_ids.len()
        + custom_field_ids.len();
    if total > MAX_PAYLOAD_RESOURCES {
        return Err(too_large_error());
    }
    Ok(ResolvedSelection {
        ids_by_category,
        tag_ids,
        custom_field_ids,
    })
}

pub type CollectedByCategory = BTreeMap<&'static str, Collected>;

fn collected_count(results: &CollectedByCategory) -> usize {
    results.values().map(|r| r.entries.len()).sum()
}

fn all_entries(results: &CollectedByCategory) -> impl Iterator<Item = &Value> {
    results.values().flat_map(|r| r.entries.iter())
}

/// `collectSnapshotClosure`: collect, fold the hard dependencies, re-collect
/// the categories that gained ids, until a pass adds nothing.
pub fn collect_closure(
    conn: &Connection,
    workspace_id: i64,
    ids_by_category: &mut BTreeMap<&'static str, Vec<String>>,
) -> Result<CollectedByCategory> {
    let mut results = CollectedByCategory::new();
    let mut pending: BTreeSet<&'static str> = RESOURCE_CATEGORIES.iter().copied().collect();
    while !pending.is_empty() {
        let mut dependencies = Vec::new();
        for category in &pending {
            let result =
                collect::collect(conn, workspace_id, category, &ids_by_category[category])?;
            dependencies.extend(result.hard_dependencies.iter().cloned());
            results.insert(category, result);
        }
        let bot_fields = collect_manifest_references(all_entries(&results)).bot_field_ids;
        dependencies.extend(bot_fields.into_iter().map(|id| ("settings", id)));
        pending = BTreeSet::new();
        for (category, id) in dependencies {
            let Some(ids) = ids_by_category.get_mut(category) else {
                continue;
            };
            if !ids.contains(&id) {
                ids.push(id);
                pending.insert(category);
            }
        }
        if collected_count(&results) > MAX_PAYLOAD_RESOURCES {
            return Err(too_large_error());
        }
    }
    Ok(results)
}

fn assert_no_view_only_copies(
    conn: &Connection,
    workspace_id: i64,
    ids_by_category: &BTreeMap<&'static str, Vec<String>>,
) -> Result<()> {
    for (category, ids) in ids_by_category {
        let numeric = numeric_ids(ids);
        if numeric.is_empty() {
            continue;
        }
        for kind in view_only_check_kinds(category) {
            if !blocking_ids_in(conn, workspace_id, kind, &numeric, Blocking::Edit)?.is_empty() {
                return Err(view_only_copy_error());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifests
// ---------------------------------------------------------------------------

fn json_rows<M: chatbotx_db::repo::Model + RowJson>(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<Value>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(find_many::<M>(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
        &[&workspace_id, &id_list(ids)],
    )?
    .iter()
    .map(RowJson::row_json)
    .collect())
}

/// `collectFolderAncestry` / `collectProductCategoryAncestry`: walk parents to the root.
fn ancestry<M: chatbotx_db::repo::Model + RowJson>(
    conn: &Connection,
    workspace_id: i64,
    start: Vec<String>,
    shape: impl Fn(&Value) -> Value,
) -> Result<Map<String, Value>> {
    let mut manifest = Map::new();
    let mut pending = start;
    while !pending.is_empty() {
        let fetch: Vec<String> = pending
            .drain(..)
            .filter(|id| !manifest.contains_key(id))
            .collect();
        if fetch.is_empty() {
            break;
        }
        for row in json_rows::<M>(conn, workspace_id, &numeric_ids(&fetch))? {
            let id = row["id"].as_str().unwrap_or_default().to_string();
            manifest.insert(id, shape(&row));
            if let Some(parent) = row["parentId"].as_str()
                && !manifest.contains_key(parent)
            {
                pending.push(parent.to_string());
            }
        }
    }
    Ok(manifest)
}

fn pipeline_manifest(
    conn: &Connection,
    workspace_id: i64,
    flow_graphs: &[Value],
    trigger_conditions: &[Value],
) -> Result<Map<String, Value>> {
    let mut referenced: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for graph in flow_graphs {
        for (kind, ids) in
            collect_references_by_kind(graph, Some(&["pipeline", "pipelineStage", "pipelineField"]))
        {
            referenced.entry(kind).or_default().extend(ids);
        }
    }
    let from_triggers = trigger_condition_references(trigger_conditions);
    let mut pipeline_ids: BTreeSet<String> = referenced.remove("pipeline").unwrap_or_default();
    pipeline_ids.extend(from_triggers.get("pipeline").cloned().unwrap_or_default());
    let mut stage_ids = referenced.remove("pipelineStage").unwrap_or_default();
    stage_ids.extend(
        from_triggers
            .get("pipelineStage")
            .cloned()
            .unwrap_or_default(),
    );
    let field_ids = referenced.remove("pipelineField").unwrap_or_default();
    let stage_ids: Vec<String> = stage_ids.into_iter().collect();
    let field_ids: Vec<String> = field_ids.into_iter().collect();
    for row in json_rows::<m::PipelineStage>(conn, workspace_id, &numeric_ids(&stage_ids))?
        .iter()
        .chain(json_rows::<m::PipelineField>(conn, workspace_id, &numeric_ids(&field_ids))?.iter())
    {
        if let Some(id) = row["pipelineId"].as_str() {
            pipeline_ids.insert(id.to_string());
        }
    }
    if pipeline_ids.is_empty() {
        return Ok(Map::new());
    }
    let ids: Vec<String> = pipeline_ids.into_iter().collect();
    let pipelines = json_rows::<m::Pipeline>(conn, workspace_id, &numeric_ids(&ids))?;
    let entries = collect::pipeline_entries(conn, workspace_id, &pipelines, false)?;
    Ok(pipelines
        .iter()
        .zip(entries)
        .map(|(p, e)| (p["id"].as_str().unwrap_or_default().to_string(), e))
        .collect())
}

fn place_category_manifest(
    conn: &Connection,
    workspace_id: i64,
    flow_graphs: &[Value],
) -> Result<Map<String, Value>> {
    let mut ids = BTreeSet::new();
    for graph in flow_graphs {
        ids.extend(
            collect_references_by_kind(graph, Some(&["placeCategory"]))
                .remove("placeCategory")
                .unwrap_or_default(),
        );
    }
    let ids: Vec<String> = ids.into_iter().collect();
    Ok(
        json_rows::<m::PlaceCategory>(conn, workspace_id, &numeric_ids(&ids))?
            .iter()
            .map(|r| {
                (
                    r["id"].as_str().unwrap_or_default().to_string(),
                    json!({ "name": r["name"] }),
                )
            })
            .collect(),
    )
}

fn inbox_manifest(
    conn: &Connection,
    workspace_id: i64,
    inbox_ids: &[String],
) -> Result<Map<String, Value>> {
    Ok(chatbotx_db::repo::inbox::list_identities_by_ids(
        conn,
        workspace_id,
        &numeric_ids(inbox_ids),
    )?
    .into_iter()
    .map(|row| {
        (
            row.id.to_string(),
            json!({ "channelType": row.channel, "name": row.name }),
        )
    })
    .collect())
}

// ---------------------------------------------------------------------------
// Dropped references
// ---------------------------------------------------------------------------

const SHIPPED_CATEGORY_BY_KIND: &[(&str, &str)] = &[
    ("flow", "flows"),
    ("trigger", "triggers"),
    ("sequence", "sequences"),
    ("aiAgent", "aiAgents"),
    ("aiFunction", "aiFunctions"),
    ("aiFile", "aiFiles"),
    ("aiMcpServer", "aiMcpServers"),
    ("calendar", "calendars"),
    ("questionnaire", "questionnaires"),
    ("couponTopic", "couponTopics"),
];

/// `countDroppedResourceReferences`.
fn count_dropped_resource_references<'a>(
    entries: impl IntoIterator<Item = &'a Value>,
    shipped: &BTreeMap<&str, BTreeSet<String>>,
) -> usize {
    let mut kinds: Vec<&str> = SHIPPED_CATEGORY_BY_KIND.iter().map(|(k, _)| *k).collect();
    kinds.extend([
        "pipeline",
        "pipelineStage",
        "pipelineField",
        "productCategory",
        "placeCategory",
        "botField",
    ]);
    let mut dropped = BTreeSet::new();
    for entry in entries {
        // Trigger condition rows are counted separately.
        let mut entry = entry.clone();
        if let Some(object) = entry.as_object_mut()
            && object.contains_key("conditions")
        {
            object.remove("conditions");
        }
        for (kind, ids) in collect_references_by_kind(&entry, Some(&kinds)) {
            let carried = shipped.get(kind.as_str());
            for id in ids {
                if is_numeric_id(&id) && !carried.is_some_and(|c| c.contains(&id)) {
                    dropped.insert(format!("{kind}:{id}"));
                }
            }
        }
        let mut token_ids = BTreeSet::new();
        collect_bot_field_token_ids(&entry, &mut token_ids);
        for id in token_ids {
            if !shipped.get("botField").is_some_and(|c| c.contains(&id)) {
                dropped.insert(format!("botField:{id}"));
            }
        }
    }
    dropped.len()
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// `BuildSnapshotResult`.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub payload: Value,
    pub category_counts: Value,
    pub dropped_reference_count: usize,
}

/// `buildItemPermissions`.
fn item_permissions(
    selection: &Value,
    present: &BTreeMap<&str, Vec<String>>,
) -> Result<Map<String, Value>> {
    let mut permissions = Map::new();
    let Some(selection) = selection.as_object() else {
        return Ok(permissions);
    };
    for (category, chosen) in selection {
        let present_ids = present.get(category.as_str()).cloned().unwrap_or_default();
        let locked: Vec<String> = if chosen["mode"] == "all" {
            if chosen["lockAll"] == true {
                present_ids
            } else {
                Vec::new()
            }
        } else {
            let ids = selection_ids(chosen);
            let locked: Vec<String> = chosen["lockedIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if locked.iter().any(|id| !ids.contains(id)) {
                return Err(BusinessError::exception(
                    "A locked item must be one of the selected items",
                    "templateSaveInvalidSelection",
                    400,
                ));
            }
            locked
                .into_iter()
                .filter(|id| present_ids.contains(id))
                .collect()
        };
        if !locked.is_empty() {
            permissions.insert(
                category.clone(),
                locked
                    .into_iter()
                    .map(|id| (id, json!({ "allowEdit": false })))
                    .collect(),
            );
        }
    }
    Ok(permissions)
}

fn ids_of(entries: &[Value]) -> BTreeSet<String> {
    entries
        .iter()
        .filter_map(|e| e["sourceId"].as_str().map(str::to_string))
        .collect()
}

fn folder_ref(row_folder: Option<i64>, mut entry: Value) -> Value {
    if let Some(folder) = row_folder {
        entry["folderSourceId"] = json!(folder.to_string());
    }
    entry
}

/// `buildTemplateSnapshot`.
pub fn build_snapshot(
    conn: &Connection,
    workspace_id: i64,
    tenant_id: i64,
    selection: &Value,
) -> Result<Snapshot> {
    let mut resolved = resolve_selection(conn, workspace_id, selection)?;
    let results = collect_closure(conn, workspace_id, &mut resolved.ids_by_category)?;
    assert_no_view_only_copies(conn, workspace_id, &resolved.ids_by_category)?;

    // `resolveReferencedManifestIds`.
    let references = collect_manifest_references(all_entries(&results));
    let named_fields = snapshot_find_custom_fields(
        conn,
        &ManifestLookup {
            workspace_id,
            names: Some(references.custom_field_names.iter().cloned().collect()),
            ..Default::default()
        },
    )?;
    let mut by_name: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for field in &named_fields {
        by_name.entry(&field.name).or_default().push(field.id);
    }
    let named_tags = snapshot_find_tags(
        conn,
        &ManifestLookup {
            workspace_id,
            names: Some(references.tag_names.iter().cloned().collect()),
            ..Default::default()
        },
    )?;
    let mut referenced_tag_ids: Vec<String> = references
        .tag_ids
        .iter()
        .filter(|id| is_numeric_id(id))
        .cloned()
        .collect();
    referenced_tag_ids.extend(named_tags.iter().map(|t| t.id.to_string()));
    let referenced_tag_ids = dedupe(&referenced_tag_ids);
    let mut referenced_field_ids: Vec<String> =
        references.custom_field_ids.iter().cloned().collect();
    referenced_field_ids.extend(
        by_name
            .values()
            .filter(|ids| ids.len() == 1)
            .map(|ids| ids[0].to_string()),
    );
    let tag_ids = dedupe(&[resolved.tag_ids.clone(), referenced_tag_ids.clone()].concat());
    let custom_field_ids = dedupe(
        &[
            resolved.custom_field_ids.clone(),
            referenced_field_ids.clone(),
        ]
        .concat(),
    );
    if collected_count(&results) + tag_ids.len() + custom_field_ids.len() > MAX_PAYLOAD_RESOURCES {
        return Err(too_large_error());
    }

    let empty = Collected::default();
    let result = |category: &str| results.get(category).unwrap_or(&empty);
    let trigger_conditions: Vec<Value> = result("triggers")
        .entries
        .iter()
        .flat_map(|e| e["conditions"].as_array().cloned().unwrap_or_default())
        .collect();
    let flow_graphs: Vec<Value> = result("flows")
        .entries
        .iter()
        .map(|e| {
            json!({
                "nodes": if e["nodes"].is_array() { e["nodes"].clone() } else { json!([]) },
                "edges": if e["edges"].is_array() { e["edges"].clone() } else { json!([]) },
            })
        })
        .collect();
    let to_i64 = |ids: &[String]| numeric_ids(ids);
    let tag_rows = snapshot_find_tags(
        conn,
        &ManifestLookup {
            workspace_id,
            ids: Some(to_i64(&tag_ids)),
            ..Default::default()
        },
    )?;
    let field_rows = snapshot_find_custom_fields(
        conn,
        &ManifestLookup {
            workspace_id,
            ids: Some(to_i64(&custom_field_ids)),
            ..Default::default()
        },
    )?;
    let mut folder_ids: Vec<String> = results
        .values()
        .flat_map(|r| r.folder_ids.iter().cloned())
        .collect();
    folder_ids.extend(
        tag_rows
            .iter()
            .filter_map(|t| t.folder_id)
            .chain(field_rows.iter().filter_map(|f| f.folder_id))
            .map(|id| id.to_string()),
    );
    let folders = ancestry::<m::Folder>(
        conn,
        workspace_id,
        dedupe(&folder_ids),
        |r| json!({ "name": r["name"], "folderType": r["folderType"], "parentSourceId": r["parentId"] }),
    )?;
    let category_ids: Vec<String> = results
        .values()
        .flat_map(|r| r.product_category_ids.iter().cloned())
        .collect();
    let product_categories =
        ancestry::<m::ProductCategory>(conn, workspace_id, dedupe(&category_ids), |r| {
            let mut entry = json!({ "name": r["name"], "parentSourceId": r["parentId"] });
            if r["rank"] != 10 {
                entry["rank"] = r["rank"].clone();
            }
            entry
        })?;
    let pipelines = pipeline_manifest(conn, workspace_id, &flow_graphs, &trigger_conditions)?;
    let places = place_category_manifest(conn, workspace_id, &flow_graphs)?;
    let inbox_ids: Vec<String> = results
        .values()
        .flat_map(|r| r.inbox_ids.iter().cloned())
        .collect();
    let inboxes = inbox_manifest(conn, workspace_id, &dedupe(&inbox_ids))?;

    // `countDroppedReferences`.
    let found_tags: BTreeSet<String> = tag_rows.iter().map(|t| t.id.to_string()).collect();
    let found_fields: BTreeSet<String> = field_rows.iter().map(|f| f.id.to_string()).collect();
    let selected_pipelines = &result("pipelines").entries;
    let pipeline_values: Vec<&Value> = pipelines
        .values()
        .chain(selected_pipelines.iter())
        .collect();
    let mut pipeline_set: BTreeSet<String> = pipelines.keys().cloned().collect();
    pipeline_set.extend(ids_of(selected_pipelines));
    let nested_ids = |key: &str| -> BTreeSet<String> {
        pipeline_values
            .iter()
            .flat_map(|p| p[key].as_array().cloned().unwrap_or_default())
            .filter_map(|s| s["sourceId"].as_str().map(str::to_string))
            .collect()
    };
    let stage_set = nested_ids("stages");
    let field_set = nested_ids("fields");
    let product_category_set: BTreeSet<String> = product_categories.keys().cloned().collect();
    let place_set: BTreeSet<String> = places.keys().cloned().collect();
    let dropped_manifest = referenced_tag_ids
        .iter()
        .filter(|id| !found_tags.contains(*id))
        .count()
        + dedupe(&referenced_field_ids)
            .iter()
            .filter(|id| !found_fields.contains(*id))
            .count();
    let condition_shipped: BTreeMap<&str, &BTreeSet<String>> = BTreeMap::new();
    let sequences_set = ids_of(&result("sequences").entries);
    let products_set = ids_of(&result("products").entries);
    let mut condition_shipped = condition_shipped;
    condition_shipped.insert("sequence", &sequences_set);
    condition_shipped.insert("product", &products_set);
    condition_shipped.insert("productCategory", &product_category_set);
    condition_shipped.insert("pipeline", &pipeline_set);
    condition_shipped.insert("pipelineStage", &stage_set);
    let dropped_conditions = trigger_conditions
        .iter()
        .filter(|row| {
            condition_references(row)
                .iter()
                .any(|(kind, id)| condition_shipped.get(kind).is_some_and(|c| !c.contains(id)))
        })
        .count();
    let mut shipped: BTreeMap<&str, BTreeSet<String>> = SHIPPED_CATEGORY_BY_KIND
        .iter()
        .map(|(kind, category)| (*kind, ids_of(&result(category).entries)))
        .collect();
    shipped.insert("botField", ids_of(&result("settings").entries));
    shipped.insert("pipeline", pipeline_set.clone());
    shipped.insert("pipelineStage", stage_set.clone());
    shipped.insert("pipelineField", field_set);
    shipped.insert("productCategory", product_category_set.clone());
    shipped.insert("placeCategory", place_set);
    let dropped = dropped_manifest
        + dropped_conditions
        + count_dropped_resource_references(all_entries(&results), &shipped);

    let mut present: BTreeMap<&str, Vec<String>> = RESOURCE_CATEGORIES
        .iter()
        .map(|c| {
            (
                *c,
                result(c)
                    .entries
                    .iter()
                    .filter_map(|e| e["sourceId"].as_str().map(str::to_string))
                    .collect(),
            )
        })
        .collect();
    present.insert("tags", tag_rows.iter().map(|t| t.id.to_string()).collect());
    present.insert(
        "customFields",
        field_rows.iter().map(|f| f.id.to_string()).collect(),
    );
    present.insert(
        "productCategories",
        product_categories.keys().cloned().collect(),
    );
    let permissions = item_permissions(selection, &present)?;

    let mut resources = Map::new();
    for category in RESOURCE_CATEGORY_ENUM {
        let entries = &result(category).entries;
        resources.insert(
            (*category).into(),
            if *category == "settings" {
                json!({
                    "savedReplies": entries.iter().filter(|e| e["kind"] == "savedReply").collect::<Vec<_>>(),
                    "botFields": entries.iter().filter(|e| e["kind"] == "botField").collect::<Vec<_>>(),
                })
            } else {
                json!(entries)
            },
        );
    }
    let needs_v2 = !permissions.is_empty()
        || !inboxes.is_empty()
        || V2_CATEGORIES.iter().any(|c| !result(c).entries.is_empty());
    let mut payload = json!({
        "formatVersion": if needs_v2 { TEMPLATE_EXPORT_FORMAT_VERSION_V2 } else { TEMPLATE_EXPORT_FORMAT_VERSION },
        "exportedAt": iso_millis(now_ms()),
        "source": { "workspaceId": workspace_id.to_string(), "tenantId": tenant_id.to_string() },
        "manifests": {
            "customFields": field_rows.iter().map(|f| (f.id.to_string(), folder_ref(f.folder_id, json!({ "name": f.name, "type": f.r#type })))).collect::<Map<_, _>>(),
            "tags": tag_rows.iter().map(|t| (t.id.to_string(), folder_ref(t.folder_id, json!({ "name": t.name })))).collect::<Map<_, _>>(),
            "productCategories": product_categories,
            "folders": folders,
            "pipelines": pipelines,
            "placeCategories": places,
            "inboxes": inboxes,
        },
        "resources": resources,
    });
    if !permissions.is_empty() {
        payload["permissions"] = Value::Object(permissions);
    }
    let parsed = parse_template_export(&payload).map_err(|reason| {
        BusinessError::exception(
            format!("Template snapshot failed validation: {reason}"),
            "templateSnapshotInvalid",
            400,
        )
    })?;
    if parsed.to_string().len() > MAX_PAYLOAD_BYTES {
        return Err(too_many_bytes_error());
    }
    Ok(Snapshot {
        category_counts: category_counts(&parsed),
        payload: parsed,
        dropped_reference_count: dropped,
    })
}

/// `categoryCountsOfPayload`.
pub fn category_counts(payload: &Value) -> Value {
    let len = |v: &Value| v.as_array().map_or(0, Vec::len) + v.as_object().map_or(0, Map::len);
    let resources = &payload["resources"];
    let manifests = &payload["manifests"];
    let mut counts = Map::new();
    for key in ["tags", "customFields", "productCategories"] {
        counts.insert(key.into(), json!(len(&manifests[key])));
    }
    counts.insert(
        "settings".into(),
        json!(
            len(&resources["settings"]["savedReplies"]) + len(&resources["settings"]["botFields"])
        ),
    );
    for category in RESOURCE_CATEGORY_ENUM.iter().filter(|c| **c != "settings") {
        counts.insert((*category).into(), json!(len(&resources[*category])));
    }
    Value::Object(counts)
}

// ---------------------------------------------------------------------------
// Hash and version changes
// ---------------------------------------------------------------------------

/// `canonicalize`: keys sorted (serde_json maps are sorted already).
pub fn canonical_text(value: &Value) -> String {
    value.to_string()
}

/// The keys an installer owns per category (`fingerprint.project`).
fn installer_owned(category: &str, entry: &Value) -> &'static [&'static str] {
    match category {
        "flows" => &["active", "enableInInbox", "folderId"],
        "aiAgents" => &["isDefault"],
        "keywords" => &["folderId"],
        "triggers" => &["active", "folderId"],
        "sequences" => &["active", "folderId", "subscribers", "messages"],
        "settings" if entry["kind"] == "botField" => &["folderId"],
        _ => &[],
    }
}

/// `unorderedPaths` per category.
fn unordered_paths(category: &str) -> &'static [&'static str] {
    match category {
        "aiAgents" => &["webSearchAuthorizedDomains"],
        "keywords" => &["keywords"],
        _ => &[],
    }
}

/// `fingerprintEntry` with the category's fingerprint.
pub fn fingerprint_entry(category: &str, entry: &Value) -> Value {
    let mut out = entry.clone();
    if let Some(object) = out.as_object_mut() {
        for key in installer_owned(category, entry) {
            object.remove(*key);
        }
        for path in unordered_paths(category) {
            if let Some(Value::Array(items)) = object.get_mut(*path) {
                items.sort_by_key(canonical_text);
            }
        }
    }
    out
}

/// `hashTemplatePayload`: sha256 of the canonical content (no envelope keys).
pub fn hash_payload(payload: &Value) -> String {
    let mut content = Map::new();
    for (key, value) in payload.as_object().into_iter().flatten() {
        if key == "exportedAt" || key == "source" {
            continue;
        }
        let projected = if key == "resources" {
            let mut resources = Map::new();
            for (category, list) in value.as_object().into_iter().flatten() {
                let project = |c: &str, v: &Value| match v {
                    Value::Array(items) => {
                        Value::Array(items.iter().map(|e| fingerprint_entry(c, e)).collect())
                    }
                    other => other.clone(),
                };
                let out = if category == "settings" && list.is_object() {
                    Value::Object(
                        list.as_object()
                            .into_iter()
                            .flatten()
                            .map(|(k, v)| (k.clone(), project("settings", v)))
                            .collect(),
                    )
                } else {
                    project(category, list)
                };
                resources.insert(category.clone(), out);
            }
            Value::Object(resources)
        } else {
            value.clone()
        };
        content.insert(key.clone(), projected);
    }
    chatbotx_core::utils::crypto::sha256_hex(&canonical_text(&Value::Object(content)))
}

/// `hashTemplateContent`: the hash of the payload as the schema reads it.
pub fn hash_content(payload: &Value) -> String {
    match parse_template_export(payload) {
        Ok(parsed) => hash_payload(&parsed),
        Err(_) => hash_payload(payload),
    }
}

/// `entriesForCategory`.
pub fn entries_for_category(resources: &Value, category: &str) -> Vec<Value> {
    if category == "settings" {
        let tag = |list: &str, kind: &str| -> Vec<Value> {
            resources["settings"][list]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    let mut e = e.clone();
                    e["kind"] = json!(kind);
                    e
                })
                .collect()
        };
        return [
            tag("savedReplies", "savedReply"),
            tag("botFields", "botField"),
        ]
        .concat();
    }
    resources[category].as_array().cloned().unwrap_or_default()
}

/// `describeVersionChanges`.
pub fn describe_version_changes(previous: &Value, next: &Value) -> Value {
    let items = |payload: &Value, category: &str| -> BTreeMap<String, (String, String)> {
        let rows: Vec<(String, Value)> =
            if matches!(category, "tags" | "customFields" | "productCategories") {
                payload["manifests"][category]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(id, e)| (id.clone(), e.clone()))
                    .collect()
            } else {
                entries_for_category(&payload["resources"], category)
                    .into_iter()
                    .map(|e| (e["sourceId"].as_str().unwrap_or_default().to_string(), e))
                    .collect()
            };
        rows.into_iter()
            .map(|(id, e)| {
                let name = ["name", "shortcut", "title"]
                    .iter()
                    .find_map(|k| e[*k].as_str().filter(|s| !s.is_empty()))
                    .unwrap_or(&id)
                    .to_string();
                (id, (name, canonical_text(&e)))
            })
            .collect()
    };
    let mut changes = Vec::new();
    for category in collect::CATEGORIES {
        let before = items(previous, category);
        let after = items(next, category);
        let mut added = Vec::new();
        let mut changed = Vec::new();
        for (id, (name, content)) in &after {
            match before.get(id) {
                None => added.push(name.clone()),
                Some((_, earlier)) if earlier != content => changed.push(name.clone()),
                _ => {}
            }
        }
        let removed: Vec<String> = before
            .iter()
            .filter(|(id, _)| !after.contains_key(*id))
            .map(|(_, (name, _))| name.clone())
            .collect();
        if !added.is_empty() || !changed.is_empty() || !removed.is_empty() {
            changes.push(json!({ "category": category, "added": added, "changed": changed, "removed": removed }));
        }
    }
    Value::Array(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_names_skip_system_and_prefixed_fields() {
        assert_eq!(
            custom_field_name_from_token(" raw:budget "),
            Some("budget".into())
        );
        assert_eq!(custom_field_name_from_token("first_name"), None);
        assert_eq!(custom_field_name_from_token("coupon:7"), None);
        let refs = collect_manifest_references([&json!({
            "text": "Hi {{budget}} {{bot_field:12}} {{first_name}}",
            "step": { "stepType": "addContactTag", "tags": [" vip "] },
            "conditions": [{ "type": "tagApplied", "sourceId": "5", "operator": null, "value": null }],
        })]);
        assert!(refs.custom_field_names.contains("budget"));
        assert!(refs.bot_field_ids.contains("12"));
        assert!(refs.tag_names.contains("vip"));
        assert!(refs.tag_ids.contains("5"));
    }

    #[test]
    fn hash_ignores_envelope_and_installer_fields() {
        let a = json!({ "exportedAt": "x", "source": {}, "resources": { "flows": [{ "sourceId": "1", "active": true }] } });
        let b = json!({ "exportedAt": "y", "source": { "a": 1 }, "resources": { "flows": [{ "sourceId": "1", "active": false }] } });
        assert_eq!(hash_payload(&a), hash_payload(&b));
        let c = json!({ "resources": { "flows": [{ "sourceId": "2" }] } });
        assert_ne!(hash_payload(&a), hash_payload(&c));
    }

    #[test]
    fn version_changes_by_name() {
        let before = json!({ "manifests": { "tags": { "1": { "name": "a" } } }, "resources": { "flows": [{ "sourceId": "9", "name": "F", "nodes": [] }] } });
        let after = json!({ "manifests": { "tags": {} }, "resources": { "flows": [{ "sourceId": "9", "name": "F", "nodes": [1] }, { "sourceId": "10", "name": "G" }] } });
        assert_eq!(
            describe_version_changes(&before, &after),
            json!([
                { "category": "flows", "added": ["G"], "changed": ["F"], "removed": [] },
                { "category": "tags", "added": [], "changed": [], "removed": ["a"] },
            ])
        );
    }

    #[test]
    fn item_permissions_lock_only_selected() {
        let present = BTreeMap::from([("flows", vec!["1".to_string(), "2".to_string()])]);
        let ok = item_permissions(
            &json!({ "flows": { "mode": "ids", "ids": ["1", "3"], "lockedIds": ["1", "3"] } }),
            &present,
        )
        .unwrap();
        assert_eq!(
            Value::Object(ok),
            json!({ "flows": { "1": { "allowEdit": false } } })
        );
        assert!(
            item_permissions(
                &json!({ "flows": { "mode": "ids", "ids": ["1"], "lockedIds": ["2"] } }),
                &present
            )
            .is_err()
        );
        let all = item_permissions(
            &json!({ "flows": { "mode": "all", "lockAll": true } }),
            &present,
        )
        .unwrap();
        assert_eq!(all["flows"].as_object().map(Map::len), Some(2));
    }
}
