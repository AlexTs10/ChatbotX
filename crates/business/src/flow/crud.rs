//! `flow/service.ts`: `listPublishedOptions`, `list`, `listNodes`,
//! `findById`, `update`, `duplicate`, `deleteMany`, `createPublishedDefault`
//! and `messengerWelcomeMessageFlowService.listCurrentByFlow` (P3.6-D2).
//! No audit records, no template locks / guards, no welcome-message sync
//! jobs: the functions return what the jobs would need.

use std::collections::{BTreeMap, HashSet};

use chatbotx_core::flow_config::import_export::{RemapOptions, remap_flow_graph_references};
use chatbotx_db::Db;
use chatbotx_db::repo::flow::{self as repo, FlowListInput, FlowOption, FlowWithVersions};
use chatbotx_db::repo::models::{Flow, FlowVersion};
use chatbotx_db::repo::{
    PaginationInput, SortItem, find_one, integration_whatsapp, messenger_welcome_message_flow,
    now_ms, parse_pagination, small_repos,
};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::filters::{filter_flows_by_start_step_type, filter_flows_by_template_ids};
use super::{insert_analytics_session, new_id, repo_err};
use crate::copy_name::{CopyNameOptions, CopyNameStyle, free_copy_names};
use crate::{BusinessError, Result, tx};

/// `FLOW_COPY_NAME`.
const FLOW_COPY_NAME: CopyNameOptions = CopyNameOptions {
    style: CopyNameStyle::Underscore,
    max_length: 255,
};

/// `flowService.listPublishedOptions`.
pub async fn list_published_options(db: &Db, workspace_id: i64) -> Result<Vec<FlowOption>> {
    tx::read(db, move |conn| {
        Ok(repo::list_published_options(conn, workspace_id)?)
    })
    .await
}

/// The input of [`list`] (`FlowListInput` + `startType`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFlows {
    pub workspace_id: i64,
    pub name: Option<String>,
    /// The API string (`"0"` = no folder).
    pub folder_id: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
    pub start_type: Option<String>,
    pub integration_whatsapp_id: Option<i64>,
}

/// One page of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlowListPage {
    pub data: Vec<FlowWithVersions>,
    pub page_count: i64,
}

/// `flowService.list`.
pub async fn list(db: &Db, input: ListFlows) -> Result<FlowListPage> {
    tx::read(db, move |conn| list_in(conn, input)).await
}

/// `FlowNodeSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowNodeSummary {
    pub flow_id: i64,
    pub node_id: String,
    pub label: String,
    pub node_type: Option<String>,
}

/// `flowService.listNodes`: the nodes of the published version.
pub async fn list_nodes(db: &Db, workspace_id: i64, flow_id: i64) -> Result<Vec<FlowNodeSummary>> {
    let nodes = tx::read(db, move |conn| {
        Ok(repo::find_current_version_nodes(
            conn,
            workspace_id,
            flow_id,
        )?)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Flow does not exists."))?;
    Ok(nodes
        .iter()
        .map(|node| FlowNodeSummary {
            flow_id,
            node_id: node_id_text(&node["id"]),
            label: read_node_label(node),
            node_type: node["type"].as_str().map(str::to_string),
        })
        .collect())
}

/// `flowService.findById`: the flow with all its versions.
pub async fn find_by_id(db: &Db, workspace_id: i64, id: i64) -> Result<FlowWithVersions> {
    tx::read(db, move |conn| {
        Ok(repo::find_with_versions(conn, workspace_id, id)?)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Flow does not exists."))
}

/// One row of `messengerWelcomeMessageFlowService.listCurrentByFlow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeMessageFlow {
    pub integration_messenger_id: i64,
    pub page_id: String,
    pub welcome_message_flow_id: String,
}

/// `messengerWelcomeMessageFlowService.listCurrentByFlow` (oldest first).
pub async fn list_welcome_message_flows(
    db: &Db,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<WelcomeMessageFlow>> {
    let rows = tx::read(db, move |conn| {
        Ok(messenger_welcome_message_flow::list_current_by_flow(
            conn,
            workspace_id,
            flow_id,
        )?)
    })
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| WelcomeMessageFlow {
            integration_messenger_id: row.integration_messenger_id,
            page_id: row.page_id,
            welcome_message_flow_id: row.meta_flow_id,
        })
        .collect())
}

/// The data of [`update`] (`None` = not given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowUpdate {
    pub name: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
}

/// The reason of a welcome-message flow sync (`enqueueSync`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeSync {
    Rename,
    Publish,
}

