//! `features/flows/api/public.ts`: `flows.create` (P3.6-D2). The other flow
//! routes are not ported yet.

use chatbotx_business::BusinessError;
use chatbotx_business::flow::{self, FlowGraphInput};
use serde_json::{Map, Value, json};

use super::input::{Input, Str, key};
use super::{ApiState, Call, Reply, business_error, id, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["flows.create"];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "flows.create" => create(state, call).await,
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
    let spec = match input.obj.get("spec").cloned() {
        None => None,
        Some(Value::Object(map)) => Some(Value::Object(map)),
        Some(_) => {
            input.issue(
                &key("spec"),
                "invalid_type",
                "Invalid input: expected object",
            );
            None
        }
    };
    let nodes = array_key(&mut input, "nodes", 1);
    let edges = array_key(&mut input, "edges", 0);
    let publish = input.opt_bool("publish").unwrap_or(false);
    if input.has("spec") && (input.has("nodes") || input.has("edges")) {
        input.issue(
            &key("spec"),
            "custom",
            "Cannot combine \"spec\" with \"nodes\"/\"edges\" in the same request body.",
        );
    }
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
    .map_err(|e| {
        let mut out = business_error(e, super::CREATING);
        if out.code == "invalidRequestData" && out.status == 422 {
            out.defined = true;
        }
        out
    })
}
