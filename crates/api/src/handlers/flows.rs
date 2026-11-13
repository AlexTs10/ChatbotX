//! `features/flows/api/public.ts` (P3.6-D2): `flows.create`, `list`, `get`,
//! `listNodes`, `update`, `delete`, `duplicate`. The other flow routes are
//! not ported yet.

use chatbotx_business::BusinessError;
use chatbotx_business::flow::{self, FlowGraphInput, FlowUpdate, ListFlows};
use chatbotx_business::flow_version;
use chatbotx_core::flow_config::authoring::flow_spec_schema;
use chatbotx_core::flow_config::import_export::FLOW_EXPORT_FORMAT_VERSION;
use chatbotx_core::flow_config::nodes::edge_schema;
use chatbotx_core::flow_config::publish_schema::publish_flow_schema;
use chatbotx_core::flow_config::z;
use chatbotx_db::repo::flow::FlowWithVersions;
use chatbotx_db::repo::models::{Flow, FlowVersion};
use serde_json::{Map, Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::{public_paging, sort_query};
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "flows.create",
    "flows.list",
    "flows.get",
    "flows.listNodes",
    "flows.update",
    "flows.delete",
    "flows.duplicate",
    "flows.publish",
    "flows.validate",
    "flows.updateDraft",
    "flows.versions",
    "flows.export",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "flows.create" => create(state, call).await,
        "flows.list" => list(state, call).await,
        "flows.get" => get(state, call).await,
        "flows.listNodes" => list_nodes(state, call).await,
        "flows.update" => update(state, call).await,
        "flows.delete" => delete(state, call).await,
        "flows.duplicate" => duplicate(state, call).await,
        "flows.publish" => publish(state, call).await,
        "flows.validate" => validate(state, call).await,
        "flows.updateDraft" => update_draft(state, call).await,
        "flows.versions" => versions(state, call).await,
        "flows.export" => export(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

fn array_key(input: &mut Input, name: &str, min: usize) -> Option<Vec<Value>> {
    match input.obj.get(name).cloned() {
        None => None,
        Some(Value::Array(items)) if items.len() >= min => Some(items),
        Some(Value::Array(_)) => {
            input.issue(
                &key(name),
                "too_small",
                &format!("Too small: expected array to have >={min} items"),
            );
            None
        }
        Some(_) => {
            input.issue(&key(name), "invalid_type", "Invalid input: expected array");
            None
        }
    }
}

/// `createFlowRequest` + the create handler.
async fn create(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let name = input.string("name", Str::trimmed(1, 255));
    let folder_id = match input.obj.get("folderId").cloned() {
        None => {
            input.issue(
                &key("folderId"),
                "invalid_type",
                "Invalid input: expected string, received undefined",
            );
            None
        }
        Some(Value::Null) => None,
        Some(value) => input.id_at(&key("folderId"), Some(&value), true),
    };
    let spec = input
        .obj
        .get("spec")
        .cloned()
        .and_then(|value| spec_at(&mut input, &value));
    let nodes = array_key(&mut input, "nodes", 1);
    let edges = array_key(&mut input, "edges", 0);
    let publish = input.opt_bool("publish");
    // `superRefine`: runs only on an otherwise valid body.
    if !input.failed() {
        if input.has("spec") && (input.has("nodes") || input.has("edges")) {
            input.issue(
                &key("spec"),
                "custom",
                "Cannot combine \"spec\" with \"nodes\"/\"edges\" in the same request body.",
            );
        }
        if input.has("edges") && !input.has("nodes") {
            input.issue(&key("edges"), "custom", "\"edges\" requires \"nodes\".");
        }
        if publish == Some(true) && !input.has("spec") && !input.has("nodes") {
            input.issue(
                &key("publish"),
                "custom",
                "\"publish\" requires \"spec\" or \"nodes\".",
            );
        }
    }
    let publish = publish.unwrap_or(false);
    input.finish()?;
    let ws = call.workspace_id;
    let db = &state.db;
    let name = name.unwrap_or_default();
    async {
        let graph =
            flow::resolve_graph_input(db, ws, FlowGraphInput { spec, nodes, edges }, publish)
                .await?;
        let node_ids = graph.as_ref().and_then(|g| g.node_ids.clone());
        let flow_id = match graph {
            Some(graph) if publish => {
                flow::create_published(db, ws, &name, folder_id, graph).await?
            }
            graph => flow::create_draft(db, ws, &name, folder_id, graph).await?,
        };
        Ok::<_, BusinessError>((flow_id, node_ids))
    }
    .await
    .map(|(flow_id, node_ids)| {
        let mut out = Map::new();
        out.insert("id".into(), id(flow_id));
        if let Some(node_ids) = node_ids {
            out.insert(
                "nodeIds".into(),
                Value::Object(node_ids.into_iter().map(|(k, v)| (k, json!(v))).collect()),
            );
        }
        json(201, Value::Object(out))
    })
    .map_err(|e| flow_error(e, super::CREATING))
}

/// [`business_error`]; a `FlowAuthoringException` (`invalidRequestData` with
/// the authoring errors as `data`) is not a defined error of the route.
fn flow_error(error: BusinessError, errors: super::ErrorMap) -> crate::orpc::OrpcError {
    let mut out = business_error(error, errors);
    if out.code == "invalidRequestData" {
        out.defined = false;
    }
    out
}

/// `flowSpecSchema` at `spec` (the parsed spec, or its issues under `spec`).
fn spec_at(input: &mut Input, value: &Value) -> Option<Value> {
    let parsed = flow_spec_schema().safe_parse(value);
    if parsed.success {
        return parsed.data;
    }
    let issues = parsed
        .issues()
        .iter()
        .filter_map(|issue| serde_json::to_value(issue).ok())
        .map(|mut issue| {
            let mut path = vec![json!("spec")];
            path.extend(issue["path"].as_array().cloned().unwrap_or_default());
            issue["path"] = Value::Array(path);
            issue
        })
        .collect();
    input.extend_issues(issues);
    None
}

/// `flowResource` (no versions).
fn flow_json(flow: &Flow) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), id(flow.id));
    out.insert("createdAt".into(), date(flow.created_at));
    out.insert("updatedAt".into(), date(flow.updated_at));
    out.insert("name".into(), json!(flow.name));
    out.insert("active".into(), json!(flow.active));
    out.insert("enableInInbox".into(), json!(flow.enable_in_inbox));
    out.insert("currentVersionId".into(), opt_id(flow.current_version_id));
    out.insert("draftVersionId".into(), opt_id(flow.draft_version_id));
    out.insert("workspaceId".into(), id(flow.workspace_id));
    out.insert("folderId".into(), opt_id(flow.folder_id));
    out
}