/// What [`update`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowUpdated {
    /// The keys whose value changed (`changedKeys`), in input order; empty = no write.
    pub changed_keys: Vec<&'static str>,
    /// The sync the original queues after the commit.
    pub sync: Option<WelcomeSync>,
}

/// `flowService.update`: a partial update of name / active / enableInInbox.
pub async fn update(db: &Db, workspace_id: i64, id: i64, data: FlowUpdate) -> Result<FlowUpdated> {
    tx::write(db, move |tx| {
        let current = find_flow(tx, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("Flow not found"))?;
        let mut changed_keys = Vec::new();
        if data.name.as_ref().is_some_and(|n| *n != current.name) {
            changed_keys.push("name");
        }
        if data.active.is_some_and(|a| a != current.active) {
            changed_keys.push("active");
        }
        if data
            .enable_in_inbox
            .is_some_and(|e| e != current.enable_in_inbox)
        {
            changed_keys.push("enableInInbox");
        }
        if changed_keys.is_empty() {
            return Ok(FlowUpdated::default());
        }
        tx.execute(
            r#"UPDATE "Flow" SET "name" = COALESCE(?1, "name"), "active" = COALESCE(?2, "active"),
                   "enableInInbox" = COALESCE(?3, "enableInInbox"), "updatedAt" = ?4
               WHERE "id" = ?5"#,
            params![
                data.name,
                data.active,
                data.enable_in_inbox,
                now_ms(),
                current.id
            ],
        )
        .map_err(repo_err)?;
        let sync = changed_keys
            .contains(&"name")
            .then_some(WelcomeSync::Rename);
        Ok(FlowUpdated { changed_keys, sync })
    })
    .await
}

/// `flowService.duplicate`: returns the id of the copy.
pub async fn duplicate(db: &Db, workspace_id: i64, id: i64) -> Result<i64> {
    tx::write(db, move |tx| duplicate_in(tx, workspace_id, id)).await
}

/// `flowService.deleteMany`: returns the deleted ids (the ids of the
/// original `enqueueDelete` job; empty = no job).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Vec<i64>> {
    tx::write(db, move |tx| {
        let ids = repo::list_ids_by_ids(tx, workspace_id, &ids)?;
        if ids.is_empty() {
            return Ok(ids);
        }
        let list = Value::from(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>()).to_string();
        // Deleting a flow cascades to its versions, ref links and QR codes.
        tx.execute(
            r#"DELETE FROM "Flow" WHERE "workspaceId" = ?1
                 AND "id" IN (SELECT CAST(value AS INTEGER) FROM json_each(?2))"#,
            params![workspace_id, list],
        )
        .map_err(repo_err)?;
        tx.execute(
            r#"UPDATE "FlowAnalyticsSession" SET "deletedAt" = ?1
               WHERE "workspaceId" = ?2 AND "deletedAt" IS NULL
                 AND "flowId" IN (SELECT CAST(value AS INTEGER) FROM json_each(?3))"#,
            params![now_ms(), workspace_id, list],
        )
        .map_err(repo_err)?;
        Ok(ids)
    })
    .await
}

/// The input of [`create_published_default`].
#[derive(Debug, Clone, PartialEq)]
pub struct PublishedDefault {
    pub workspace_id: i64,
    pub name: String,
    pub folder_id: Option<i64>,
    pub start_node_id: i64,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
}

