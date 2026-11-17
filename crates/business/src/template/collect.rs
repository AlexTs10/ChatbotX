//! The save-time half of every resource adapter (`adapters/*` `collector`):
//! `resolveIds` (`mode: "all"`), `verifyOwnership` and `collect` for the 17
//! resource categories, plus the manifest-only `tags` / `customFields` id
//! checks. Rows are read through the generated models' JSON (ids as strings,
//! dates as ISO, flags as booleans), so an entry has the builder's shape.
//!
//! Ids are strings, as in the payload. A non-numeric id matches no row (the
//! original's bigint cast would fail the query instead).

use std::collections::{BTreeMap, BTreeSet};

use chatbotx_core::flow_config::import_export::collect_references_by_kind;
use chatbotx_core::utils::id::is_numeric_id;
use chatbotx_db::repo::models as m;
use chatbotx_db::repo::{Model, RowJson, find_many, id_list};
use chatbotx_db::rusqlite::{Connection, ToSql};
use serde_json::{Map, Value, json};

use crate::errors::Result;

/// The resource categories in registry order (`Object.keys(templateAdapterRegistry)`).
pub const RESOURCE_CATEGORIES: &[&str] = &[
    "products",
    "settings",
    "aiFunctions",
    "aiAgents",
    "calendars",
    "webchats",
    "flows",
    "keywords",
    "entryPointLinks",
    "triggers",
    "fbCommentAutomations",
    "sequences",
    "aiMcpServers",
    "aiFiles",
    "questionnaires",
    "couponTopics",
    "pipelines",
];

/// `templateResourceCategories` (enum order, used for the payload).
pub const RESOURCE_CATEGORY_ENUM: &[&str] = &[
    "flows",
    "products",
    "aiFunctions",
    "aiAgents",
    "calendars",
    "webchats",
    "keywords",
    "entryPointLinks",
    "triggers",
    "fbCommentAutomations",
    "settings",
    "sequences",
    "aiMcpServers",
    "aiFiles",
    "questionnaires",
    "couponTopics",
    "pipelines",
];

/// `templateCategories`: the resource categories, then the manifest-only ones.
pub const CATEGORIES: &[&str] = &[
    "flows",
    "products",
    "aiFunctions",
    "aiAgents",
    "calendars",
    "webchats",
    "keywords",
    "entryPointLinks",
    "triggers",
    "fbCommentAutomations",
    "settings",
    "sequences",
    "aiMcpServers",
    "aiFiles",
    "questionnaires",
    "couponTopics",
    "pipelines",
    "customFields",
    "tags",
    "productCategories",
];

/// `minFormatVersion: 2` categories (`categoriesRequiringV2`).
pub const V2_CATEGORIES: &[&str] = &[
    "sequences",
    "aiMcpServers",
    "aiFiles",
    "questionnaires",
    "couponTopics",
    "pipelines",
];