/// `flowVersionResource` (no `updatedAt`).
pub fn flow_version_json(version: &FlowVersion) -> Value {
    json!({
        "id": id(version.id),
        "createdAt": date(version.created_at),
        "workspaceId": id(version.workspace_id),
        "flowId": id(version.flow_id),
        "nodes": version.nodes.0,
        "edges": version.edges.0,
        "isDraft": version.is_draft,
        "isLatest": version.is_latest,
        "startNodeId": id(version.start_node_id),
    })
}

/// `flowWithVersionsResource`.
fn flow_with_versions_json(flow: &FlowWithVersions) -> Map<String, Value> {
    let mut out = flow_json(&flow.flow);
    out.insert(
        "flowVersions".into(),
        Value::Array(flow.flow_versions.iter().map(flow_version_json).collect()),
    );
    out
}

/// A query `z.boolean()` (`"true"` / `"false"`).
fn query_bool(input: &mut Input, name: &str) -> Option<bool> {
    match input.get(name).cloned() {
        None => None,
        Some(Value::String(text)) if text == "true" => Some(true),
        Some(Value::String(text)) if text == "false" => Some(false),
        Some(_) => {
            input.issue(
                &key(name),
                "invalid_type",
                "Invalid input: expected boolean, received string",
            );
            None
        }
    }
}

/// A nullish query `zodBigintAsString()`.
fn query_id_text(input: &mut Input, name: &str) -> Option<String> {
    let value = input.get(name).cloned()?;
    input
        .id_at(&key(name), Some(&value), false)
        .map(|_| value.as_str().unwrap_or_default().to_string())
}

