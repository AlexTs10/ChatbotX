//! `install.service.ts`, `adapters/manifests/**` and every adapter's `insert`
//! (Q.D4 slice 3). One SQLite write transaction runs the manifests (Phase R),
//! the adapters in [`INSTALL_ORDER`] (Phase 1) and the deferred flow
//! references (Phase 2), writes the `TemplateInstalledResource` rows and
//! completes the lease-fenced installation. A per-resource problem is a
//! warning, never a half-written install.
//!
//! ponytail: rows are written through one column-typed JSON writer
//! ([`Ctx::insert`]) instead of each domain service; storage-backed objects
//! (AI files, saved-reply images, questionnaire images) are not copied and
//! warn (`savedReplyImage`, `questionImage`, `aiFileUnavailable`; no object
//! staging yet); a webchat gets a fresh embed
//! credential. See DECISIONS.md (2026-11-17 Q.D4 slice 3).

use std::collections::{BTreeMap, HashMap};

use chatbotx_core::flow_config::import_export::{
    ReferenceIdMaps, RemapOptions, UnresolvedReference, map_string_leaves,
    remap_flow_graph_references, remap_references,
};
use chatbotx_core::utils::custom_field::{CustomFieldType, custom_field_resolution_key};
use chatbotx_core::utils::variables::variable_placeholder_re;
use chatbotx_db::repo::{new_id, now_ms, parse_iso};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use serde_json::{Map, Value, json};

use super::snapshot::entries_for_category;
use crate::copy_name::{CopyNameOptions, CopyNameStyle, MAX_COPY_NAME_ATTEMPTS, copy_name};
use crate::errors::{BusinessError, Result};
use crate::flow::import::{
    BindingInput, FieldManifestEntry, FieldTable, compute_custom_field_name_bindings,
    resolve_by_name_and_type, rewrite_bot_field_tokens_deep, rewrite_field_tokens_deep,
    sanitize_unresolved,
};

/// `INSTALL_LEASE_MS`.
pub const INSTALL_LEASE_MS: i64 = 15 * 60 * 1000;
const MAX_STORED_WARNINGS: usize = 200;

pub mod sync;

/// `TEMPLATE_INSTALL_ORDER`.
pub const INSTALL_ORDER: &[&str] = &[
    "couponTopics",
    "questionnaires",
    "pipelines",
    "products",
    "settings",
    "aiFunctions",
    "aiMcpServers",
    "aiFiles",
    "aiAgents",
    "calendars",
    "webchats",
    "sequences",
    "flows",
    "keywords",
    "entryPointLinks",
    "triggers",
    "fbCommentAutomations",
];

/// `FLOW_INSERT_KINDS` + `FLOW_DEFERRED_KINDS`: the reference kinds a flow
/// graph is remapped through.
const FLOW_INSERT_KINDS: &[&str] = &[
    "customField",
    "botField",
    "sequence",
    "aiAgent",
    "integration",
    "calendar",
    "questionnaire",
    "couponTopic",
    "inbox",
    "messengerPersona",
    "spreadsheet",
    "tag",
    "product",
    "webhook",
    "aiFunction",
    "aiFile",
    "aiMcpServer",
    "pipeline",
    "pipelineStage",
    "pipelineField",
    "placeCategory",
    "workspaceMember",
    "inboxTeam",
    "vendor",
    "productCategory",
    "decisionProfile",
    "userPersistentMenu",
    "leaderboard",
    "miniApp",
];
const FLOW_DEFERRED_KINDS: &[&str] = &["flow", "flowNode", "trigger"];

/// The folder type an entry with no folder lands under, with an install folder.
fn root_folder_type(category: &str) -> Option<&'static str> {
    Some(match category {
        "flows" => "flow",
        "keywords" => "automatedResponse",
        "triggers" => "trigger",
        "sequences" => "sequence",
        _ => return None,
    })
}

/// A row an install made or reused (`TrackedResource`).
#[derive(Debug, Clone)]
pub struct Tracked {
    pub category: String,
    pub resource_kind: String,
    pub resource_id: i64,
    pub source_id: String,
    pub was_existing: bool,
}

/// A reference to a flow that is written once the flows are in (`createFlowRefPatcher`).
struct FlowRef {
    category: &'static str,
    table: &'static str,
    id: i64,
    column: &'static str,
    path: String,
    source: String,
}

/// One column of a table (`PRAGMA table_info` + whether it is a JSON column).
#[derive(Debug, Clone)]
struct Column {
    name: String,
    decl: String,
    json: bool,
}

/// `TemplateInstallContext`.
pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub workspace_id: i64,
    pub installation_id: i64,
    pub app_url: String,
    pub id_maps: ReferenceIdMaps,
    pub inbox_bindings: BTreeMap<String, String>,
    pub tracked: Vec<Tracked>,
    pub warnings: Vec<Value>,
    pub resource_count: i64,
    field_name_bindings: HashMap<String, String>,
    flow_refs: Vec<FlowRef>,
    install_roots: BTreeMap<String, i64>,
    pub published_flows: Vec<Value>,
    columns: HashMap<String, Vec<Column>>,
}

pub(crate) fn db_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::from(error))
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

impl<'a> Ctx<'a> {
    pub fn new(
        conn: &'a Connection,
        workspace_id: i64,
        installation_id: i64,
        app_url: &str,
    ) -> Self {
        Ctx {
            conn,
            workspace_id,
            installation_id,
            app_url: app_url.to_string(),
            id_maps: ReferenceIdMaps::new(),
            inbox_bindings: BTreeMap::new(),
            tracked: Vec::new(),
            warnings: Vec::new(),
            resource_count: 0,
            field_name_bindings: HashMap::new(),
            flow_refs: Vec::new(),
            install_roots: BTreeMap::new(),
            published_flows: Vec::new(),
            columns: HashMap::new(),
        }
    }

    /// `ctx.warn` (at most [`MAX_STORED_WARNINGS`] are kept).
    pub fn warn(
        &mut self,
        category: &str,
        entity_kind: &str,
        path: impl Into<String>,
        value: impl Into<String>,
    ) {
        if self.warnings.len() < MAX_STORED_WARNINGS {
            self.warnings.push(json!({
                "category": category,
                "entityKind": entity_kind,
                "path": path.into(),
                "value": value.into(),
            }));
        }
    }

    /// `recordCopy`.
    pub fn record(
        &mut self,
        category: &str,
        kind: &str,
        source_id: &str,
        resource_id: i64,
        map_kind: Option<&str>,
    ) {
        if let Some(map_kind) = map_kind {
            self.map(map_kind, source_id, resource_id);
        }
        self.track(category, kind, source_id, resource_id, false);
    }

    fn track(
        &mut self,
        category: &str,
        kind: &str,
        source_id: &str,
        resource_id: i64,
        was_existing: bool,
    ) {
        self.resource_count += 1;
        self.tracked.push(Tracked {
            category: category.to_string(),
            resource_kind: kind.to_string(),
            resource_id,
            source_id: source_id.to_string(),
            was_existing,
        });
    }

    fn map(&mut self, kind: &str, source_id: &str, target: i64) {
        self.id_maps
            .entry(kind.to_string())
            .or_default()
            .insert(source_id.to_string(), target.to_string());
    }

    fn mapped(&self, kind: &str, source: &str) -> Option<i64> {
        self.id_maps.get(kind)?.get(source)?.parse().ok()
    }

    /// `resolveRef`: the target id of `source`, or a warning and `None`.
    pub fn resolve(
        &mut self,
        category: &str,
        kind: &str,
        path: String,
        source: &Value,
    ) -> Option<i64> {
        let source = text_of(source)?;
        let found = self.mapped(kind, &source);
        if found.is_none() {
            self.warn(category, kind, path, source);
        }
        found
    }

    /// `resolveFolderRef` (+ the install root for an entry with no folder).
    fn folder(&mut self, category: &str, entry: &Value) -> Option<i64> {
        let source = &entry["folderId"];
        if text_of(source).is_none_or(|s| s == "0") {
            return root_folder_type(category).and_then(|t| self.install_roots.get(t).copied());
        }
        let path = format!(
            "{category}.{}.folderId",
            entry["sourceId"].as_str().unwrap_or_default()
        );
        self.resolve(category, "folder", path, source)
    }

    /// Defers a flow reference to Phase 2.
    fn defer_flow(
        &mut self,
        category: &'static str,
        table: &'static str,
        id: i64,
        column: &'static str,
        path: String,
        source: &Value,
    ) {
        if let Some(source) = text_of(source) {
            self.flow_refs.push(FlowRef {
                category,
                table,
                id,
                column,
                path,
                source,
            });
        }
    }

