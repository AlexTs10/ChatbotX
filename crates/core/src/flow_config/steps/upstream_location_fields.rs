//! Port of `packages/flow-config/src/steps/upstream-location-fields.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

fn contains_step(value: &Value, step_id: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| contains_step(item, step_id)),
        Value::Object(map) => {
            if map.contains_key("stepType")
                && map.get("id").and_then(Value::as_str) == Some(step_id)
            {
                return true;
            }
            map.values().any(|item| contains_step(item, step_id))
        }
        _ => false,
    }
}

fn find_start_node<'a>(nodes: &'a [Value], step_id: &str) -> Option<(&'a Value, usize)> {
    for node in nodes {
        let details = &node["data"]["details"];
        if !contains_step(details, step_id) {
            continue;
        }
        let steps = arr_field(details, "steps");
        if let Some(index) = steps.iter().position(|step| step["id"] == step_id) {
            return Some((node, index));
        }
        let carrier = steps.iter().position(|step| contains_step(step, step_id));
        return Some((node, carrier.map_or(steps.len(), |c| c + 1)));
    }
    None
}

fn is_location_source(step: &Value) -> bool {
    step["stepType"] == "getUserData"
        && step["replyFormat"] == reply_format()["location"]
        && truthy(step.get("outputFieldId"))
        && !is_bot_field_reference(str_field(step, "outputFieldId"))
}

/// `findUpstreamLocationFieldKeys({ nodes, edges, stepId })`: the location
/// fields a step can read (earlier steps of its node, then upstream nodes).
pub fn find_upstream_location_field_keys(
    nodes: &[Value],
    edges: &[Value],
    step_id: &str,
) -> Vec<String> {
    let Some((start, steps_before)) = find_start_node(nodes, step_id) else {
        return Vec::new();
    };
    let nodes_by_id: std::collections::HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|n| n["id"].as_str().map(|id| (id, n)))
        .collect();
    let mut sources_by_target: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for edge in edges {
        if let (Some(source), Some(target)) = (edge["source"].as_str(), edge["target"].as_str()) {
            sources_by_target.entry(target).or_default().push(source);
        }
    }
    let mut keys: Vec<String> = Vec::new();
    let mut collect = |steps: &[Value]| {
        for step in steps {
            if is_location_source(step) {
                let key = str_field(step, "outputFieldId").to_owned();
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    };
    let start_steps = arr_field(&start["data"]["details"], "steps");
    collect(&start_steps[..steps_before.min(start_steps.len())]);
    let mut visited = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<&str> = sources_by_target
        .get(str_field(start, "id"))
        .cloned()
        .unwrap_or_default()
        .into();
    while let Some(node_id) = queue.pop_front() {
        if !visited.insert(node_id) {
            continue;
        }
        if let Some(node) = nodes_by_id.get(node_id) {
            collect(arr_field(&node["data"]["details"], "steps"));
        }
        if let Some(sources) = sources_by_target.get(node_id) {
            queue.extend(sources.iter().copied());
        }
    }
    keys
}