/// The ids [`create_published_default`] inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedDefaultIds {
    pub flow_id: i64,
    pub draft_version_id: i64,
    pub published_version_id: i64,
}

/// `flowService.createPublishedDefault` (template install, caller's
/// transaction): the flow, its analytics session, a draft and a published
/// version with the same graph.
pub fn create_published_default(
    tx: &Connection,
    input: PublishedDefault,
) -> Result<PublishedDefaultIds> {
    let ids = PublishedDefaultIds {
        flow_id: new_id(),
        draft_version_id: new_id(),
        published_version_id: new_id(),
    };
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "name", "active", "enableInInbox", "workspaceId", "folderId",
               "currentVersionId", "draftVersionId")
           VALUES (?1, ?2, 1, 0, ?3, ?4, ?5, ?6)"#,
        params![
            ids.flow_id,
            input.name,
            input.workspace_id,
            input.folder_id,
            ids.published_version_id,
            ids.draft_version_id
        ],
    )
    .map_err(repo_err)?;
    insert_analytics_session(tx, input.workspace_id, ids.flow_id)?;
    let nodes = Value::Array(input.nodes);
    let edges = Value::Array(input.edges);
    for (id, is_draft) in [
        (ids.draft_version_id, true),
        (ids.published_version_id, false),
    ] {
        insert_graph(
            tx,
            id,
            input.workspace_id,
            ids.flow_id,
            (&nodes, &edges),
            input.start_node_id,
            is_draft,
        )?;
    }
    Ok(ids)
}

/// A node id as text (ids are strings; a number is printed).
fn node_id_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `readNodeLabel`: the node's non-blank `data.name`, else its id.
fn read_node_label(node: &Value) -> String {
    match node["data"]["name"].as_str() {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => node_id_text(&node["id"]),
    }
}

fn find_flow(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Flow>> {
    Ok(find_one::<Flow>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?)
}

fn find_version(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
    draft: bool,
) -> Result<Option<FlowVersion>> {
    let filter = if draft {
        "\"flowId\" = ?1 AND \"workspaceId\" = ?2 AND \"isDraft\" = 1"
    } else {
        "\"flowId\" = ?1 AND \"workspaceId\" = ?2 AND \"isDraft\" = 0 AND \"isLatest\" = 1"
    };
    Ok(find_one::<FlowVersion>(
        conn,
        filter,
        &[&flow_id, &workspace_id],
    )?)
}

fn insert_graph(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    flow_id: i64,
    (nodes, edges): (&Value, &Value),
    start_node_id: i64,
    is_draft: bool,
) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft",
               "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            id,
            workspace_id,
            flow_id,
            nodes.to_string(),
            edges.to_string(),
            is_draft,
            !is_draft,
            start_node_id
        ],
    )
    .map_err(repo_err)?;
    Ok(())
}

