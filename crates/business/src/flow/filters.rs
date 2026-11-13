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

/// `resolveStepTypeMatches`: the step type and its legacy aliases.
pub fn resolve_step_type_matches(step_type: &str) -> Vec<&str> {
    let _ = step_type;
    todo!("P3.6-D2")
}

/// `hasStartNode`: a start node of `nodes` has a step of `step_type`.
pub fn has_start_node(nodes: &Value, step_type: &str) -> bool {
    let _ = (nodes, step_type);
    todo!("P3.6-D2")
}

/// `filterFlowsByStartStepType`.
pub fn filter_flows_by_start_step_type<T: FlowNodeVersions>(
    flows: Vec<T>,
    step_type: &str,
) -> Vec<T> {
    let _ = (flows, step_type);
    todo!("P3.6-D2")
}

/// `filterFlowsByTemplateIds`: flows whose start node sends one of the
/// WhatsApp templates `template_ids`.
pub fn filter_flows_by_template_ids<T: FlowNodeVersions>(
    flows: Vec<T>,
    template_ids: &[String],
) -> Vec<T> {
    let _ = (flows, template_ids);
    todo!("P3.6-D2")
}