    fn columns(&mut self, table: &str) -> Result<Vec<Column>> {
        if let Some(cols) = self.columns.get(table) {
            return Ok(cols.clone());
        }
        let sql: String = self
            .conn
            .query_row(
                "SELECT \"sql\" FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(db_err)?;
        let cols: Vec<Column> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(db_err)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?
            .into_iter()
            .map(|(name, decl)| Column {
                json: sql.contains(&format!("json_valid(\"{name}\")")),
                name,
                decl: decl.to_uppercase(),
            })
            .collect();
        self.columns.insert(table.to_string(), cols.clone());
        Ok(cols)
    }

    /// The fields of `fields` that are columns of `table`, as bind values.
    fn binds(&mut self, table: &str, fields: &Map<String, Value>) -> Result<Vec<(String, Sql)>> {
        let cols = self.columns(table)?;
        Ok(cols
            .iter()
            .filter_map(|col| {
                fields
                    .get(&col.name)
                    .map(|v| (col.name.clone(), to_sql(col, v)))
            })
            .collect())
    }

    /// Inserts one row of `table` (a new `id`, the workspace when the table has
    /// the column) from JSON fields; unknown keys are ignored.
    pub fn insert(&mut self, table: &str, mut fields: Map<String, Value>) -> Result<i64> {
        let id = new_id();
        fields.insert("id".into(), json!(id));
        if self.columns(table)?.iter().any(|c| c.name == "workspaceId") {
            fields
                .entry("workspaceId")
                .or_insert(json!(self.workspace_id));
        }
        let binds = self.binds(table, &fields)?;
        let names: Vec<String> = binds.iter().map(|(c, _)| format!("\"{c}\"")).collect();
        let marks = vec!["?"; binds.len()].join(", ");
        self.conn
            .execute(
                &format!(
                    "INSERT INTO \"{table}\" ({}) VALUES ({marks})",
                    names.join(", ")
                ),
                params_from_iter(binds.into_iter().map(|(_, v)| v)),
            )
            .map_err(db_err)?;
        Ok(id)
    }

    /// Updates columns of one row of `table` from JSON fields.
    pub fn update(&mut self, table: &str, id: i64, fields: Map<String, Value>) -> Result<()> {
        let binds = self.binds(table, &fields)?;
        if binds.is_empty() {
            return Ok(());
        }
        let set: Vec<String> = binds.iter().map(|(c, _)| format!("\"{c}\" = ?")).collect();
        let mut values: Vec<Sql> = binds.into_iter().map(|(_, v)| v).collect();
        values.push(Sql::Integer(id));
        self.conn
            .execute(
                &format!("UPDATE \"{table}\" SET {} WHERE \"id\" = ?", set.join(", ")),
                params_from_iter(values),
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// `insertCopyNamed`: `name`, or the first free `<name> - Copy [n]` of the workspace.
    fn free_name(&mut self, table: &str, column: &str, name: &str, extra: &str) -> Result<String> {
        let taken = |candidate: &str| -> Result<bool> {
            self.conn
                .query_row(
                    &format!(
                        "SELECT 1 FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND \"{column}\" = ?2{extra} LIMIT 1"
                    ),
                    params![self.workspace_id, candidate],
                    |_| Ok(()),
                )
                .optional()
                .map(|r| r.is_some())
                .map_err(db_err)
        };
        if !taken(name)? {
            return Ok(name.to_string());
        }
        let options = CopyNameOptions {
            style: CopyNameStyle::Dash,
            max_length: 255,
        };
        for attempt in 1..=MAX_COPY_NAME_ATTEMPTS {
            let candidate = copy_name(name, attempt, options);
            if !taken(&candidate)? {
                return Ok(candidate);
            }
        }
        Ok(format!("{name} - {}", new_id()))
    }

    fn find_id(
        &self,
        sql: &str,
        values: &[&dyn chatbotx_db::rusqlite::ToSql],
    ) -> Result<Option<i64>> {
        self.conn
            .query_row(sql, values, |r| r.get(0))
            .optional()
            .map_err(db_err)
    }
}

/// A JSON value as the bind value of `col` (STRICT column types).
fn to_sql(col: &Column, value: &Value) -> Sql {
    if value.is_null() {
        return Sql::Null;
    }
    if col.json {
        return Sql::Text(value.to_string());
    }
    match col.decl.as_str() {
        "INTEGER" => match value {
            Value::Bool(b) => Sql::Integer(i64::from(*b)),
            Value::Number(n) => n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .map_or(Sql::Null, Sql::Integer),
            Value::String(s) => s
                .parse::<i64>()
                .ok()
                .or_else(|| parse_iso(s))
                .map_or(Sql::Null, Sql::Integer),
            _ => Sql::Null,
        },
        "REAL" => match value {
            Value::Number(n) => n.as_f64().map_or(Sql::Null, Sql::Real),
            Value::String(s) => s.parse::<f64>().map_or(Sql::Null, Sql::Real),
            _ => Sql::Null,
        },
        _ => match value {
            Value::String(s) => Sql::Text(s.clone()),
            other => Sql::Text(other.to_string()),
        },
    }
}

/// The listed keys of `entry`.
fn pick(entry: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|k| entry.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect()
}

fn source_id(entry: &Value) -> String {
    text_of(&entry["sourceId"]).unwrap_or_default()
}

/// `rewriteCouponTokensDeep`: `{{coupon:<id>}}` follows the installed topic.
fn rewrite_coupon_tokens_deep(value: &Value, ids: &BTreeMap<String, String>) -> Value {
    if ids.is_empty() {
        return value.clone();
    }
    map_string_leaves(value, &|text| {
        variable_placeholder_re()
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let inner = chatbotx_core::js::trim(&caps[1]);
                match inner
                    .strip_prefix("coupon:")
                    .and_then(|id| ids.get(chatbotx_core::js::trim(id)))
                {
                    Some(target) => format!("{{{{coupon:{target}}}}}"),
                    None => caps[0].to_owned(),
                }
            })
            .into_owned()
    })
}

/// What [`run`] wrote.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: &'static str,
    pub warnings: Vec<Value>,
    pub resource_count: i64,
    pub install_folder_id: Option<i64>,
    pub published_flows: Vec<Value>,
}

/// What an install applies.
pub struct RunInput<'p> {
    pub payload: &'p Value,
    pub template_name: String,
    pub create_install_folder: bool,
    /// `{ allowEdit, allowDelete }` of the installation.
    pub permissions: Value,
    pub inbox_bindings: BTreeMap<String, String>,
    pub source_version: i64,
}

/// `runInstall` + `flushTracked`: Phase R, Phase 1 and Phase 2 on `ctx`.
pub fn run(ctx: &mut Ctx<'_>, input: &RunInput<'_>) -> Result<Outcome> {
    ctx.inbox_bindings = input.inbox_bindings.clone();
    let payload = input.payload;
    let manifests = &payload["manifests"];
    let install_folder_id = resolve_manifests(ctx, manifests, input)?;

    let resources = rewrite_field_tokens_deep(&payload["resources"], &ctx.field_name_bindings);
    let entries_of = |ctx: &Ctx<'_>, category: &str| -> Vec<Value> {
        prepared_entries(ctx, &resources, category)
    };

    // Coupon topics first, then the early bot fields, then every category.
    let coupons = entries_of(ctx, "couponTopics");
    insert_category(ctx, "couponTopics", &coupons)?;
    let settings = entries_of(ctx, "settings");
    let (bot_fields, replies): (Vec<Value>, Vec<Value>) =
        settings.into_iter().partition(|e| e["kind"] == "botField");
    insert_bot_fields(ctx, &bot_fields)?;
    for category in INSTALL_ORDER.iter().filter(|c| **c != "couponTopics") {
        if *category == "settings" {
            insert_saved_replies(ctx, &replies)?;
            continue;
        }
        let entries = entries_of(ctx, category);
        if !entries.is_empty() {
            insert_category(ctx, category, &entries)?;
        }
    }
    apply_deferred(ctx)?;
    flush_tracked(ctx, &input.permissions, payload)?;
    stamp_synced_hashes(
        ctx.conn,
        ctx.workspace_id,
        ctx.installation_id,
        input.source_version,
    )?;
    Ok(Outcome {
        status: if ctx.warnings.is_empty() {
            "completed"
        } else {
            "partial"
        },
        warnings: ctx.warnings.clone(),
        resource_count: ctx.resource_count,
        install_folder_id,
        published_flows: ctx.published_flows.clone(),
    })
}