/// `flows.list`.
async fn list(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let (page, per_page) = public_paging(&mut input);
    let active = query_bool(&mut input, "active").unwrap_or(true);
    let enable_in_inbox = query_bool(&mut input, "enableInInbox");
    let name = input
        .get("name")
        .cloned()
        .and_then(|value| input.string_at(&key("name"), Some(&value), Str::default(), false));
    let folder_id = query_id_text(&mut input, "folderId");
    let start_type = input.get("startType").cloned().and_then(|value| {
        input.enum_at(
            &key("startType"),
            Some(&value),
            &["sendWaTemplateMessage", "sendMessengerTemplateMessage"],
            false,
        )
    });
    let integration_whatsapp_id = input
        .get("integrationWhatsappId")
        .cloned()
        .and_then(|value| input.id_at(&key("integrationWhatsappId"), Some(&value), false));
    let sort = sort_query(&mut input, Some(&["name", "createdAt", "updatedAt"]));
    input.finish()?;
    let page = flow::list(
        &state.db,
        ListFlows {
            workspace_id: call.workspace_id,
            name,
            folder_id,
            active: Some(active),
            enable_in_inbox,
            page: Some(page),
            per_page: Some(per_page),
            sort: Some(sort.unwrap_or_else(|| {
                vec![chatbotx_db::repo::SortItem {
                    id: "createdAt".into(),
                    desc: true,
                }]
            })),
            start_type,
            integration_whatsapp_id,
        },
    )
    .await
    .map_err(|e| business_error(e, super::FINDING))?;
    let data: Vec<Value> = page
        .data
        .iter()
        .map(|row| {
            json!({
                "id": id(row.flow.id),
                "name": row.flow.name,
                "folderId": opt_id(row.flow.folder_id),
                "active": row.flow.active,
                "enableInInbox": row.flow.enable_in_inbox,
            })
        })
        .collect();
    Ok(json(
        200,
        json!({ "data": data, "pageCount": page.page_count }),
    ))
}

/// The `{id}` path param.
fn path_id(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let flow_id = input.id("id");
    input.finish()?;
    Ok(flow_id.unwrap_or_default())
}

/// `flows.get`.
async fn get(state: &ApiState, call: Call<'_>) -> Reply {
    let flow_id = path_id(&call)?;
    let ws = call.workspace_id;
    let flow = flow::find_by_id(&state.db, ws, flow_id)
        .await
        .map_err(|e| business_error(e, super::FINDING))?;
    let welcome = flow::list_welcome_message_flows(&state.db, ws, flow_id)
        .await
        .map_err(|e| business_error(e, super::FINDING))?;
    let mut out = flow_with_versions_json(&flow);
    out.insert(
        "welcomeMessageFlows".into(),
        Value::Array(
            welcome
                .into_iter()
                .map(|row| {
                    json!({
                        "integrationMessengerId": id(row.integration_messenger_id),
                        "pageId": row.page_id,
                        "welcomeMessageFlowId": row.welcome_message_flow_id,
                    })
                })
                .collect(),
        ),
    );
    Ok(json(200, Value::Object(out)))
}

/// `flows.listNodes`.
async fn list_nodes(state: &ApiState, call: Call<'_>) -> Reply {
    let flow_id = path_id(&call)?;
    let nodes = flow::list_nodes(&state.db, call.workspace_id, flow_id)
        .await
        .map_err(|e| business_error(e, super::FINDING))?;
    let data: Vec<Value> = nodes
        .into_iter()
        .map(|node| {
            json!({
                "flowId": id(node.flow_id),
                "nodeId": node.node_id,
                "label": node.label,
                "nodeType": node.node_type,
            })
        })
        .collect();
    Ok(json(200, json!({ "data": data })))
}

/// `flows.update` (`updateFlowSchema`).
async fn update(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let name = input.opt_string("name", Str::trimmed(1, 255));
    let active = input.opt_bool("active");
    let enable_in_inbox = input.opt_bool("enableInInbox");
    let flow_id = input.id("id");
    input.finish()?;
    flow::update(
        &state.db,
        call.workspace_id,
        flow_id.unwrap_or_default(),
        FlowUpdate {
            name,
            active,
            enable_in_inbox,
        },
    )
    .await
    .map_err(|e| business_error(e, super::MUTATING))?;
    Ok(no_content())
}

/// `flows.delete`.
async fn delete(state: &ApiState, call: Call<'_>) -> Reply {
    let flow_id = path_id(&call)?;
    flow::delete_many(&state.db, call.workspace_id, vec![flow_id])
        .await
        .map_err(|e| business_error(e, super::MUTATING))?;
    Ok(no_content())
}

/// `flows.duplicate`.
async fn duplicate(state: &ApiState, call: Call<'_>) -> Reply {
    let flow_id = path_id(&call)?;
    let copy = flow::duplicate(&state.db, call.workspace_id, flow_id)
        .await
        .map_err(|e| business_error(e, super::MUTATING))?;
    Ok(json(201, json!({ "id": id(copy) })))
}

