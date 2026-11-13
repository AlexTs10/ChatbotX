//! `flow/service.ts` (`createDraft`, `createPublished`,
//! `createPublishedDefault`), `flow-version/assert-publishable.ts`,
//! `capabilities/service.ts` `getFlowAuthoringContext` (flows and custom
//! fields) and the builder's `lib/resolve-flow-graph-input.ts` /
//! `lib/compile-spec-to-graph.ts` (P3.6-D2: the create path).

use std::collections::BTreeMap;

use chatbotx_core::flow_config::authoring::{
    AuthoringCustomField, AuthoringFlowRef, FlowAuthoringContext, FlowAuthoringException,
    compile_flow_spec, format_zod_path_segment, normalize_authored_graph,
    zod_error_to_flow_authoring_errors,
};
use chatbotx_core::flow_config::nodes::send_message_node_default_fn;
use chatbotx_core::flow_config::publish_schema::publish_flow_schema;
use chatbotx_core::flow_config::z::PathSegment;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{BusinessError, Result, tx};

mod crud;
pub mod filters;

pub use crud::*;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// The oRPC error of a `FlowAuthoringException` (`toKnownOrpcError`).
fn authoring_error(error: FlowAuthoringException) -> BusinessError {
    BusinessError::Exception {
        message: error.message(),
        code: "invalidRequestData".to_string(),
        status: 422,
        field: None,
        data: serde_json::to_value(&error.errors).ok(),
    }
}

fn new_id() -> i64 {
    chatbotx_core::utils::id::create_id()
        .parse()
        .unwrap_or_default()
}