/// The entries of `category` with the `{{coupon:..}}` / `{{bot_field:..}}`
/// tokens rewritten to the copies (read at use time: they are inserted first).
fn prepared_entries(ctx: &Ctx<'_>, resources: &Value, category: &str) -> Vec<Value> {
    let entries = Value::Array(entries_for_category(resources, category));
    let empty = BTreeMap::new();
    let coupons = ctx.id_maps.get("couponTopic").unwrap_or(&empty);
    let bots = ctx.id_maps.get("botField").unwrap_or(&empty);
    let out = rewrite_bot_field_tokens_deep(&rewrite_coupon_tokens_deep(&entries, coupons), bots);
    out.as_array().cloned().unwrap_or_default()
}

/// `lockFor` + `flushTracked`: the provenance rows with their copied lock.
fn flush_tracked(ctx: &mut Ctx<'_>, permissions: &Value, payload: &Value) -> Result<()> {
    let allow_edit = permissions["allowEdit"] != false;
    let allow_delete = permissions["allowDelete"] != false;
    let base = now_ms();
    for (i, t) in std::mem::take(&mut ctx.tracked).into_iter().enumerate() {
        let item_locked = payload["permissions"][&t.category][&t.source_id]["allowEdit"] == false;
        let (edit, delete) = if t.was_existing {
            (true, true)
        } else {
            (allow_edit && !item_locked, allow_delete)
        };
        // `spreadCreatedAt`: distinct, ordered creation times.
        ctx.conn
            .execute(
                r#"INSERT INTO "TemplateInstalledResource" ("id", "createdAt", "updatedAt", "installationId",
                       "workspaceId", "category", "resourceKind", "resourceId", "sourceResourceId",
                       "wasExisting", "allowEdit", "allowDelete")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                   ON CONFLICT DO NOTHING"#,
                params![
                    new_id(),
                    base + i as i64,
                    ctx.installation_id,
                    ctx.workspace_id,
                    t.category,
                    t.resource_kind,
                    t.resource_id,
                    t.source_id,
                    t.was_existing,
                    edit,
                    delete
                ],
            )
            .map_err(db_err)?;
    }
    Ok(())
}

/// `isManifestKind`: rows a sync never compares.
pub const MANIFEST_KINDS: &[&str] = &[
    "folder",
    "tag",
    "customField",
    "productCategory",
    "placeCategory",
    "pipeline",
];

/// `handler.project` + `hashCanonical`: the hash of an installed copy's content
/// as the collector reads it (`None` = the row is gone).
pub fn project_hash(
    conn: &Connection,
    workspace_id: i64,
    category: &str,
    resource_id: i64,
) -> Result<Option<String>> {
    Ok(
        project(conn, workspace_id, category, resource_id)?.map(|entry| {
            chatbotx_core::utils::crypto::sha256_hex(&super::snapshot::canonical_text(&entry))
        }),
    )
}

/// `handler.project`: the copy as an entry (no `sourceId`), fingerprinted.
pub fn project(
    conn: &Connection,
    workspace_id: i64,
    category: &str,
    resource_id: i64,
) -> Result<Option<Value>> {
    let id = resource_id.to_string();
    let collected =
        super::collect::collect(conn, workspace_id, category, std::slice::from_ref(&id))?;
    let Some(mut entry) = collected
        .entries
        .into_iter()
        .find(|e| text_of(&e["sourceId"]).as_deref() == Some(id.as_str()))
    else {
        return Ok(None);
    };
    if let Some(object) = entry.as_object_mut() {
        object.remove("sourceId");
    }
    Ok(Some(super::snapshot::fingerprint_entry(category, &entry)))
}