/// The graph part of a publish / updateDraft body.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphBody {
    /// `{ spec }` (the parsed spec).
    Spec(Value),
    /// `{ nodes, edges }`.
    Graph {
        nodes: Vec<Value>,
        edges: Vec<Value>,
    },
}

fn zod_issues(issues: &[chatbotx_core::flow_config::z::Issue]) -> Vec<Value> {
    issues
        .iter()
        .filter_map(|issue| serde_json::to_value(issue).ok())
        .collect()
}

fn custom_root_issue(message: &str) -> Value {
    json!({ "code": "custom", "path": [], "message": message })
}

/// `flowSpecRequest`: `{ spec }`, no `nodes` / `edges`.
fn flow_spec_request(body: &Map<String, Value>) -> Result<Value, Vec<Value>> {
    if body.contains_key("nodes") || body.contains_key("edges") {
        return Err(vec![custom_root_issue(
            "Cannot combine \"nodes\"/\"edges\" with \"spec\" in the same request body.",
        )]);
    }
    let parsed =
        z::object(vec![("spec", flow_spec_schema())]).safe_parse(&Value::Object(body.clone()));
    if parsed.success {
        Ok(parsed
            .data
            .and_then(|d| d.get("spec").cloned())
            .unwrap_or_default())
    } else {
        Err(zod_issues(parsed.issues()))
    }
}

/// A `{ nodes, edges }` member of a request union (no `spec`).
fn graph_request(body: &Map<String, Value>, schema: z::Schema) -> Result<GraphBody, Vec<Value>> {
    if body.contains_key("spec") {
        return Err(vec![custom_root_issue(
            "Cannot combine \"spec\" with \"nodes\"/\"edges\" in the same request body.",
        )]);
    }
    let parsed = schema.safe_parse(&Value::Object(body.clone()));
    if !parsed.success {
        return Err(zod_issues(parsed.issues()));
    }
    // The stored graph is the body's own (the parsed one drops unknown keys).
    let list = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    Ok(GraphBody::Graph {
        nodes: list("nodes"),
        edges: list("edges"),
    })
}

/// `z.union([flowSpecRequest, graph])`: one `invalid_union` issue at the root.
fn spec_or_graph(body: &Map<String, Value>, graph: z::Schema) -> Result<GraphBody, Vec<Value>> {
    let spec_issues = match flow_spec_request(body) {
        Ok(spec) => return Ok(GraphBody::Spec(spec)),
        Err(issues) => issues,
    };
    let graph_issues = match graph_request(body, graph) {
        Ok(out) => return Ok(out),
        Err(issues) => issues,
    };
    Err(vec![json!({
        "code": "invalid_union",
        "errors": [spec_issues, graph_issues],
        "path": [],
        "message": "Invalid input",
    })])
}

/// `publishFlowRequest` (`flowSpecRequest` | `publishFlowSchema`); `Err` = the issues.
pub fn publish_flow_request(body: &Map<String, Value>) -> Result<GraphBody, Vec<Value>> {
    spec_or_graph(body, publish_flow_schema())
}

/// `updateDraftFlowRequest` (`flowSpecRequest` | `updateDraftFlowVersionSchema`).
pub fn update_draft_flow_request(body: &Map<String, Value>) -> Result<GraphBody, Vec<Value>> {
    spec_or_graph(
        body,
        z::object(vec![
            ("nodes", z::array(z::any())),
            ("edges", z::array(edge_schema())),
        ]),
    )
}

/// A publish / updateDraft body parser.
type RequestParser = fn(&Map<String, Value>) -> Result<GraphBody, Vec<Value>>;

/// The `{id}` param and the parsed body of a publish / updateDraft call.
fn graph_call(
    call: &Call<'_>,
    parse: RequestParser,
) -> Result<(i64, GraphBody), crate::orpc::OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let parsed = parse(&input.obj);
    let flow_id = input.id("id");
    let body = match parsed {
        Ok(body) => Some(body),
        Err(issues) => {
            input.extend_issues(issues);
            None
        }
    };
    input.finish()?;
    Ok((
        flow_id.unwrap_or_default(),
        body.unwrap_or(GraphBody::Graph {
            nodes: vec![],
            edges: vec![],
        }),
    ))
}

