//! `features/flows/api/public.ts` (P3.6-D2): `flows.create`, `list`, `get`,
//! `listNodes`, `update`, `delete`, `duplicate`. The other flow routes are
//! not ported yet.

use chatbotx_business::BusinessError;
use chatbotx_business::flow::{self, FlowGraphInput, FlowUpdate, ListFlows};
use chatbotx_core::flow_config::authoring::flow_spec_schema;
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
        "flows.publish" | "flows.validate" | "flows.updateDraft" | "flows.versions" => {
            todo!("P3.6-D2")
        }
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

/// `publishFlowRequest` (`flowSpecRequest` | `publishFlowSchema`); `Err` = the issues.
pub fn publish_flow_request(body: &Map<String, Value>) -> Result<GraphBody, Vec<Value>> {
    let _ = body;
    todo!("P3.6-D2")
}

/// `updateDraftFlowRequest` (`flowSpecRequest` | `updateDraftFlowVersionSchema`).
pub fn update_draft_flow_request(body: &Map<String, Value>) -> Result<GraphBody, Vec<Value>> {
    let _ = body;
    todo!("P3.6-D2")
}
