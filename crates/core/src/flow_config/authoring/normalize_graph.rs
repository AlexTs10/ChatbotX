//! Port of `packages/flow-config/src/authoring/normalize-graph.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `NormalizedAuthoredGraph`.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedAuthoredGraph {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub start_node_id: String,
    pub node_ids: std::collections::BTreeMap<String, String>,
}

/// `normalizeFlowGraphDecisionMappings(nodes)`.
pub fn normalize_flow_graph_decision_mappings(nodes: &[Value]) -> Vec<Value> {
    nodes
        .iter()
        .map(|node| {
            let Some(steps) = node["data"]["details"]["steps"].as_array() else {
                return node.clone();
            };
            if !node["data"]["details"].is_object() {
                return node.clone();
            }
            let steps: Vec<Value> = steps
                .iter()
                .map(|step| {
                    if step["stepType"] == "evaluateDecision" {
                        normalize_evaluate_decision_step(step)
                    } else {
                        step.clone()
                    }
                })
                .collect();
            let mut out = node.clone();
            out["data"]["details"]["steps"] = Value::Array(steps);
            out
        })
        .collect()
}

fn graph_error(path: String, message: String) -> FlowAuthoringError {
    FlowAuthoringError {
        path,
        code: "invalidGraph".into(),
        message,
        ..FlowAuthoringError::default()
    }
}

/// `normalizeAuthoredGraph(nodes, edges)`: fresh internal ids, a layout for
/// nodes without a position, exactly one start node.
pub fn normalize_authored_graph(
    nodes: &[Value],
    edges: &[Value],
) -> Result<NormalizedAuthoredGraph, FlowAuthoringException> {
    let mut errors = Vec::new();
    if nodes.is_empty() {
        errors.push(graph_error(
            "nodes".into(),
            "A flow needs at least one node.".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (index, node) in nodes.iter().enumerate() {
        let id = str_field(node, "id");
        if !seen.insert(id.to_owned()) {
            errors.push(graph_error(
                format!("nodes[{index}].id"),
                format!("Duplicate node id \"{id}\"."),
            ));
        }
    }
    for (index, edge) in edges.iter().enumerate() {
        let (source, target) = (str_field(edge, "source"), str_field(edge, "target"));
        if !seen.contains(source) {
            errors.push(graph_error(
                format!("edges[{index}].source"),
                format!("Edge source \"{source}\" does not match any node id."),
            ));
        }
        if !seen.contains(target) {
            errors.push(graph_error(
                format!("edges[{index}].target"),
                format!("Edge target \"{target}\" does not match any node id."),
            ));
        }
    }
    if !errors.is_empty() {
        return Err(FlowAuthoringException::new(errors));
    }
    let mut id_by_authored: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for node in nodes {
        id_by_authored
            .entry(str_field(node, "id").to_owned())
            .or_insert_with(create_id);
    }
    let start_authored = nodes
        .iter()
        .find(|n| n["data"]["isStartNode"] == true)
        .map_or_else(|| str_field(&nodes[0], "id"), |n| str_field(n, "id"))
        .to_owned();
    let start_node_id = id_by_authored[&start_authored].clone();
    let node_ids: Vec<&str> = nodes.iter().map(|n| str_field(n, "id")).collect();
    let layout_edges: Vec<LayoutEdge<'_>> = edges
        .iter()
        .map(|e| LayoutEdge {
            source: str_field(e, "source"),
            target: str_field(e, "target"),
        })
        .collect();
    let positions = layout_nodes(&node_ids, &layout_edges, &start_authored);
    let resolved_nodes = nodes
        .iter()
        .map(|node| {
            let authored = str_field(node, "id");
            let id = id_by_authored[authored].clone();
            let mut out = node.clone();
            out["id"] = json!(id);
            if node.get("position").is_none_or(Value::is_null) {
                out["position"] = positions.get(authored).map_or(
                    Value::Null,
                    |p| json!({ "x": num_value(p.x), "y": num_value(p.y) }),
                );
            }
            if node.get("measured").is_none_or(Value::is_null) {
                out["measured"] = default_node_measured().clone();
            }
            let mut data = node
                .get("data")
                .filter(|d| d.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            data["isStartNode"] = json!(id == start_node_id);
            out["data"] = data;
            out
        })
        .collect();
    let resolved_edges = edges
        .iter()
        .map(|edge| {
            let source = id_by_authored[str_field(edge, "source")].clone();
            let target = id_by_authored[str_field(edge, "target")].clone();
            json!({
                "id": edge.get("id").cloned().unwrap_or_else(|| json!(create_id())),
                "sourceHandle": edge.get("sourceHandle").cloned().unwrap_or_else(|| json!(source)),
                "targetHandle": edge.get("targetHandle").cloned().unwrap_or_else(|| json!(target)),
                "source": source,
                "target": target,
            })
        })
        .collect();
    Ok(NormalizedAuthoredGraph {
        nodes: resolved_nodes,
        edges: resolved_edges,
        start_node_id,
        node_ids: id_by_authored,
    })
}