/// `stampSyncedHashes`: the baseline a later sync compares a copy against.
pub fn stamp_synced_hashes(
    conn: &Connection,
    workspace_id: i64,
    installation_id: i64,
    version: i64,
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            r#"SELECT "id", "category", "resourceKind", "resourceId" FROM "TemplateInstalledResource"
               WHERE "installationId" = ?1 AND "wasExisting" = 0 AND "detachedAt" IS NULL AND "syncedVersion" IS NULL"#,
        )
        .map_err(db_err)?;
    let rows: Vec<(i64, String, String, i64)> = stmt
        .query_map([installation_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(db_err)?
        .collect::<chatbotx_db::rusqlite::Result<_>>()
        .map_err(db_err)?;
    for (id, category, kind, resource_id) in rows {
        if MANIFEST_KINDS.contains(&kind.as_str()) || category == "aiFiles" {
            continue;
        }
        let hash = project_hash(conn, workspace_id, &category, resource_id)?;
        conn
            .execute(
                r#"UPDATE "TemplateInstalledResource" SET "syncedHash" = ?1, "syncedVersion" = ?2 WHERE "id" = ?3"#,
                params![hash, version, id],
            )
            .map_err(db_err)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase R: manifests
// ---------------------------------------------------------------------------

fn entries(manifest: &Value) -> Vec<(String, Value)> {
    manifest
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// `resolveManifests`: the install folder id, if one was made.
fn resolve_manifests(
    ctx: &mut Ctx<'_>,
    manifests: &Value,
    input: &RunInput<'_>,
) -> Result<Option<i64>> {
    let mut install_folder_id = None;
    if input.create_install_folder {
        for folder_type in ["flow", "automatedResponse", "trigger", "sequence"] {
            let id = ctx.insert(
                "Folder",
                pick(
                    &json!({ "name": input.template_name, "folderType": folder_type, "paths": [] }),
                    &["name", "folderType", "paths"],
                ),
            )?;
            ctx.track(
                "flows",
                "folder",
                &format!("__installRoot:{folder_type}"),
                id,
                false,
            );
            ctx.map("folder", &format!("__installRoot:{folder_type}"), id);
            ctx.install_roots.insert(folder_type.to_string(), id);
            if folder_type == "flow" {
                install_folder_id = Some(id);
            }
        }
    }
    resolve_folders(ctx, &manifests["folders"])?;
    resolve_custom_fields(ctx, &manifests["customFields"])?;
    resolve_tags(ctx, &manifests["tags"])?;
    resolve_product_categories(ctx, &manifests["productCategories"])?;
    for (source, entry) in entries(&manifests["pipelines"]) {
        install_pipeline(ctx, &source, &entry, true)?;
    }
    for (source, entry) in entries(&manifests["placeCategories"]) {
        let name = entry["name"].as_str().unwrap_or_default().to_string();
        let found = ctx.find_id(
            r#"SELECT "id" FROM "PlaceCategory" WHERE "workspaceId" = ?1 AND "name" = ?2 LIMIT 1"#,
            &[&ctx.workspace_id, &name],
        )?;
        let (id, existing) = match found {
            Some(id) => (id, true),
            None => (ctx.insert("PlaceCategory", pick(&entry, &["name"]))?, false),
        };
        ctx.map("placeCategory", &source, id);
        ctx.track("flows", "placeCategory", &source, id, existing);
    }
    resolve_inboxes(ctx, &manifests["inboxes"])?;
    Ok(install_folder_id)
}

/// `resolveFolderManifest`: every folder created, parents first.
fn resolve_folders(ctx: &mut Ctx<'_>, manifest: &Value) -> Result<()> {
    let mut pending = entries(manifest);
    let mut paths: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
    while !pending.is_empty() {
        let before = pending.len();
        let mut rest = Vec::new();
        for (source, entry) in pending {
            let folder_type = entry["folderType"].as_str().unwrap_or("flow").to_string();
            let parent = match text_of(&entry["parentSourceId"]) {
                None => Some(ctx.install_roots.get(&folder_type).copied()),
                Some(parent) if manifest.get(&parent).is_none() => Some(None),
                Some(parent) => ctx.mapped("folder", &parent).map(Some),
            };
            let Some(parent) = parent else {
                rest.push((source, entry));
                continue;
            };
            let mut path = parent
                .map(|p| paths.get(&p).cloned().unwrap_or_default())
                .unwrap_or_default();
            if let Some(p) = parent {
                path.push(json!(p.to_string()));
            }
            // `reuse` by folder type + parent + name.
            let name = entry["name"].as_str().unwrap_or_default().to_string();
            let found = ctx.find_id(
                r#"SELECT "id" FROM "Folder" WHERE "workspaceId" = ?1 AND "folderType" = ?2 AND "parentId" IS ?3 AND "name" = ?4 LIMIT 1"#,
                &[&ctx.workspace_id, &folder_type, &parent, &name],
            )?;
            let (id, existing) = match found {
                Some(id) => {
                    let stored: String = ctx
                        .conn
                        .query_row(
                            r#"SELECT "paths" FROM "Folder" WHERE "id" = ?1"#,
                            [id],
                            |r| r.get(0),
                        )
                        .map_err(db_err)?;
                    path = serde_json::from_str(&stored).unwrap_or_default();
                    (id, true)
                }
                None => {
                    let fields = pick(
                        &json!({ "name": name, "folderType": folder_type, "parentId": parent, "paths": path }),
                        &["name", "folderType", "parentId", "paths"],
                    );
                    (ctx.insert("Folder", fields)?, false)
                }
            };
            paths.insert(id, path);
            ctx.map("folder", &source, id);
            ctx.track(
                folder_category(&folder_type),
                "folder",
                &source,
                id,
                existing,
            );
        }
        if rest.len() == before {
            for (source, _) in &rest {
                ctx.warn(
                    "flows",
                    "folder",
                    format!("manifests.folders.{source}"),
                    source.clone(),
                );
            }
            break;
        }
        pending = rest;
    }
    Ok(())
}

/// `folderTypeToCategory`.
fn folder_category(folder_type: &str) -> &'static str {
    match folder_type {
        "fbComment" | "igComment" | "igStory" => "fbCommentAutomations",
        "automatedResponse" | "outboundAutomatedResponse" => "keywords",
        "sequence" => "sequences",
        "trigger" => "triggers",
        "customField" => "customFields",
        _ => "flows",
    }
}

fn field_manifest(manifest: &Value) -> Vec<(String, FieldManifestEntry, Value)> {
    entries(manifest)
        .into_iter()
        .filter_map(|(source, e)| {
            let kind = CustomFieldType::parse(e["type"].as_str()?)?;
            Some((
                source,
                FieldManifestEntry {
                    name: e["name"].as_str()?.to_string(),
                    kind,
                },
                e,
            ))
        })
        .collect()
}

/// `resolveCustomFieldManifest`: same-typed fields are reused, the rest created.
fn resolve_custom_fields(ctx: &mut Ctx<'_>, manifest: &Value) -> Result<()> {
    let fields = field_manifest(manifest);
    if fields.is_empty() {
        return Ok(());
    }
    let wanted: Vec<FieldManifestEntry> = fields.iter().map(|(_, f, _)| f.clone()).collect();
    let resolution =
        resolve_by_name_and_type(ctx.conn, ctx.workspace_id, FieldTable::CustomField, &wanted)?;
    let (bindings, _) = compute_custom_field_name_bindings(
        &fields
            .iter()
            .map(|(source, f, _)| BindingInput {
                source_id: source.clone(),
                name: f.name.clone(),
                kind: f.kind.as_str().to_owned(),
                target: resolution
                    .resolved
                    .get(&custom_field_resolution_key(&f.name, f.kind))
                    .cloned(),
            })
            .collect::<Vec<_>>(),
    );
    ctx.field_name_bindings = bindings;
    for (source, f, _) in &fields {
        let Some(target) = resolution
            .resolved
            .get(&custom_field_resolution_key(&f.name, f.kind))
        else {
            ctx.warn(
                "customFields",
                "customField",
                format!("manifests.customFields.{source}"),
                source.clone(),
            );
            continue;
        };
        let Ok(id) = target.id.parse::<i64>() else {
            continue;
        };
        let created = resolution.created_ids.contains(&target.id);
        ctx.map("customField", source, id);
        ctx.track("customFields", "customField", source, id, !created);
    }
    Ok(())
}

/// `resolveTagManifest`: tags by trimmed name, created when missing.
fn resolve_tags(ctx: &mut Ctx<'_>, manifest: &Value) -> Result<()> {
    for (source, entry) in entries(manifest) {
        let name = entry["name"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            ctx.warn(
                "tags",
                "tag",
                format!("manifests.tags.{source}"),
                source.clone(),
            );
            continue;
        }
        let found = ctx.find_id(
            r#"SELECT "id" FROM "Tag" WHERE "workspaceId" = ?1 AND "name" = ?2 AND "deletedAt" IS NULL LIMIT 1"#,
            &[&ctx.workspace_id, &name],
        )?;
        let (id, existing) = match found {
            Some(id) => (id, true),
            None => (
                ctx.insert("Tag", pick(&json!({ "name": name }), &["name"]))?,
                false,
            ),
        };
        ctx.map("tag", &source, id);
        ctx.track("tags", "tag", &source, id, existing);
    }
    Ok(())
}

/// `resolveProductCategoryManifest`: by name under the same parent, parents first.
fn resolve_product_categories(ctx: &mut Ctx<'_>, manifest: &Value) -> Result<()> {
    let mut pending = entries(manifest);
    while !pending.is_empty() {
        let before = pending.len();
        let mut rest = Vec::new();
        for (source, entry) in pending {
            let parent = match text_of(&entry["parentSourceId"]) {
                None => Some(None),
                Some(p) if manifest.get(&p).is_none() => Some(None),
                Some(p) => ctx.mapped("productCategory", &p).map(Some),
            };
            let Some(parent) = parent else {
                rest.push((source, entry));
                continue;
            };
            let name = entry["name"].as_str().unwrap_or_default().to_string();
            let found = ctx.find_id(
                r#"SELECT "id" FROM "ProductCategory" WHERE "workspaceId" = ?1 AND "name" = ?2 AND "parentId" IS ?3 LIMIT 1"#,
                &[&ctx.workspace_id, &name, &parent],
            )?;
            let (id, existing) = match found {
                Some(id) => (id, true),
                None => {
                    let mut fields = pick(&entry, &["name", "rank"]);
                    fields.insert("parentId".into(), json!(parent));
                    (ctx.insert("ProductCategory", fields)?, false)
                }
            };
            ctx.map("productCategory", &source, id);
            ctx.track(
                "productCategories",
                "productCategory",
                &source,
                id,
                existing,
            );
        }
        if rest.len() == before {
            break;
        }
        pending = rest;
    }
    Ok(())
}

/// `resolveInboxManifest`: the installer's binding, else the same inbox id
/// when this workspace has it connected on the same channel.
fn resolve_inboxes(ctx: &mut Ctx<'_>, manifest: &Value) -> Result<()> {
    let entries = entries(manifest);
    if entries.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = entries
        .iter()
        .filter_map(|(source, _)| {
            ctx.inbox_bindings
                .get(source)
                .unwrap_or(source)
                .parse()
                .ok()
        })
        .collect();
    let found = chatbotx_db::repo::inbox::list_identities_by_ids(ctx.conn, ctx.workspace_id, &ids)?;
    for (source, entry) in entries {
        let bound = ctx.inbox_bindings.get(&source).cloned();
        let wanted = bound.clone().unwrap_or_else(|| source.clone());
        let target = found.iter().find(|row| {
            row.id.to_string() == wanted
                && row.status == "connected"
                && entry["channelType"] == row.channel
        });
        match target {
            Some(row) => {
                let id = row.id;
                ctx.map("inbox", &source, id);
            }
            None if bound.is_some() => return Err(super::request::invalid_inbox_binding(&source)),
            None => {}
        }
    }
    Ok(())
}

/// A pipeline with its stages and fields: a manifest pipeline reuses one of
/// the same `referenceName`; a `pipelines` entry is always a new copy.
fn install_pipeline(
    ctx: &mut Ctx<'_>,
    source: &str,
    entry: &Value,
    reuse: bool,
) -> Result<Option<i64>> {
    let reference = entry["referenceName"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let existing = if reuse {
        ctx.find_id(
            r#"SELECT "id" FROM "Pipeline" WHERE "workspaceId" = ?1 AND "referenceName" = ?2 LIMIT 1"#,
            &[&ctx.workspace_id, &reference],
        )?
    } else {
        None
    };
    let category = if reuse { "flows" } else { "pipelines" };
    let pipeline_id = match existing {
        Some(id) => {
            ctx.map("pipeline", source, id);
            ctx.track(category, "pipeline", source, id, true);
            id
        }
        None => {
            let name = ctx.free_name(
                "Pipeline",
                "name",
                entry["name"].as_str().unwrap_or("Pipeline"),
                "",
            )?;
            let reference = ctx.free_name("Pipeline", "referenceName", &reference, "")?;
            let mut fields = pick(entry, &["type", "currency", "settings"]);
            fields.insert("name".into(), json!(name));
            fields.insert("referenceName".into(), json!(reference));
            let id = ctx.insert("Pipeline", fields)?;
            ctx.map("pipeline", source, id);
            ctx.track(category, "pipeline", source, id, false);
            id
        }
    };
    let existing_children =
        |ctx: &Ctx<'_>, table: &str, key: &str, value: &str| -> Result<Option<i64>> {
            ctx.find_id(
                &format!(
                    r#"SELECT "id" FROM "{table}" WHERE "pipelineId" = ?1 AND "{key}" = ?2 LIMIT 1"#
                ),
                &[&pipeline_id, &value],
            )
        };
    for (position, stage) in entry["stages"].as_array().into_iter().flatten().enumerate() {
        let name = stage["name"].as_str().unwrap_or_default();
        let id = match existing.and(existing_children(ctx, "PipelineStage", "name", name)?) {
            Some(id) => id,
            None => {
                let mut fields = pick(
                    stage,
                    &["name", "description", "textColor", "backgroundColor"],
                );
                fields.insert("pipelineId".into(), json!(pipeline_id));
                fields.insert("position".into(), json!(position));
                ctx.insert("PipelineStage", fields)?
            }
        };
        ctx.map("pipelineStage", &source_id(stage), id);
    }
    for (position, field) in entry["fields"].as_array().into_iter().flatten().enumerate() {
        let reference = field["referenceName"].as_str().unwrap_or_default();
        let id = match existing.and(existing_children(
            ctx,
            "PipelineField",
            "referenceName",
            reference,
        )?) {
            Some(id) => id,
            None => {
                let mut fields = pick(field, &["name", "referenceName", "type", "required"]);
                fields.insert("pipelineId".into(), json!(pipeline_id));
                fields.insert("position".into(), json!(position));
                ctx.insert("PipelineField", fields)?
            }
        };
        ctx.map("pipelineField", &source_id(field), id);
    }
    Ok(Some(pipeline_id))
}

// ---------------------------------------------------------------------------
// Phase 1: adapters
// ---------------------------------------------------------------------------

/// The early entries of settings: bot fields reused by name and type, or created.
fn insert_bot_fields(ctx: &mut Ctx<'_>, entries: &[Value]) -> Result<()> {
    let fields: Vec<(String, FieldManifestEntry, Value)> = entries
        .iter()
        .filter_map(|e| {
            Some((
                source_id(e),
                FieldManifestEntry {
                    name: e["name"].as_str()?.to_string(),
                    kind: CustomFieldType::parse(e["type"].as_str()?)?,
                },
                e.clone(),
            ))
        })
        .collect();
    if fields.is_empty() {
        return Ok(());
    }
    let wanted: Vec<FieldManifestEntry> = fields.iter().map(|(_, f, _)| f.clone()).collect();
    let resolution =
        resolve_by_name_and_type(ctx.conn, ctx.workspace_id, FieldTable::BotField, &wanted)?;
    for (source, f, entry) in &fields {
        let Some(target) = resolution
            .resolved
            .get(&custom_field_resolution_key(&f.name, f.kind))
        else {
            continue;
        };
        let Ok(id) = target.id.parse::<i64>() else {
            continue;
        };
        let created = resolution.created_ids.contains(&target.id);
        if created {
            let mut values = pick(entry, &["value", "description"]);
            let folder = ctx.folder("settings", entry);
            values.insert("folderId".into(), json!(folder));
            ctx.update("BotField", id, values)?;
        }
        ctx.map("botField", source, id);
        ctx.track("settings", "botField", source, id, !created);
    }
    Ok(())
}

/// Saved replies: one with the same shortcut is reused; images are not copied.
fn insert_saved_replies(ctx: &mut Ctx<'_>, entries: &[Value]) -> Result<()> {
    for entry in entries {
        let source = source_id(entry);
        let shortcut = entry["shortcut"].as_str().unwrap_or_default().to_string();
        let found = ctx.find_id(
            r#"SELECT "id" FROM "SavedReply" WHERE "workspaceId" = ?1 AND "shortcut" = ?2 LIMIT 1"#,
            &[&ctx.workspace_id, &shortcut],
        )?;
        if let Some(id) = found {
            ctx.track("settings", "savedReply", &source, id, true);
            continue;
        }
        let id = ctx.insert("SavedReply", pick(entry, &["shortcut", "text"]))?;
        if entry["images"].as_array().is_some_and(|i| !i.is_empty()) {
            ctx.warn(
                "settings",
                "savedReplyImage",
                format!("settings.{source}.images"),
                source.clone(),
            );
        }
        ctx.record("settings", "savedReply", &source, id, Some("savedReply"));
    }
    Ok(())
}

fn insert_category(ctx: &mut Ctx<'_>, category: &'static str, entries: &[Value]) -> Result<()> {
    for entry in entries {
        let source = source_id(entry);
        match category {
            "couponTopics" => {
                let name = ctx.free_name(
                    "CouponTopic",
                    "name",
                    entry["name"].as_str().unwrap_or_default(),
                    r#" AND "deletedAt" IS NULL"#,
                )?;
                let mut fields = pick(entry, &["description", "expiresAt", "status"]);
                fields.insert("name".into(), json!(name));
                let id = ctx.insert("CouponTopic", fields)?;
                if entry["expiresAt"]
                    .as_str()
                    .and_then(parse_iso)
                    .is_some_and(|at| at <= now_ms())
                {
                    let at = entry["expiresAt"].as_str().unwrap_or_default().to_string();
                    ctx.warn(
                        "couponTopics",
                        "couponTopicExpired",
                        format!("couponTopics.{source}.expiresAt"),
                        at,
                    );
                }
                ctx.record(category, "couponTopic", &source, id, Some("couponTopic"));
            }
            "questionnaires" => {
                let mut fields = pick(
                    entry,
                    &[
                        "name",
                        "enableScore",
                        "enableRetryMessages",
                        "enableCustomFieldMapping",
                    ],
                );
                fields.insert("triggerFlowId".into(), Value::Null);
                let id = ctx.insert("Questionnaire", fields)?;
                ctx.defer_flow(
                    category,
                    "Questionnaire",
                    id,
                    "triggerFlowId",
                    format!("questionnaires.{source}.triggerFlowId"),
                    &entry["triggerFlowId"],
                );
                for (i, q) in entry["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let mut fields = pick(
                        q,
                        &[
                            "title",
                            "type",
                            "active",
                            "orderNo",
                            "point",
                            "retryMessage",
                            "systemFieldKey",
                            "config",
                        ],
                    );
                    let field = ctx.resolve(
                        category,
                        "customField",
                        format!("questionnaires.{source}.questions.{i}.customFieldId"),
                        &q["customFieldId"],
                    );
                    fields.insert("customFieldId".into(), json!(field));
                    fields.insert("questionnaireId".into(), json!(id));
                    if text_of(&q["image"]).is_some() {
                        ctx.warn(
                            category,
                            "questionImage",
                            format!("questionnaires.{source}.questions.{i}.image"),
                            source.clone(),
                        );
                    }
                    ctx.insert("QuestionnaireQuestion", fields)?;
                }
                ctx.record(
                    category,
                    "questionnaire",
                    &source,
                    id,
                    Some("questionnaire"),
                );
            }
            "pipelines" => {
                install_pipeline(ctx, &source, entry, false)?;
            }
            "products" => insert_product(ctx, entry, &source)?,
            "aiFunctions" => {
                let mut fields = pick(entry, &["name", "purpose", "dataCollect", "outputMessage"]);
                fields.insert("triggerFlowId".into(), Value::Null);
                let id = ctx.insert("AIFunction", fields)?;
                ctx.defer_flow(
                    category,
                    "AIFunction",
                    id,
                    "triggerFlowId",
                    format!("aiFunctions.{source}.triggerFlowId"),
                    &entry["triggerFlowId"],
                );
                ctx.record(category, "aiFunction", &source, id, Some("aiFunction"));
            }
            "aiMcpServers" => {
                let name = ctx.free_name(
                    "AIMCPServer",
                    "name",
                    entry["name"].as_str().unwrap_or_default(),
                    "",
                )?;
                let mut fields = pick(entry, &["url", "availableTools", "selectedTools"]);
                fields.insert("name".into(), json!(name));
                // Never the source credentials: the installer adds their own.
                fields.insert("auth".into(), json!({ "type": "none" }));
                let id = ctx.insert("AIMCPServer", fields)?;
                if entry["authType"] != "none"
                    || entry["urlHadSecret"] == true
                    || text_of(&entry["url"]).is_none()
                {
                    let names: Vec<String> = entry["headerNames"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|h| h.as_str().map(str::to_string))
                        .collect();
                    ctx.warn(
                        category,
                        "credentialRequired",
                        format!("aiMcpServers.{source}.auth"),
                        names.join(", "),
                    );
                }
                ctx.record(category, "aiMcpServer", &source, id, Some("aiMcpServer"));
            }
            "aiFiles" => {
                // ponytail: no object staging yet; add with a storage copy + re-embed.
                ctx.warn(
                    category,
                    "aiFileUnavailable",
                    format!("aiFiles.{source}"),
                    source.clone(),
                );
            }
            "aiAgents" => {
                let mut fields = agent_fields(ctx, entry, &source);
                fields.insert("isDefault".into(), json!(false));
                fields.insert("actionRules".into(), json!([]));
                let id = ctx.insert("AIAgent", fields)?;
                ctx.record(category, "aiAgent", &source, id, Some("aiAgent"));
            }
            "calendars" => {
                let mut fields = pick(
                    entry,
                    &[
                        "name",
                        "description",
                        "timezone",
                        "locationType",
                        "locationDetail",
                        "durationMinutes",
                        "bufferAfterMinutes",
                        "scheduleWindowType",
                        "scheduleWindowConfig",
                        "maxAppointmentsPerUser",
                        "dailyLimitEnabled",
                        "maxPerDay",
                        "allowGroupMeeting",
                        "maxPerSlot",
                        "confirmationMessage",
                    ],
                );
                fields.insert(
                    "publicLinkSlug".into(),
                    json!(chatbotx_core::utils::id::create_id()),
                );
                let id = ctx.insert("AppointmentCalendar", fields)?;
                for column in ["confirmationFlowId", "cancellationFlowId"] {
                    ctx.defer_flow(
                        category,
                        "AppointmentCalendar",
                        id,
                        column,
                        format!("calendars.{source}.{column}"),
                        &entry[column],
                    );
                }
                for slot in entry["availability"].as_array().into_iter().flatten() {
                    let mut fields = pick(slot, &["weekday", "startMinute", "endMinute"]);
                    fields.insert("calendarId".into(), json!(id));
                    ctx.insert("AppointmentCalendarAvailability", fields)?;
                }
                // A reminder needs its flow: written once flows are in.
                for (i, reminder) in entry["reminders"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let mut fields = pick(reminder, &["timingValue", "timingUnit"]);
                    fields.insert("calendarId".into(), json!(id));
                    fields.insert("flowId".into(), reminder["flowId"].clone());
                    ctx.flow_refs.push(FlowRef {
                        category: "calendars",
                        table: "AppointmentCalendarReminder",
                        id: -1,
                        column: "flowId",
                        path: format!("calendars.{source}.reminders.{i}.flowId"),
                        source: serde_json::to_string(&Value::Object(fields)).unwrap_or_default(),
                    });
                }
                ctx.record(category, "calendar", &source, id, Some("calendar"));
            }
            "webchats" => {
                let data = crate::integration_webchat::WebchatData {
                    name: entry["name"].as_str().unwrap_or("Webchat").to_string(),
                    welcome_flow_id: None,
                    authorized_domains: strings(&entry["authorizedDomains"]),
                    conversation_starters: entry["conversationStarters"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                    persistent_menus: entry["persistentMenus"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                    brand_color: entry["brandColor"]
                        .as_str()
                        .unwrap_or("#000000")
                        .to_string(),
                    hide_header: entry["hideHeader"] == true,
                    show_logo: entry["showLogo"] != false,
                    hide_message_input: entry["hideMessageInput"] == true,
                    custom_css: entry["customCss"].as_str().map(str::to_string),
                    enable: entry["enable"] != false,
                };
                let app_url = ctx.app_url.clone();
                let (webchat, _) = crate::integration_webchat::create_in(
                    ctx.conn,
                    ctx.workspace_id,
                    None,
                    data,
                    &app_url,
                    now_ms(),
                )?;
                ctx.defer_flow(
                    category,
                    "IntegrationWebchat",
                    webchat.id,
                    "welcomeFlowId",
                    format!("webchats.{source}.welcomeFlowId"),
                    &entry["welcomeFlowId"],
                );
                ctx.record(category, "integrationWebchat", &source, webchat.id, None);
            }
            "sequences" => {
                let name = ctx.free_name(
                    "Sequence",
                    "name",
                    entry["name"].as_str().unwrap_or_default(),
                    "",
                )?;
                let steps = entry["steps"].as_array().cloned().unwrap_or_default();
                let folder = ctx.folder(category, entry);
                let fields = pick(
                    &json!({ "name": name, "active": entry["active"], "folderId": folder, "subscribers": 0, "messages": steps.len() }),
                    &["name", "active", "folderId", "subscribers", "messages"],
                );
                let id = ctx.insert("Sequence", fields)?;
                ctx.record(category, "sequence", &source, id, Some("sequence"));
                for step in &steps {
                    let mut fields = pick(
                        step,
                        &[
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
                        ],
                    );
                    fields.insert("sequenceId".into(), json!(id));
                    let step_id = ctx.insert("SequenceStep", fields)?;
                    let order = step["order"].to_string();
                    ctx.defer_flow(
                        category,
                        "SequenceStep",
                        step_id,
                        "flowId",
                        format!("sequences.{source}.steps.{order}.flowId"),
                        &step["flowId"],
                    );
                }
            }
            "flows" => insert_flow(ctx, entry, &source)?,
            "keywords" => {
                let mut fields = keyword_fields(ctx, entry, &source);
                let folder = ctx.folder(category, entry);
                fields.insert("type".into(), entry["type"].clone());
                fields.insert("status".into(), json!(true));
                fields.insert("folderId".into(), json!(folder));
                let id = ctx.insert("AutomatedResponse", fields)?;
                ctx.record(category, "automatedResponse", &source, id, None);
            }
            "entryPointLinks" => {
                let Some(flow) = ctx.resolve(
                    category,
                    "flow",
                    format!("entryPointLinks.{source}.flowId"),
                    &entry["flowId"],
                ) else {
                    continue;
                };
                let field = ctx.resolve(
                    category,
                    "customField",
                    format!("entryPointLinks.{source}.customFieldId"),
                    &entry["customFieldId"],
                );
                let name = ctx.free_name(
                    "Reflink",
                    "name",
                    entry["name"].as_str().unwrap_or_default(),
                    "",
                )?;
                let mut fields = pick(entry, &["type", "qrStyles"]);
                fields.insert("name".into(), json!(name));
                fields.insert("flowId".into(), json!(flow));
                fields.insert("customFieldId".into(), json!(field));
                let id = ctx.insert("Reflink", fields)?;
                ctx.record(category, "reflink", &source, id, None);
            }
            "triggers" => insert_trigger(ctx, entry, &source)?,
            "fbCommentAutomations" => {
                let reply = |ctx: &mut Ctx<'_>, key: &str| -> Value {
                    let mut value = entry[key].clone();
                    let kind = match value["type"].as_str() {
                        Some("flow") => "flow",
                        Some("AIAgent") => "aiAgent",
                        _ => return value,
                    };
                    let target = ctx.resolve(
                        "fbCommentAutomations",
                        kind,
                        format!("fbCommentAutomations.{source}.{key}.value"),
                        &value["value"],
                    );
                    value["value"] = target.map_or(Value::Null, |t| json!(t.to_string()));
                    value
                };
                let private_reply = reply(ctx, "privateReply");
                let public_reply = reply(ctx, "publicReply");
                let folder = ctx.folder(category, entry);
                let inbox = match text_of(&entry["inboxSourceId"]) {
                    None => Some(None),
                    Some(s) => ctx.mapped("inbox", &s).map(Some),
                };
                if inbox.is_none() {
                    let s = text_of(&entry["inboxSourceId"]).unwrap_or_default();
                    ctx.warn(
                        category,
                        "inboxBindingRequired",
                        format!("fbCommentAutomations.{source}.inboxId"),
                        s,
                    );
                }
                // `clearPostSelection`: chosen posts mean nothing in another account.
                let mut post = entry["post"].clone();
                let post_cleared = post["type"] == "postIds";
                if post_cleared {
                    post = json!({ "type": "all" });
                    ctx.warn(
                        category,
                        "postSelectionCleared",
                        format!("fbCommentAutomations.{source}.post"),
                        source.clone(),
                    );
                }
                let mut fields = pick(
                    entry,
                    &[
                        "name",
                        "type",
                        "startTime",
                        "endTime",
                        "includeKeywords",
                        "excludeKeywords",
                        "options",
                        "hideComments",
                        "replyAfter",
                    ],
                );
                fields.insert(
                    "isActive".into(),
                    json!(entry["isActive"] == true && inbox.is_some() && !post_cleared),
                );
                fields.insert("post".into(), post);
                fields.insert("privateReply".into(), private_reply);
                fields.insert("publicReply".into(), public_reply);
                fields.insert("folderId".into(), json!(folder));
                fields.insert("inboxId".into(), json!(inbox.flatten()));
                let id = ctx.insert("CommentAutomation", fields)?;
                ctx.record(category, "fbCommentAutomation", &source, id, None);
            }
            _ => {}
        }
    }
    Ok(())
}

/// The template-owned columns of a keyword (`text`, the match, `flowId`).
fn keyword_fields(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> Map<String, Value> {
    let flow = ctx.resolve(
        "keywords",
        "flow",
        format!("keywords.{source}.flowId"),
        &entry["flowId"],
    );
    let mut fields = pick(entry, &["text"]);
    match entry["conditions"].as_array().filter(|c| !c.is_empty()) {
        Some(c) => {
            fields.insert("conditions".into(), json!(c));
            fields.insert("keywords".into(), entry["keywords"].clone());
        }
        None => {
            fields.insert(
                "keywords".into(),
                if entry["keywords"].is_array() {
                    entry["keywords"].clone()
                } else {
                    json!([])
                },
            );
        }
    }
    fields.insert("flowId".into(), json!(flow));
    fields
}

/// The template-owned columns of an AI agent (tools and models resolved; an
/// `openaiCompatible` model is dropped with an `integration` warning).
fn agent_fields(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> Map<String, Value> {
    let mut unresolved: Vec<UnresolvedReference> = Vec::new();
    let mut collect = |r: &UnresolvedReference| unresolved.push(r.clone());
    let tools = remap_references(
        &json!({ "tools": entry["tools"] }),
        &ctx.id_maps,
        RemapOptions {
            kinds: Some(vec![
                "aiFunction".into(),
                "aiFile".into(),
                "aiMcpServer".into(),
            ]),
            on_unresolved: Some(&mut collect),
        },
    )["tools"]
        .clone();
    for r in unresolved {
        ctx.warn(
            "aiAgents",
            &r.entity_kind,
            format!("aiAgents.{source}.tools"),
            r.value,
        );
    }
    let mut models = Vec::new();
    for model in entry["models"].as_array().into_iter().flatten() {
        if model["kind"] == "openaiCompatible" {
            let integration = text_of(&model["integrationId"]).unwrap_or_default();
            ctx.warn(
                "aiAgents",
                "integration",
                format!("aiAgents.{source}.models.integrationId"),
                integration,
            );
        } else {
            models.push(model.clone());
        }
    }
    let mut fields = pick(
        entry,
        &[
            "name",
            "prompt",
            "messages",
            "temperature",
            "maxOutputTokens",
            "isRichResponse",
        ],
    );
    fields.insert("tools".into(), tools);
    fields.insert("models".into(), json!(models));
    fields.insert(
        "webSearchAuthorizedDomains".into(),
        if entry["webSearchAuthorizedDomains"].is_array() {
            entry["webSearchAuthorizedDomains"].clone()
        } else {
            json!([])
        },
    );
    fields
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn insert_product(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> Result<()> {
    let mut fields = pick(
        entry,
        &[
            "name",
            "shortDescription",
            "longDescription",
            "price",
            "taxes",
            "discount",
            "currency",
            "productUrl",
            "sku",
            "inventoryPolicy",
            "inventoryQuantity",
            "allowOutOfStockPurchase",
            "images",
            "tags",
            "rank",
            "status",
            "isSearchable",
            "allowSpecialRequest",
            "isAddonOnly",
        ],
    );
    for column in ["categoryId", "subcategoryId"] {
        let target = ctx.resolve(
            "products",
            "productCategory",
            format!("products.{source}.{column}"),
            &entry[column],
        );
        fields.insert(column.into(), json!(target));
    }
    if let Some(vendor) = entry["vendor"].as_str().filter(|v| !v.is_empty()) {
        let found = ctx.find_id(
            r#"SELECT "id" FROM "Vendor" WHERE "workspaceId" = ?1 AND "name" = ?2 LIMIT 1"#,
            &[&ctx.workspace_id, &vendor],
        )?;
        let vendor_id = match found {
            Some(id) => id,
            None => ctx.insert("Vendor", pick(&json!({ "name": vendor }), &["name"]))?,
        };
        fields.insert("vendorId".into(), json!(vendor_id));
    }
    let id = ctx.insert("Product", fields)?;
    for option in entry["variantOptions"].as_array().into_iter().flatten() {
        let mut fields = pick(option, &["name", "values", "position"]);
        fields.insert("productId".into(), json!(id));
        ctx.insert("ProductVariantOption", fields)?;
    }
    for variant in entry["variants"].as_array().into_iter().flatten() {
        let mut fields = pick(variant, &["combination", "price", "isEnabled"]);
        fields.insert("productId".into(), json!(id));
        ctx.insert("ProductVariant", fields)?;
    }
    for addon in entry["addons"].as_array().into_iter().flatten() {
        let mut fields = pick(addon, &["name", "maxSelections"]);
        fields.insert("productId".into(), json!(id));
        fields.insert("addonProductIds".into(), json!([]));
        let addon_id = ctx.insert("ProductAddon", fields)?;
        // Add-on products resolve once every product is in.
        ctx.flow_refs.push(FlowRef {
            category: "products",
            table: "ProductAddon",
            id: addon_id,
            column: "addonProductIds",
            path: format!("products.{source}.addons"),
            source: addon["addonProductIds"].to_string(),
        });
    }
    ctx.record("products", "product", source, id, Some("product"));
    Ok(())
}

/// `insertFlows`: the graph remapped through the maps so far; references to
/// flows, nodes and triggers are remapped in Phase 2.
/// The flow's graph remapped over `kinds` and sanitized; `false` when a
/// reference did not resolve (each one is warned).
fn flow_graph(
    ctx: &mut Ctx<'_>,
    entry: &Value,
    source: &str,
    kinds: &[&str],
) -> Result<(Vec<Value>, Vec<Value>, bool)> {
    let graph = json!({ "nodes": entry["nodes"], "edges": entry["edges"] });
    let kinds: Vec<String> = kinds.iter().map(|k| (*k).to_string()).collect();
    let mut unresolved: Vec<UnresolvedReference> = Vec::new();
    let mut collect = |r: &UnresolvedReference| unresolved.push(r.clone());
    let remapped = remap_flow_graph_references(
        &graph,
        &ctx.id_maps,
        RemapOptions {
            kinds: Some(kinds),
            on_unresolved: Some(&mut collect),
        },
    );
    let sanitized = sanitize_unresolved(ctx.conn, ctx.workspace_id, &remapped, &unresolved)?;
    for r in &unresolved {
        ctx.warn(
            "flows",
            &r.entity_kind,
            format!("flows.{source}.{}", r.path),
            r.value.clone(),
        );
    }
    let nodes = sanitized["nodes"].as_array().cloned().unwrap_or_default();
    let edges = sanitized["edges"].as_array().cloned().unwrap_or_default();
    Ok((nodes, edges, unresolved.is_empty()))
}

fn insert_flow(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> Result<()> {
    let (nodes, edges, all_resolved) = flow_graph(ctx, entry, source, FLOW_INSERT_KINDS)?;
    let folder = ctx.folder("flows", entry);
    let start: i64 = text_of(&entry["startNodeId"])
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    let publishable = entry["published"] == true
        && all_resolved
        && crate::flow::assert_flow_graph_publishable(&json!(nodes), &json!(edges)).is_ok();
    if !publishable && entry["published"] == true {
        ctx.warn(
            "flows",
            "flowNotPublished",
            format!("flows.{source}"),
            source.to_string(),
        );
    }
    let ids = crate::flow::create_published_default(
        ctx.conn,
        crate::flow::PublishedDefault {
            workspace_id: ctx.workspace_id,
            name: entry["name"].as_str().unwrap_or("Flow").to_string(),
            folder_id: folder,
            start_node_id: start,
            nodes,
            edges,
        },
    )?;
    let active = publishable && entry["active"] == true;
    ctx.conn
        .execute(
            r#"UPDATE "Flow" SET "active" = ?1, "enableInInbox" = ?2, "currentVersionId" = CASE WHEN ?3 THEN "currentVersionId" END WHERE "id" = ?4"#,
            params![active, entry["enableInInbox"] == true, publishable, ids.flow_id],
        )
        .map_err(db_err)?;
    if !publishable {
        ctx.conn
            .execute(
                r#"DELETE FROM "FlowVersion" WHERE "id" = ?1"#,
                [ids.published_version_id],
            )
            .map_err(db_err)?;
    } else {
        ctx.published_flows
            .push(json!({ "flowId": ids.flow_id.to_string(), "versionId": ids.published_version_id.to_string() }));
    }
    for node in entry["nodes"].as_array().into_iter().flatten() {
        if let Some(node_id) = text_of(&node["id"]) {
            ctx.id_maps
                .entry("flowNode".into())
                .or_default()
                .insert(node_id.clone(), node_id);
        }
    }
    ctx.record("flows", "flow", source, ids.flow_id, Some("flow"));
    Ok(())
}

/// Triggers: actions remapped (flows are in), conditions rewritten by kind.
fn insert_trigger(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> Result<()> {
    let (actions, all_resolved) = trigger_actions(ctx, entry, source);
    let folder = ctx.folder("triggers", entry);
    let name = ctx.free_name(
        "Trigger",
        "name",
        entry["name"].as_str().unwrap_or_default(),
        "",
    )?;
    let mut fields = Map::new();
    fields.insert("name".into(), json!(name));
    fields.insert(
        "active".into(),
        json!(entry["active"] == true && all_resolved),
    );
    fields.insert("actions".into(), actions);
    fields.insert("folderId".into(), json!(folder));
    let id = ctx.insert("Trigger", fields)?;
    if write_conditions(ctx, entry, source, id)? {
        ctx.conn
            .execute(r#"UPDATE "Trigger" SET "active" = 0 WHERE "id" = ?1"#, [id])
            .map_err(db_err)?;
    }
    ctx.record("triggers", "trigger", source, id, Some("trigger"));
    Ok(())
}

/// The trigger's actions remapped; `false` when a reference did not resolve.
fn trigger_actions(ctx: &mut Ctx<'_>, entry: &Value, source: &str) -> (Value, bool) {
    let mut unresolved: Vec<UnresolvedReference> = Vec::new();
    let mut collect = |r: &UnresolvedReference| unresolved.push(r.clone());
    let actions = remap_references(
        &entry["actions"],
        &ctx.id_maps,
        RemapOptions {
            kinds: None,
            on_unresolved: Some(&mut collect),
        },
    );
    for r in &unresolved {
        ctx.warn(
            "triggers",
            &r.entity_kind,
            format!("triggers.{source}.actions.{}", r.path),
            r.value.clone(),
        );
    }
    let actions = if actions.is_null() {
        json!([])
    } else {
        actions
    };
    (actions, unresolved.is_empty())
}

/// Inserts the trigger's `Condition` rows; `true` when a row was lost (its
/// source did not resolve), so the trigger must not stay active.
fn write_conditions(ctx: &mut Ctx<'_>, entry: &Value, source: &str, id: i64) -> Result<bool> {
    let mut lost = false;
    for (i, condition) in entry["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let mut fields = pick(condition, &["type", "operator", "value"]);
        if condition["type"] == "pipelineItemMovedToStage"
            && text_of(&condition["value"]["stageId"]).is_some()
        {
            let stage = ctx.resolve(
                "triggers",
                "pipelineStage",
                format!("triggers.{source}.conditions.{i}.value.stageId"),
                &condition["value"]["stageId"],
            );
            let mut value = condition["value"].clone();
            value["stageId"] = stage.map_or(Value::Null, |s| json!(s.to_string()));
            fields.insert("value".into(), value);
        }
        let kind = condition["type"]
            .as_str()
            .and_then(super::snapshot::condition_source_kind);
        let target = match kind {
            Some(kind) => {
                let t = ctx.resolve(
                    "triggers",
                    kind,
                    format!("triggers.{source}.conditions.{i}.sourceId"),
                    &condition["sourceId"],
                );
                if t.is_none() {
                    lost = true;
                    continue;
                }
                t.map(|t| json!(t.to_string()))
            }
            None => Some(condition["sourceId"].clone()),
        };
        fields.insert("sourceId".into(), target.unwrap_or(Value::Null));
        fields.insert("triggerId".into(), json!(id));
        ctx.insert("Condition", fields)?;
    }
    Ok(lost)
}

// ---------------------------------------------------------------------------
// Phase 2: deferred references
// ---------------------------------------------------------------------------

fn apply_deferred(ctx: &mut Ctx<'_>) -> Result<()> {
    for r in std::mem::take(&mut ctx.flow_refs) {
        match (r.table, r.column) {
            ("ProductAddon", _) => {
                let sources: Vec<Value> = serde_json::from_str(&r.source).unwrap_or_default();
                let mut ids = Vec::new();
                for s in &sources {
                    if let Some(id) = ctx.resolve(r.category, "product", r.path.clone(), s) {
                        ids.push(json!(id.to_string()));
                    }
                }
                ctx.update(
                    r.table,
                    r.id,
                    pick(&json!({ "addonProductIds": ids }), &["addonProductIds"]),
                )?;
            }
            ("AppointmentCalendarReminder", _) => {
                let mut fields: Map<String, Value> =
                    serde_json::from_str(&r.source).unwrap_or_default();
                let flow = fields.get("flowId").cloned().unwrap_or(Value::Null);
                if let Some(flow) = ctx.resolve(r.category, "flow", r.path.clone(), &flow) {
                    fields.insert("flowId".into(), json!(flow));
                    ctx.insert(r.table, fields)?;
                }
            }
            (table, column) => {
                if let Some(flow) =
                    ctx.resolve(r.category, "flow", r.path.clone(), &json!(r.source))
                {
                    let mut fields = Map::new();
                    fields.insert(column.into(), json!(flow));
                    ctx.update(table, r.id, fields)?;
                }
            }
        }
    }
    // `createDeferredFlowPatch`: flow, node and trigger references in the graphs.
    let flows: Vec<(String, i64)> = ctx
        .tracked
        .iter()
        .filter(|t| t.resource_kind == "flow")
        .map(|t| (t.source_id.clone(), t.resource_id))
        .collect();
    let kinds: Vec<String> = FLOW_DEFERRED_KINDS
        .iter()
        .map(|k| (*k).to_string())
        .collect();
    for (source, flow_id) in flows {
        let mut stmt = ctx
            .conn
            .prepare(r#"SELECT "id", "nodes", "edges" FROM "FlowVersion" WHERE "flowId" = ?1"#)
            .map_err(db_err)?;
        let versions: Vec<(i64, String, String)> = stmt
            .query_map([flow_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_err)?
            .collect::<chatbotx_db::rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut warned = false;
        for (version_id, nodes, edges) in versions {
            let graph = json!({
                "nodes": serde_json::from_str::<Value>(&nodes).unwrap_or(json!([])),
                "edges": serde_json::from_str::<Value>(&edges).unwrap_or(json!([])),
            });
            let mut unresolved: Vec<UnresolvedReference> = Vec::new();
            let mut collect = |r: &UnresolvedReference| unresolved.push(r.clone());
            let remapped = remap_flow_graph_references(
                &graph,
                &ctx.id_maps,
                RemapOptions {
                    kinds: Some(kinds.clone()),
                    on_unresolved: Some(&mut collect),
                },
            );
            let paths: Vec<&str> = unresolved.iter().map(|r| r.path.as_str()).collect();
            let cleaned = chatbotx_core::flow_config::import_export::remove_unresolved_references(
                &remapped, &paths,
            );
            if !warned {
                for r in &unresolved {
                    ctx.warn(
                        "flows",
                        &r.entity_kind,
                        format!("flows.{source}.{}", r.path),
                        r.value.clone(),
                    );
                }
                warned = true;
            }
            ctx.conn
                .execute(
                    r#"UPDATE "FlowVersion" SET "nodes" = ?1, "edges" = ?2 WHERE "id" = ?3"#,
                    params![
                        cleaned["nodes"].to_string(),
                        cleaned["edges"].to_string(),
                        version_id
                    ],
                )
                .map_err(db_err)?;
        }
    }
    Ok(())
}