fn list_in(conn: &Connection, input: ListFlows) -> Result<FlowListPage> {
    // Snowflake ids grow with creation time: the id tie-break keeps the
    // order of rows created in the same millisecond.
    let sort = input.sort.clone().map(|mut items| {
        let desc = items.last().is_some_and(|item| item.desc);
        if !items.iter().any(|item| item.id == "id") {
            items.push(SortItem {
                id: "id".into(),
                desc,
            });
        }
        items
    });
    let mut query = FlowListInput {
        workspace_id: input.workspace_id,
        name: input.name.clone(),
        folder_id: input.folder_id.clone(),
        active: input.active,
        enable_in_inbox: input.enable_in_inbox,
        page: input.page,
        per_page: input.per_page,
        sort,
    };
    let pagination = parse_pagination(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let Some(start_type) = input.start_type.as_deref() else {
        let data = repo::list_with_versions(conn, &query)?;
        let total = repo::count(conn, &query)?;
        let page_count = match pagination {
            Some(p) if p.limit > 0 => (total + p.limit - 1) / p.limit,
            _ => 1,
        };
        return Ok(FlowListPage { data, page_count });
    };
    query.page = None;
    query.per_page = None;
    let flows = repo::list_with_versions(conn, &query)?;
    let mut filtered = filter_flows_by_start_step_type(flows, start_type);
    if start_type == "sendWaTemplateMessage" {
        filtered = match input.integration_whatsapp_id {
            None => Vec::new(),
            Some(integration) => {
                integration_whatsapp::find_by_id_for_workspace(
                    conn,
                    integration,
                    input.workspace_id,
                )?
                .ok_or_else(|| BusinessError::not_found("WhatsApp channel not found"))?;
                let template_ids: Vec<String> =
                    small_repos::whatsapp_message_template_list_ids_by_integration(
                        conn,
                        integration,
                    )?
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                filter_flows_by_template_ids(filtered, &template_ids)
            }
        };
    }
    let Some(p) = pagination.filter(|p| p.limit > 0) else {
        return Ok(FlowListPage {
            data: filtered,
            page_count: 1,
        });
    };
    let total = filtered.len() as i64;
    let data = filtered
        .into_iter()
        .skip(usize::try_from(p.offset).unwrap_or_default())
        .take(usize::try_from(p.limit).unwrap_or_default())
        .collect();
    Ok(FlowListPage {
        data,
        page_count: (total + p.limit - 1) / p.limit,
    })
}

fn duplicate_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<i64> {
    let flow = find_flow(conn, workspace_id, id)?
        .ok_or_else(|| BusinessError::not_found("Flow not found"))?;
    let draft = find_version(conn, workspace_id, flow.id, true)?
        .ok_or_else(|| BusinessError::not_found("Draft version not found"))?;
    let published = find_version(conn, workspace_id, flow.id, false)?;
    let prefix = crate::copy_name::copy_name_prefix(&flow.name, FLOW_COPY_NAME);
    let taken: HashSet<String> = repo::list_names_with_prefix(conn, workspace_id, &prefix)?
        .into_iter()
        .collect();
    let name = free_copy_names(&flow.name, &taken, FLOW_COPY_NAME)
        .into_iter()
        .next()
        .ok_or_else(|| BusinessError::validation("name", "Name is already taken"))?;
    let copy_id = new_id();
    let maps = BTreeMap::from([(
        "flow".to_string(),
        BTreeMap::from([(flow.id.to_string(), copy_id.to_string())]),
    )]);
    let point_at_copy = |version: &FlowVersion| {
        let graph = remap_flow_graph_references(
            &json!({ "nodes": version.nodes.0, "edges": version.edges.0 }),
            &maps,
            RemapOptions {
                kinds: Some(vec!["flow".to_string()]),
                on_unresolved: None,
            },
        );
        (graph["nodes"].clone(), graph["edges"].clone())
    };
    let draft_graph = point_at_copy(&draft);
    let published_graph = published
        .as_ref()
        .map(|v| (point_at_copy(v), v.start_node_id));
    let draft_version_id = new_id();
    let published_version_id = published_graph.as_ref().map(|_| new_id());
    conn.execute(
        r#"INSERT INTO "Flow" ("id", "name", "active", "enableInInbox", "workspaceId", "folderId",
               "currentVersionId", "draftVersionId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            copy_id,
            name,
            published.is_some() && flow.active,
            flow.enable_in_inbox,
            workspace_id,
            flow.folder_id,
            published_version_id,
            draft_version_id
        ],
    )
    .map_err(repo_err)?;
    insert_analytics_session(conn, workspace_id, copy_id)?;
    insert_graph(
        conn,
        draft_version_id,
        workspace_id,
        copy_id,
        (&draft_graph.0, &draft_graph.1),
        draft.start_node_id,
        true,
    )?;
    if let (Some(((nodes, edges), start)), Some(version_id)) =
        (published_graph, published_version_id)
    {
        insert_graph(
            conn,
            version_id,
            workspace_id,
            copy_id,
            (&nodes, &edges),
            start,
            false,
        )?;
    }
    Ok(copy_id)
}