/// `TemplateCollectResult`.
#[derive(Debug, Clone, Default)]
pub struct Collected {
    pub entries: Vec<Value>,
    pub folder_ids: Vec<String>,
    pub product_category_ids: Vec<String>,
    pub inbox_ids: Vec<String>,
    /// `(category, sourceId)`.
    pub hard_dependencies: Vec<(&'static str, String)>,
}

/// The numeric ids among `ids`, deduplicated, as integers.
pub(crate) fn numeric_ids(ids: &[String]) -> Vec<i64> {
    let mut seen = BTreeSet::new();
    ids.iter()
        .filter(|id| is_numeric_id(id))
        .filter_map(|id| id.parse::<i64>().ok())
        .filter(|id| seen.insert(*id))
        .collect()
}

fn models<M: Model + RowJson>(
    conn: &Connection,
    tail: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<Value>> {
    Ok(find_many::<M>(conn, tail, params)?
        .iter()
        .map(RowJson::row_json)
        .collect())
}

/// Rows of `M` of the workspace among `ids` (`extra` = more `AND` filters).
fn rows<M: Model + RowJson>(
    conn: &Connection,
    workspace_id: i64,
    ids: &[String],
    extra: &str,
) -> Result<Vec<Value>> {
    let ids = numeric_ids(ids);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    models::<M>(
        conn,
        &format!(
            r#"WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)){extra} ORDER BY "id""#
        ),
        &[&workspace_id, &id_list(&ids)],
    )
}

/// Child rows of `M` whose `parent` column is one of `parent_ids`.
fn children<M: Model + RowJson>(
    conn: &Connection,
    parent: &str,
    parent_ids: &[i64],
    extra_order: &str,
) -> Result<Vec<Value>> {
    if parent_ids.is_empty() {
        return Ok(Vec::new());
    }
    models::<M>(
        conn,
        &format!(
            r#"WHERE "{parent}" IN (SELECT value FROM json_each(?1)) ORDER BY {extra_order}"id""#
        ),
        &[&id_list(parent_ids)],
    )
}

fn row_ids(rows: &[Value]) -> Vec<i64> {
    rows.iter()
        .filter_map(|row| row["id"].as_str()?.parse().ok())
        .collect()
}

/// `{ sourceId, ...keys }` of one row (`(out, column)` renames).
fn entry(row: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    out.insert("sourceId".into(), row["id"].clone());
    for key in keys {
        out.insert((*key).into(), row[*key].clone());
    }
    Value::Object(out)
}

fn pick(row: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        out.insert((*key).into(), row[*key].clone());
    }
    Value::Object(out)
}

fn of_parent<'a>(rows: &'a [Value], column: &'a str, parent: &'a Value) -> impl Iterator<Item = &'a Value> {
    rows.iter().filter(move |row| &row[column] == parent)
}

fn strings<'a>(values: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
        .collect()
}

/// `dependenciesOn`: deduped, empty ids dropped.
fn dependencies_on(category: &'static str, ids: Vec<String>) -> Vec<(&'static str, String)> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .map(|id| (category, id))
        .collect()
}

/// `DEPENDENCY_CATEGORY_BY_KIND`.
const DEPENDENCY_CATEGORY_BY_KIND: &[(&str, &str)] = &[
    ("flow", "flows"),
    ("aiAgent", "aiAgents"),
    ("calendar", "calendars"),
    ("sequence", "sequences"),
    ("questionnaire", "questionnaires"),
    ("couponTopic", "couponTopics"),
];

/// `collectResourceDependencies`: the shipped-category rows `value` names.
pub(crate) fn resource_dependencies(value: &Value) -> Vec<(&'static str, String)> {
    let kinds: Vec<&str> = DEPENDENCY_CATEGORY_BY_KIND.iter().map(|(k, _)| *k).collect();
    let by_kind = collect_references_by_kind(value, Some(&kinds));
    DEPENDENCY_CATEGORY_BY_KIND
        .iter()
        .flat_map(|(kind, category)| {
            by_kind
                .get(*kind)
                .into_iter()
                .flatten()
                .map(|id| (*category, id.clone()))
        })
        .collect()
}

