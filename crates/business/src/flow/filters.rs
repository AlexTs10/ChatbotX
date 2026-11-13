//! `flow/filters.ts`: flows whose start node sends a template message.

use serde_json::Value;

/// The `nodes` of every version of a flow (`FlowWithNodeVersions`).
pub trait FlowNodeVersions {
    fn version_nodes(&self) -> Vec<&Value>;
}

impl FlowNodeVersions for Value {
    fn version_nodes(&self) -> Vec<&Value> {
        self["flowVersions"]
            .as_array()
            .map(|versions| versions.iter().map(|v| &v["nodes"]).collect())
            .unwrap_or_default()
    }
}

impl FlowNodeVersions for chatbotx_db::repo::flow::FlowWithVersions {
    fn version_nodes(&self) -> Vec<&Value> {
        self.flow_versions.iter().map(|v| &v.nodes.0).collect()
    }
}

const SEND_WA_TEMPLATE: &str = "sendWaTemplateMessage";

/// `resolveStepTypeMatches`: the step type and its legacy aliases.
pub fn resolve_step_type_matches(step_type: &str) -> Vec<&str> {
    let mut out = vec![step_type];
    if step_type == SEND_WA_TEMPLATE {
        out.push("WA_TM01");
    }
    out
}

/// The start nodes' steps of `nodes` (`[]` for a non-array).
fn start_steps(nodes: &Value) -> impl Iterator<Item = &Value> {
    nodes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node.is_object() && node["data"]["isStartNode"] == Value::Bool(true))
        .flat_map(|node| {
            node["data"]["details"]["steps"]
                .as_array()
                .into_iter()
                .flatten()
        })
}

fn step_type(step: &Value) -> &str {
    step["stepType"].as_str().unwrap_or_default()
}

/// `hasStartNode`: a start node of `nodes` has a step of `step_type`.
pub fn has_start_node(nodes: &Value, step_type_name: &str) -> bool {
    let matches = resolve_step_type_matches(step_type_name);
    start_steps(nodes).any(|step| matches.contains(&step_type(step)))
}

/// `filterFlowsByStartStepType`.
pub fn filter_flows_by_start_step_type<T: FlowNodeVersions>(
    flows: Vec<T>,
    step_type: &str,
) -> Vec<T> {
    flows
        .into_iter()
        .filter(|flow| {
            flow.version_nodes()
                .into_iter()
                .any(|nodes| has_start_node(nodes, step_type))
        })
        .collect()
}

/// `filterFlowsByTemplateIds`: flows whose start node sends one of the
/// WhatsApp templates `template_ids`.
pub fn filter_flows_by_template_ids<T: FlowNodeVersions>(
    flows: Vec<T>,
    template_ids: &[String],
) -> Vec<T> {
    if template_ids.is_empty() {
        return Vec::new();
    }
    let matches = resolve_step_type_matches(SEND_WA_TEMPLATE);
    flows
        .into_iter()
        .filter(|flow| {
            flow.version_nodes().into_iter().any(|nodes| {
                start_steps(nodes).any(|step| {
                    matches.contains(&step_type(step))
                        && step["template"]["id"]
                            .as_str()
                            .is_some_and(|id| template_ids.iter().any(|t| t == id))
                })
            })
        })
        .collect()
}