/// The graph of a body: a spec is compiled (`validate` = `compileAndValidateSpec`).
async fn graph_of(
    state: &ApiState,
    workspace_id: i64,
    body: GraphBody,
    validate: bool,
) -> chatbotx_business::Result<(Vec<Value>, Vec<Value>)> {
    match body {
        GraphBody::Graph { nodes, edges } => Ok((nodes, edges)),
        GraphBody::Spec(spec) => {
            let graph = flow::resolve_graph_input(
                &state.db,
                workspace_id,
                FlowGraphInput {
                    spec: Some(spec),
                    nodes: None,
                    edges: None,
                },
                validate,
            )
            .await?
            .unwrap_or(flow::FlowGraph {
                nodes: vec![],
                edges: vec![],
                start_node_id: String::new(),
                node_ids: None,
            });
            Ok((graph.nodes, graph.edges))
        }
    }
}

/// `flows.publish`.
async fn publish(state: &ApiState, call: Call<'_>) -> Reply {
    let (flow_id, body) = graph_call(&call, publish_flow_request)?;
    let ws = call.workspace_id;
    async {
        let (nodes, edges) = graph_of(state, ws, body, true).await?;
        flow_version::publish(&state.db, ws, flow_id, nodes, edges).await
    }
    .await
    .map_err(|e| flow_error(e, super::MUTATING))?;
    Ok(no_content())
}

/// `flows.validate` (`flowSpecRequest` -> `publishFlowSchema`).
async fn validate(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let spec = match flow_spec_request(&input.obj) {
        Ok(spec) => Some(spec),
        Err(issues) => {
            input.extend_issues(issues);
            None
        }
    };
    input.finish()?;
    let (nodes, edges) = graph_of(
        state,
        call.workspace_id,
        GraphBody::Spec(spec.unwrap_or_default()),
        true,
    )
    .await
    .map_err(|e| flow_error(e, super::MUTATING))?;
    Ok(json(200, json!({ "nodes": nodes, "edges": edges })))
}

/// `flows.updateDraft`.
async fn update_draft(state: &ApiState, call: Call<'_>) -> Reply {
    let (flow_id, body) = graph_call(&call, update_draft_flow_request)?;
    let ws = call.workspace_id;
    async {
        let (nodes, edges) = graph_of(state, ws, body, false).await?;
        flow_version::update_draft_by_flow_id(&state.db, ws, flow_id, nodes, edges).await
    }
    .await
    .map_err(|e| flow_error(e, super::MUTATING))?;
    Ok(no_content())
}

/// `flows.versions`.
async fn versions(state: &ApiState, call: Call<'_>) -> Reply {
    let flow_id = path_id(&call)?;
    let data = flow_version::list(&state.db, call.workspace_id, flow_id)
        .await
        .map_err(|e| business_error(e, super::FINDING))?;
    Ok(json(
        200,
        json!({ "data": data.iter().map(flow_version_json).collect::<Vec<_>>() }),
    ))
}

/// `flows.export`: `{ fileName, export }` (format version 2).
async fn export(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let flow_id = input.id("id");
    let version = input.get("version").cloned().and_then(|value| {
        input.enum_at(
            &key("version"),
            Some(&value),
            &["published", "draft"],
            false,
        )
    });
    input.finish()?;
    let out = flow::build_export(
        &state.db,
        call.workspace_id,
        flow_id.unwrap_or_default(),
        version.as_deref() == Some("draft"),
    )
    .await
    .map_err(|e| business_error(e, super::FINDING))?;
    let fields = |rows: &[(i64, String, String)]| -> Map<String, Value> {
        rows.iter()
            .map(|(field_id, name, field_type)| {
                (
                    field_id.to_string(),
                    json!({ "name": name, "type": field_type }),
                )
            })
            .collect()
    };
    let envelope = json!({
        "formatVersion": FLOW_EXPORT_FORMAT_VERSION,
        "exportedAt": date(chatbotx_db::repo::now_ms()),
        "source": { "workspaceId": id(out.flow.workspace_id), "flowId": id(out.flow.id) },
        "flows": [{
            "name": out.flow.name,
            "active": out.flow.active,
            "enableInInbox": out.flow.enable_in_inbox,
            "startNodeId": id(out.version.start_node_id),
            "nodes": out.version.nodes.0,
            "edges": out.version.edges.0,
        }],
        "customFields": fields(&out.custom_fields),
        "botFields": fields(&out.bot_fields),
    });
    Ok(json(
        200,
        json!({ "fileName": out.file_name, "export": envelope }),
    ))
}