/// `getFlowAuthoringContext` (flows and custom fields of the workspace;
/// templates and pipelines are not loaded yet).
fn authoring_context(conn: &Connection, workspace_id: i64) -> Result<FlowAuthoringContext> {
    let mut flows = BTreeMap::new();
    let mut statement = conn
        .prepare(r#"SELECT "id", "name" FROM "Flow" WHERE "workspaceId" = ?1 ORDER BY "id""#)
        .map_err(repo_err)?;
    let rows = statement
        .query_map([workspace_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(repo_err)?;
    for row in rows {
        let (id, name) = row.map_err(repo_err)?;
        flows.insert(name, AuthoringFlowRef { id: id.to_string() });
    }
    let mut fields = BTreeMap::new();
    let mut statement = conn
        .prepare(r#"SELECT "id", "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 ORDER BY "id""#)
        .map_err(repo_err)?;
    let rows = statement
        .query_map([workspace_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(repo_err)?;
    for row in rows {
        let (id, name, field_type) = row.map_err(repo_err)?;
        fields.insert(
            name,
            AuthoringCustomField {
                id: id.to_string(),
                field_type,
            },
        );
    }
    Ok(FlowAuthoringContext {
        templates_by_name: BTreeMap::new(),
        custom_fields_by_name: fields,
        flows_by_name: flows,
        pipelines_by_reference_name: Some(BTreeMap::new()),
    })
}

/// `publishFlowSchema.safeParse` + `zodErrorToFlowAuthoringErrors(code)`;
/// `spec_paths` maps a node issue back to its spec path.
fn validate_publishable(
    nodes: &[Value],
    edges: &[Value],
    code: &str,
    spec_paths: Option<&BTreeMap<String, String>>,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let parsed = publish_flow_schema().safe_parse(&json!({ "nodes": nodes, "edges": edges }));
    if let Some(error) = parsed.error.filter(|_| !parsed.success) {
        let mapper = |path: &[PathSegment]| -> Option<String> {
            let spec_paths = spec_paths?;
            let [
                PathSegment::Key(field),
                PathSegment::Index(index),
                rest @ ..,
            ] = path
            else {
                return None;
            };
            if field != "nodes" {
                return None;
            }
            let node_id = nodes.get(*index)?.get("id")?.as_str()?;
            let spec_path = spec_paths.get(node_id)?;
            Some(rest.iter().fold(spec_path.clone(), |acc, seg| {
                format_zod_path_segment(&acc, seg)
            }))
        };
        let errors = zod_error_to_flow_authoring_errors(&error, code, Some(&mapper));
        return Err(authoring_error(FlowAuthoringException::new(errors)));
    }
    let data = parsed.data.unwrap_or_default();
    let list = |key: &str| {
        data.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    Ok((list("nodes"), list("edges")))
}

/// `folderService.ensureExists` (a flow folder of the workspace).
fn ensure_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'flow'"#,
        params![folder_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Folder not found"))
}

fn insert_version(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    flow_id: i64,
    graph: &FlowGraph,
    is_draft: bool,
    is_latest: bool,
) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft",
               "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            id,
            workspace_id,
            flow_id,
            Value::Array(graph.nodes.clone()).to_string(),
            Value::Array(graph.edges.clone()).to_string(),
            is_draft,
            is_latest,
            graph.start_node_id.parse::<i64>().unwrap_or_default(),
        ],
    )
    .map_err(repo_err)?;
    Ok(())
}

fn insert_analytics_session(conn: &Connection, workspace_id: i64, flow_id: i64) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "FlowAnalyticsSession" ("id", "flowId", "workspaceId") VALUES (?1, ?2, ?3)"#,
        params![new_id(), flow_id, workspace_id],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `defaultDraftGraph`: one start `Send Message #1` node.
fn default_draft_graph() -> FlowGraph {
    let node = send_message_node_default_fn(&json!({
        "dataProps": { "name": "Send Message #1", "isStartNode": true },
    }));
    let start_node_id = node["id"].as_str().unwrap_or_default().to_string();
    FlowGraph {
        nodes: vec![node],
        edges: Vec::new(),
        start_node_id,
        node_ids: None,
    }
}

/// `ResolvedFlowGraph`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowGraph {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub start_node_id: String,
    /// Authored id -> internal id (`nodeIds`; raw graphs only).
    pub node_ids: Option<Vec<(String, String)>>,
}

/// The graph part of a create / publish body.
#[derive(Debug, Clone, Default)]
pub struct FlowGraphInput {
    pub spec: Option<Value>,
    pub nodes: Option<Vec<Value>>,
    pub edges: Option<Vec<Value>>,
}

/// `resolveFlowGraphInput`: compile a spec (with the workspace authoring
/// context) or normalize a raw graph; `validate` = the publish schema check.
/// A rejected graph is `422 invalidRequestData` whose `data` is the
/// `FlowAuthoringException` errors.
pub async fn resolve_graph_input(
    db: &Db,
    workspace_id: i64,
    input: FlowGraphInput,
    validate: bool,
) -> Result<Option<FlowGraph>> {
    if let Some(spec) = input.spec {
        let ctx = tx::read(db, move |conn| authoring_context(conn, workspace_id)).await?;
        let compiled = compile_flow_spec(&spec, &ctx).map_err(authoring_error)?;
        let (nodes, edges) = if validate {
            validate_publishable(
                &compiled.nodes,
                &compiled.edges,
                "invalidStep",
                Some(&compiled.spec_path_by_node_id),
            )?
        } else {
            (compiled.nodes, compiled.edges)
        };
        return Ok(Some(FlowGraph {
            nodes,
            edges,
            start_node_id: compiled.start_node_id,
            node_ids: None,
        }));
    }
    let Some(nodes) = input.nodes else {
        return Ok(None);
    };
    let graph = normalize_authored_graph(&nodes, &input.edges.unwrap_or_default())
        .map_err(authoring_error)?;
    let node_ids = Some(graph.node_ids.into_iter().collect());
    let (nodes, edges) = if validate {
        validate_publishable(&graph.nodes, &graph.edges, "invalidStep", None)?
    } else {
        (graph.nodes, graph.edges)
    };
    Ok(Some(FlowGraph {
        nodes,
        edges,
        start_node_id: graph.start_node_id,
        node_ids,
    }))
}

/// `flowService.createDraft`: the folder check, the flow, its analytics
/// session and one draft version (the default one-node graph when `graph`
/// is `None`). Returns the flow id.
pub async fn create_draft(
    db: &Db,
    workspace_id: i64,
    name: &str,
    folder_id: Option<i64>,
    graph: Option<FlowGraph>,
) -> Result<i64> {
    let name = name.to_string();
    let graph = graph.unwrap_or_else(default_draft_graph);
    tx::write(db, move |tx| {
        if let Some(folder_id) = folder_id {
            ensure_folder(tx, workspace_id, folder_id)?;
        }
        let flow_id = new_id();
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "workspaceId", "folderId") VALUES (?1, ?2, ?3, ?4)"#,
            params![flow_id, name, workspace_id, folder_id],
        )
        .map_err(repo_err)?;
        insert_analytics_session(tx, workspace_id, flow_id)?;
        insert_version(tx, new_id(), workspace_id, flow_id, &graph, true, false)?;
        Ok(flow_id)
    })
    .await
}

/// `flowService.createPublished`: the publish check, the folder check, the
/// flow with a draft and a published (latest) version. Returns the flow id.
pub async fn create_published(
    db: &Db,
    workspace_id: i64,
    name: &str,
    folder_id: Option<i64>,
    graph: FlowGraph,
) -> Result<i64> {
    // `assertFlowGraphPublishable`.
    validate_publishable(&graph.nodes, &graph.edges, "invalidGraph", None)?;
    let name = name.to_string();
    tx::write(db, move |tx| {
        if let Some(folder_id) = folder_id {
            ensure_folder(tx, workspace_id, folder_id)?;
        }
        let (flow_id, draft_id, published_id) = (new_id(), new_id(), new_id());
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "active", "enableInInbox", "workspaceId", "folderId",
                   "currentVersionId", "draftVersionId")
               VALUES (?1, ?2, 1, 0, ?3, ?4, ?5, ?6)"#,
            params![flow_id, name, workspace_id, folder_id, published_id, draft_id],
        )
        .map_err(repo_err)?;
        insert_analytics_session(tx, workspace_id, flow_id)?;
        insert_version(tx, draft_id, workspace_id, flow_id, &graph, true, false)?;
        insert_version(tx, published_id, workspace_id, flow_id, &graph, false, true)?;
        Ok(flow_id)
    })
    .await
}
