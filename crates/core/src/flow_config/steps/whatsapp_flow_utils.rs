//! Port of `packages/flow-config/src/steps/whatsapp-flow-utils.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

fn is_whatsapp_flow_step(step: &Value) -> bool {
    step.is_object()
        && step["stepType"] == "whatsappFlow"
        && step["buttons"]
            .as_array()
            .is_some_and(|b| b.iter().all(|b| b["id"].is_string()))
        && step["flow"].is_object()
        && step["flow"]["sourceId"].is_string()
        && step["flow"]["fieldMappings"].is_array()
}

fn flow_step_by_button_id<'a>(step: &'a Value, button_id: &str) -> Option<&'a Value> {
    (is_whatsapp_flow_step(step)
        && arr_field(step, "buttons")
            .iter()
            .any(|b| b["id"] == button_id))
    .then_some(step)
}

/// `findWhatsappFlowStepByButtonId(nodes, buttonId)`.
pub fn find_whatsapp_flow_step_by_button_id<'a>(
    nodes: &'a [Value],
    button_id: &str,
) -> Option<&'a Value> {
    for node in nodes {
        let details = &node["data"]["details"];
        if let Some(found) = details["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|s| flow_step_by_button_id(s, button_id))
        {
            return Some(found);
        }
        if let Some(before) = details.get("beforeStep")
            && let Some(found) = flow_step_by_button_id(before, button_id)
        {
            return Some(found);
        }
    }
    None
}