/// The table and `AND` filter of a category's `findIds`.
fn id_source(category: &str) -> Option<(&'static str, &'static str)> {
    Some(match category {
        "products" => ("Product", ""),
        "aiFunctions" => ("AIFunction", ""),
        "aiAgents" => ("AIAgent", ""),
        "calendars" => ("AppointmentCalendar", r#" AND "deletedAt" IS NULL"#),
        "webchats" => ("IntegrationWebchat", ""),
        "flows" => ("Flow", ""),
        "keywords" => ("AutomatedResponse", r#" AND "type" = 'inbound'"#),
        "entryPointLinks" => ("Reflink", ""),
        "triggers" => ("Trigger", ""),
        "fbCommentAutomations" => ("CommentAutomation", ""),
        "sequences" => ("Sequence", ""),
        "aiMcpServers" => ("AIMCPServer", ""),
        "aiFiles" => ("AIFile", ""),
        "questionnaires" => ("Questionnaire", r#" AND "deletedAt" IS NULL"#),
        "couponTopics" => ("CouponTopic", r#" AND "deletedAt" IS NULL"#),
        "pipelines" => ("Pipeline", ""),
        "tags" => ("Tag", r#" AND "deletedAt" IS NULL"#),
        "customFields" => ("CustomField", ""),
        _ => return None,
    })
}

fn query_ids(
    conn: &Connection,
    table: &str,
    extra: &str,
    workspace_id: i64,
    ids: Option<&[i64]>,
    limit: Option<i64>,
    order: &str,
) -> Result<Vec<String>> {
    // `?2` = the id list, or NULL for every row.
    let limit_sql = limit.map(|l| format!(" LIMIT {l}")).unwrap_or_default();
    let sql = format!(
        r#"SELECT "id" FROM "{table}" WHERE "workspaceId" = ?1
             AND (?2 IS NULL OR "id" IN (SELECT value FROM json_each(?2))){extra}
           ORDER BY {order}{limit_sql}"#
    );
    let list: Option<String> = ids.map(id_list);
    let mut stmt = conn.prepare(&sql).map_err(super::db_error)?;
    stmt.query_map(chatbotx_db::rusqlite::params![workspace_id, list], |r| {
        r.get::<_, i64>(0).map(|id| id.to_string())
    })
    .map_err(super::db_error)?
    .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
    .map_err(super::db_error)
}

/// `findIds`: the workspace's ids of a category (`ids` narrows, `limit` bounds).
fn find_ids(
    conn: &Connection,
    workspace_id: i64,
    category: &str,
    ids: Option<&[i64]>,
    limit: Option<i64>,
) -> Result<Vec<String>> {
    if category == "settings" {
        let mut out = query_ids(conn, "SavedReply", "", workspace_id, ids, limit, r#""id""#)?;
        out.extend(query_ids(conn, "BotField", "", workspace_id, ids, limit, r#""id""#)?);
        return Ok(out);
    }
    let Some((table, extra)) = id_source(category) else {
        return Ok(Vec::new());
    };
    // Manifest-only categories list "all" oldest first (`findTags` / `findCustomFields`).
    let order = if matches!(category, "tags" | "customFields") {
        r#""createdAt", "id""#
    } else {
        r#""id""#
    };
    query_ids(conn, table, extra, workspace_id, ids, limit, order)
}

/// `resolveIds`: `mode: "all"`.
pub fn resolve_ids(conn: &Connection, workspace_id: i64, category: &str, limit: i64) -> Result<Vec<String>> {
    find_ids(conn, workspace_id, category, None, Some(limit))
}

/// `verifyOwnership`: the ids, among `ids`, the workspace owns.
pub fn verify_ownership(conn: &Connection, workspace_id: i64, category: &str, ids: &[String]) -> Result<Vec<String>> {
    let numeric = numeric_ids(ids);
    if numeric.is_empty() {
        return Ok(Vec::new());
    }
    find_ids(conn, workspace_id, category, Some(&numeric), None)
}

/// `collect` of one resource category.
pub fn collect(conn: &Connection, workspace_id: i64, category: &str, ids: &[String]) -> Result<Collected> {
    if ids.is_empty() {
        return Ok(Collected::default());
    }
    match category {
        "flows" => collect_flows(conn, workspace_id, ids),
        "products" => collect_products(conn, workspace_id, ids),
        "settings" => collect_settings(conn, workspace_id, ids),
        "aiFunctions" => {
            let rows = rows::<m::AiFunction>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                hard_dependencies: dependencies_on("flows", strings(rows.iter().map(|r| &r["triggerFlowId"]))),
                entries: rows
                    .iter()
                    .map(|r| entry(r, &["name", "purpose", "dataCollect", "outputMessage", "triggerFlowId"]))
                    .collect(),
                ..Default::default()
            })
        }
        "aiAgents" => {
            let rows = rows::<m::AiAgent>(conn, workspace_id, ids, "")?;
            let entries: Vec<Value> = rows
                .iter()
                .map(|r| {
                    entry(
                        r,
                        &[
                            "name",
                            "prompt",
                            "messages",
                            "models",
                            "temperature",
                            "maxOutputTokens",
                            "tools",
                            "isDefault",
                            "isRichResponse",
                            "webSearchAuthorizedDomains",
                        ],
                    )
                })
                .collect();
            let tools: Vec<String> = entries
                .iter()
                .flat_map(|e| strings(e["tools"].as_array().into_iter().flatten()))
                .collect();
            let mut hard_dependencies = Vec::new();
            for (prefix, category) in [("mcp:", "aiMcpServers"), ("file:", "aiFiles"), ("fn:", "aiFunctions")] {
                hard_dependencies.extend(dependencies_on(
                    category,
                    tools
                        .iter()
                        .filter_map(|t| t.strip_prefix(prefix))
                        .map(|rest| rest.split(':').next().unwrap_or_default().to_string())
                        .collect(),
                ));
            }
            Ok(Collected {
                entries,
                hard_dependencies,
                ..Default::default()
            })
        }
        "calendars" => collect_calendars(conn, workspace_id, ids),
        "webchats" => {
            let rows = rows::<m::IntegrationWebchat>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                hard_dependencies: dependencies_on("flows", strings(rows.iter().map(|r| &r["welcomeFlowId"]))),
                // `auth` is the webchat's own client-side embed credential, carried
                // over as the original does (never a third-party token).
                entries: rows
                    .iter()
                    .map(|r| {
                        entry(
                            r,
                            &[
                                "name",
                                "auth",
                                "enable",
                                "authorizedDomains",
                                "conversationStarters",
                                "persistentMenus",
                                "brandColor",
                                "hideHeader",
                                "showLogo",
                                "hideMessageInput",
                                "customCss",
                                "welcomeFlowId",
                            ],
                        )
                    })
                    .collect(),
                ..Default::default()
            })
        }
        "keywords" => {
            let rows = rows::<m::AutomatedResponse>(conn, workspace_id, ids, r#" AND "type" = 'inbound'"#)?;
            Ok(Collected {
                folder_ids: strings(rows.iter().map(|r| &r["folderId"])),
                hard_dependencies: dependencies_on("flows", strings(rows.iter().map(|r| &r["flowId"]))),
                entries: rows
                    .iter()
                    .map(|r| entry(r, &["type", "text", "keywords", "conditions", "flowId", "folderId"]))
                    .collect(),
                ..Default::default()
            })
        }
        "entryPointLinks" => {
            let rows = rows::<m::Reflink>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                hard_dependencies: dependencies_on("flows", strings(rows.iter().map(|r| &r["flowId"]))),
                entries: rows
                    .iter()
                    .map(|r| entry(r, &["name", "type", "flowId", "customFieldId", "qrStyles"]))
                    .collect(),
                ..Default::default()
            })
        }
        "triggers" => collect_triggers(conn, workspace_id, ids),
        "fbCommentAutomations" => {
            let rows = rows::<m::CommentAutomation>(conn, workspace_id, ids, "")?;
            let entries = rows
                .iter()
                .map(|r| {
                    let mut e = entry(
                        r,
                        &[
                            "name",
                            "type",
                            "isActive",
                            "startTime",
                            "endTime",
                            "post",
                            "privateReply",
                            "publicReply",
                            "includeKeywords",
                            "excludeKeywords",
                            "options",
                            "hideComments",
                            "replyAfter",
                            "folderId",
                        ],
                    );
                    e["inboxSourceId"] = r["inboxId"].clone();
                    e
                })
                .collect();
            let replies: Vec<&Value> = rows
                .iter()
                .flat_map(|r| [&r["privateReply"], &r["publicReply"]])
                .collect();
            let mut hard_dependencies = Vec::new();
            for (kind, category) in [("flow", "flows"), ("AIAgent", "aiAgents")] {
                hard_dependencies.extend(dependencies_on(
                    category,
                    strings(replies.iter().filter(|r| r["type"] == kind).map(|r| &r["value"])),
                ));
            }
            Ok(Collected {
                entries,
                folder_ids: strings(rows.iter().map(|r| &r["folderId"])),
                inbox_ids: strings(rows.iter().map(|r| &r["inboxId"])),
                hard_dependencies,
                ..Default::default()
            })
        }
        "sequences" => {
            let rows = rows::<m::Sequence>(conn, workspace_id, ids, "")?;
            let steps: Vec<Value> = children::<m::SequenceStep>(conn, "sequenceId", &row_ids(&rows), r#""order", "#)?
                .into_iter()
                // `isContentStep`: an off step that sends nothing is no content.
                .filter(|s| s["isActive"] == true || !s["flowId"].is_null())
                .collect();
            let step_keys = [
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
            Ok(Collected {
                entries: rows
                    .iter()
                    .map(|r| {
                        let mut e = entry(r, &["name", "active", "folderId"]);
                        e["steps"] = of_parent(&steps, "sequenceId", &r["id"])
                            .map(|s| pick(s, &step_keys))
                            .collect();
                        e
                    })
                    .collect(),
                folder_ids: strings(rows.iter().map(|r| &r["folderId"])),
                hard_dependencies: dependencies_on("flows", strings(steps.iter().map(|s| &s["flowId"]))),
                ..Default::default()
            })
        }
        "aiMcpServers" => {
            let rows = rows::<m::AiMcpServer>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                entries: rows
                    .iter()
                    .map(|r| {
                        let (url, had_secret) = strip_url_secrets(r["url"].as_str().unwrap_or_default());
                        let auth_type = r["auth"]["type"].clone();
                        let header_names: Vec<Value> = if auth_type == "header" {
                            r["auth"]["headers"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|h| h["header"].clone())
                                .collect()
                        } else {
                            Vec::new()
                        };
                        // Never the header values or URL credentials.
                        json!({
                            "sourceId": r["id"],
                            "name": r["name"],
                            "url": url,
                            "urlHadSecret": had_secret,
                            "authType": auth_type,
                            "headerNames": header_names,
                            "availableTools": r["availableTools"],
                            "selectedTools": r["selectedTools"],
                        })
                    })
                    .collect(),
                ..Default::default()
            })
        }
        "aiFiles" => {
            let rows = rows::<m::AiFile>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                entries: rows
                    .iter()
                    .map(|r| entry(r, &["name", "path", "mimeType", "size", "chunkingConfig"]))
                    .collect(),
                ..Default::default()
            })
        }
        "questionnaires" => {
            let rows = rows::<m::Questionnaire>(conn, workspace_id, ids, r#" AND "deletedAt" IS NULL"#)?;
            let questions: Vec<Value> = children::<m::QuestionnaireQuestion>(
                conn,
                "questionnaireId",
                &row_ids(&rows),
                r#""orderNo", "#,
            )?
            .into_iter()
            .filter(|q| q["deletedAt"].is_null())
            .collect();
            let question_keys = [
                "title",
                "type",
                "active",
                "image",
                "orderNo",
                "point",
                "retryMessage",
                "customFieldId",
                "systemFieldKey",
                "config",
            ];
            Ok(Collected {
                hard_dependencies: dependencies_on("flows", strings(rows.iter().map(|r| &r["triggerFlowId"]))),
                entries: rows
                    .iter()
                    .map(|r| {
                        let mut e = entry(
                            r,
                            &[
                                "name",
                                "enableScore",
                                "enableRetryMessages",
                                "enableCustomFieldMapping",
                                "triggerFlowId",
                            ],
                        );
                        e["questions"] = of_parent(&questions, "questionnaireId", &r["id"])
                            .map(|q| pick(q, &question_keys))
                            .collect();
                        e
                    })
                    .collect(),
                ..Default::default()
            })
        }
        "couponTopics" => {
            let rows = rows::<m::CouponTopic>(conn, workspace_id, ids, r#" AND "deletedAt" IS NULL"#)?;
            Ok(Collected {
                entries: rows
                    .iter()
                    .map(|r| entry(r, &["name", "description", "expiresAt", "status"]))
                    .collect(),
                ..Default::default()
            })
        }
        "pipelines" => {
            let rows = rows::<m::Pipeline>(conn, workspace_id, ids, "")?;
            Ok(Collected {
                entries: pipeline_entries(conn, workspace_id, &rows, true)?,
                ..Default::default()
            })
        }
        _ => Ok(Collected::default()),
    }
}

/// A pipeline with every stage and field (`manifests.pipelines` shape; with
/// `sourceId` for a `pipelines` entry).
pub(crate) fn pipeline_entries(
    conn: &Connection,
    workspace_id: i64,
    pipelines: &[Value],
    with_source_id: bool,
) -> Result<Vec<Value>> {
    let ids = row_ids(pipelines);
    let filter = r#" AND "workspaceId" = ?2 ORDER BY "position", "id""#;
    let stages = models::<m::PipelineStage>(
        conn,
        &format!(r#"WHERE "pipelineId" IN (SELECT value FROM json_each(?1)){filter}"#),
        &[&id_list(&ids), &workspace_id],
    )?;
    let fields = models::<m::PipelineField>(
        conn,
        &format!(r#"WHERE "pipelineId" IN (SELECT value FROM json_each(?1)){filter}"#),
        &[&id_list(&ids), &workspace_id],
    )?;
    Ok(pipelines
        .iter()
        .map(|p| {
            let mut e = pick(p, &["name", "referenceName", "type", "currency"]);
            if with_source_id {
                e["sourceId"] = p["id"].clone();
            }
            e["settings"] = if p["settings"].is_object() {
                p["settings"].clone()
            } else {
                json!({})
            };
            e["stages"] = of_parent(&stages, "pipelineId", &p["id"])
                .map(|s| entry(s, &["name", "description", "textColor", "backgroundColor"]))
                .collect();
            e["fields"] = of_parent(&fields, "pipelineId", &p["id"])
                .map(|f| entry(f, &["name", "referenceName", "type", "required"]))
                .collect();
            e
        })
        .collect())
}

fn collect_flows(conn: &Connection, workspace_id: i64, ids: &[String]) -> Result<Collected> {
    let flows = rows::<m::Flow>(conn, workspace_id, ids, "")?;
    // `findPreferredByFlowIds`: the latest published version, else the draft.
    let versions = models::<m::FlowVersion>(
        conn,
        r#"WHERE "workspaceId" = ?1 AND "flowId" IN (SELECT value FROM json_each(?2))
             AND ("isDraft" = 1 OR "isLatest" = 1)
           ORDER BY "flowId", "isDraft", "id" DESC"#,
        &[&workspace_id, &id_list(&row_ids(&flows))],
    )?;
    let mut preferred: BTreeMap<String, &Value> = BTreeMap::new();
    for version in &versions {
        if let Some(flow_id) = version["flowId"].as_str() {
            preferred.entry(flow_id.to_string()).or_insert(version);
        }
    }
    let entries: Vec<Value> = flows
        .iter()
        .filter_map(|flow| {
            let version = preferred.get(flow["id"].as_str()?)?;
            Some(json!({
                "sourceId": flow["id"],
                "published": version["isDraft"] != true,
                "name": flow["name"],
                "active": flow["active"],
                "enableInInbox": flow["enableInInbox"],
                "startNodeId": version["startNodeId"],
                "nodes": version["nodes"],
                "edges": version["edges"],
                "folderId": flow["folderId"],
            }))
        })
        .collect();
    let mut product_category_ids = BTreeSet::new();
    let mut hard_dependencies = Vec::new();
    for e in &entries {
        let found = collect_references_by_kind(e, Some(&["productCategory"]));
        product_category_ids.extend(
            found
                .get("productCategory")
                .into_iter()
                .flatten()
                .filter(|id| is_numeric_id(id))
                .cloned(),
        );
        hard_dependencies.extend(resource_dependencies(&json!({ "nodes": e["nodes"], "edges": e["edges"] })));
    }
    Ok(Collected {
        folder_ids: strings(flows.iter().map(|f| &f["folderId"])),
        product_category_ids: product_category_ids.into_iter().collect(),
        hard_dependencies,
        entries,
        ..Default::default()
    })
}

fn collect_products(conn: &Connection, workspace_id: i64, ids: &[String]) -> Result<Collected> {
    let products = rows::<m::Product>(conn, workspace_id, ids, "")?;
    let product_ids = row_ids(&products);
    let options = children::<m::ProductVariantOption>(conn, "productId", &product_ids, "")?;
    let variants = children::<m::ProductVariant>(conn, "productId", &product_ids, "")?;
    let addons = children::<m::ProductAddon>(conn, "productId", &product_ids, "")?;
    let vendor_ids: Vec<i64> = products
        .iter()
        .filter_map(|p| p["vendorId"].as_str()?.parse().ok())
        .collect();
    let vendors = if vendor_ids.is_empty() {
        Vec::new()
    } else {
        models::<m::Vendor>(
            conn,
            r#"WHERE "id" IN (SELECT value FROM json_each(?1))"#,
            &[&id_list(&vendor_ids)],
        )?
    };
    let entries = products
        .iter()
        .map(|p| {
            let mut e = entry(
                p,
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
                    "categoryId",
                    "subcategoryId",
                    "status",
                    "isSearchable",
                    "allowSpecialRequest",
                    "isAddonOnly",
                ],
            );
            e["vendor"] = vendors
                .iter()
                .find(|v| v["id"] == p["vendorId"])
                .map_or(Value::Null, |v| v["name"].clone());
            e["variantOptions"] = of_parent(&options, "productId", &p["id"])
                .map(|o| pick(o, &["name", "values", "position"]))
                .collect();
            e["variants"] = of_parent(&variants, "productId", &p["id"])
                .map(|v| pick(v, &["combination", "price", "isEnabled"]))
                .collect();
            e["addons"] = of_parent(&addons, "productId", &p["id"])
                .map(|a| pick(a, &["name", "maxSelections", "addonProductIds"]))
                .collect();
            e
        })
        .collect();
    let mut categories = BTreeSet::new();
    for p in &products {
        categories.extend(strings([&p["categoryId"], &p["subcategoryId"]]));
    }
    let selected: BTreeSet<&String> = ids.iter().collect();
    let addon_ids = addons
        .iter()
        .flat_map(|a| strings(a["addonProductIds"].as_array().into_iter().flatten()))
        .filter(|id| !selected.contains(id))
        .collect();
    Ok(Collected {
        entries,
        product_category_ids: categories.into_iter().collect(),
        hard_dependencies: dependencies_on("products", addon_ids),
        ..Default::default()
    })
}

fn collect_settings(conn: &Connection, workspace_id: i64, ids: &[String]) -> Result<Collected> {
    let replies = rows::<m::SavedReply>(conn, workspace_id, ids, "")?;
    let bot_fields = rows::<m::BotField>(conn, workspace_id, ids, "")?;
    let reply_ids = row_ids(&replies);
    let mut images: Vec<(String, Value)> = Vec::new();
    if !reply_ids.is_empty() {
        let mut stmt = conn
            .prepare(
                r#"SELECT i."savedReplyId", f."path", f."name", f."mimeType", f."size"
                   FROM "SavedReplyImage" i JOIN "MediaLibraryFile" f ON f."id" = i."mediaLibraryFileId"
                   WHERE i."workspaceId" = ?1 AND i."savedReplyId" IN (SELECT value FROM json_each(?2))
                   ORDER BY i."position", i."id""#,
            )
            .map_err(super::db_error)?;
        images = stmt
            .query_map(chatbotx_db::rusqlite::params![workspace_id, id_list(&reply_ids)], |r| {
                Ok((
                    r.get::<_, i64>(0)?.to_string(),
                    json!({
                        "path": r.get::<_, String>(1)?,
                        "name": r.get::<_, String>(2)?,
                        "mimeType": r.get::<_, String>(3)?,
                        "size": r.get::<_, i64>(4)?,
                    }),
                ))
            })
            .map_err(super::db_error)?
            .collect::<chatbotx_db::rusqlite::Result<_>>()
            .map_err(super::db_error)?;
    }
    let mut entries: Vec<Value> = replies
        .iter()
        .map(|r| {
            let mut e = entry(r, &["shortcut", "text"]);
            e["kind"] = json!("savedReply");
            e["images"] = images
                .iter()
                .filter(|(owner, _)| Some(owner.as_str()) == r["id"].as_str())
                .map(|(_, image)| image.clone())
                .collect();
            e
        })
        .collect();
    entries.extend(bot_fields.iter().map(|r| {
        let mut e = entry(r, &["name", "type", "value", "description", "folderId"]);
        e["kind"] = json!("botField");
        e
    }));
    Ok(Collected {
        entries,
        folder_ids: strings(bot_fields.iter().map(|r| &r["folderId"])),
        ..Default::default()
    })
}

fn collect_calendars(conn: &Connection, workspace_id: i64, ids: &[String]) -> Result<Collected> {
    let calendars = rows::<m::AppointmentCalendar>(conn, workspace_id, ids, "")?;
    let calendar_ids = row_ids(&calendars);
    let slots = children::<m::AppointmentCalendarAvailability>(conn, "calendarId", &calendar_ids, "")?;
    let reminders = children::<m::AppointmentCalendarReminder>(conn, "calendarId", &calendar_ids, "")?;
    let entries = calendars
        .iter()
        .map(|c| {
            let mut e = entry(
                c,
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
                    "confirmationFlowId",
                    "cancellationFlowId",
                ],
            );
            e["availability"] = of_parent(&slots, "calendarId", &c["id"])
                .map(|s| pick(s, &["weekday", "startMinute", "endMinute"]))
                .collect();
            e["reminders"] = of_parent(&reminders, "calendarId", &c["id"])
                .map(|r| pick(r, &["flowId", "timingValue", "timingUnit"]))
                .collect();
            e
        })
        .collect();
    let mut flows = strings(reminders.iter().map(|r| &r["flowId"]));
    flows.extend(strings(
        calendars
            .iter()
            .flat_map(|c| [&c["confirmationFlowId"], &c["cancellationFlowId"]]),
    ));
    Ok(Collected {
        entries,
        hard_dependencies: dependencies_on("flows", flows),
        ..Default::default()
    })
}

fn collect_triggers(conn: &Connection, workspace_id: i64, ids: &[String]) -> Result<Collected> {
    let triggers = rows::<m::Trigger>(conn, workspace_id, ids, "")?;
    let conditions = children::<m::Condition>(conn, "triggerId", &row_ids(&triggers), "")?;
    let actions = super::snapshot::with_trigger_field_keys_as_ids(
        conn,
        workspace_id,
        triggers.iter().map(|t| t["actions"].clone()).collect(),
    )?;
    let entries: Vec<Value> = triggers
        .iter()
        .zip(actions)
        .map(|(t, actions)| {
            let mut e = entry(t, &["name", "active", "folderId"]);
            e["conditions"] = of_parent(&conditions, "triggerId", &t["id"])
                .map(|c| pick(c, &["type", "sourceId", "operator", "value"]))
                .collect();
            e["actions"] = actions;
            e
        })
        .collect();
    let all_conditions: Vec<Value> = entries
        .iter()
        .flat_map(|e| e["conditions"].as_array().cloned().unwrap_or_default())
        .collect();
    let references = super::snapshot::trigger_condition_references(&all_conditions);
    let mut hard_dependencies: Vec<_> = entries
        .iter()
        .flat_map(|e| resource_dependencies(&e["actions"]))
        .collect();
    hard_dependencies.extend(dependencies_on(
        "sequences",
        references.get("sequence").cloned().unwrap_or_default().into_iter().collect(),
    ));
    Ok(Collected {
        folder_ids: strings(triggers.iter().map(|t| &t["folderId"])),
        product_category_ids: references
            .get("productCategory")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        hard_dependencies,
        entries,
        ..Default::default()
    })
}

/// `stripUrlSecrets`: the URL's origin only, and whether anything was dropped.
pub fn strip_url_secrets(raw: &str) -> (String, bool) {
    let Ok(parsed) = url::Url::parse(raw) else {
        return (String::new(), true);
    };
    let origin = parsed.origin();
    if !origin.is_tuple() {
        return (String::new(), true);
    }
    let had_secret = !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some_and(|q| !q.is_empty())
        || parsed.fragment().is_some_and(|f| !f.is_empty())
        || !matches!(parsed.path(), "/" | "");
    (format!("{}/", origin.ascii_serialization()), had_secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_url_secrets() {
        assert_eq!(strip_url_secrets("https://mcp.example.com/"), ("https://mcp.example.com/".into(), false));
        assert_eq!(
            strip_url_secrets("https://u:p@mcp.example.com/sse?key=1"),
            ("https://mcp.example.com/".into(), true)
        );
        assert_eq!(strip_url_secrets("not a url"), (String::new(), true));
    }

    #[test]
    fn numeric_ids_dedupe_and_drop_names() {
        assert_eq!(numeric_ids(&["2".into(), "x".into(), "2".into(), "10".into()]), vec![2, 10]);
    }
}
