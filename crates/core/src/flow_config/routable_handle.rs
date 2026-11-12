//! Port of `packages/flow-config/src/routable-handle.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn routable_handle_kinds() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"stepButtons":"stepButtons","cardButtons":"cardButtons","listOptions":"listOptions","pageElements":"pageElements","quickReplySettings":"quickReplySettings"}"#).expect("valid JSON")
    });
    &V
}

/// `FlowRouteUpdate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRouteUpdate {
    pub source_node_id: String,
    pub handle_id: String,
    /// `route.targetNodeId`, or `None` to clear the route.
    pub route: Option<String>,
}

/// `FoundRoutableButton`.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundRoutableButton {
    pub button: Value,
    pub runtime_node_id: String,
}

fn node_steps(node: &Value) -> Option<&Vec<Value>> {
    node["data"]["details"].get("steps")?.as_array()
}

fn runtime_node_id(step: &Value, node: &Value) -> String {
    step["nodeId"]
        .as_str()
        .or_else(|| node["id"].as_str())
        .unwrap_or("")
        .to_owned()
}

fn find_in_step_buttons(node: &Value, handle_id: &str) -> Option<(Value, String, usize)> {
    for (index, step) in node_steps(node)?.iter().enumerate() {
        let Some(buttons) = step.get("buttons").and_then(Value::as_array) else {
            continue;
        };
        if let Some(button) = buttons.iter().find(|b| b["id"] == handle_id) {
            return Some((button.clone(), runtime_node_id(step, node), index));
        }
    }
    None
}

fn find_in_card_buttons(node: &Value, handle_id: &str) -> Option<(Value, String, usize)> {
    for (index, step) in node_steps(node)?.iter().enumerate() {
        let Some(cards) = step.get("cards").and_then(Value::as_array) else {
            continue;
        };
        for card in cards {
            if let Some(button) = arr_field(card, "buttons")
                .iter()
                .find(|b| b["id"] == handle_id)
            {
                return Some((button.clone(), runtime_node_id(step, node), index));
            }
        }
    }
    None
}

fn find_in_list_options(node: &Value, handle_id: &str) -> Option<(Value, String, usize)> {
    for (index, step) in node_steps(node)?.iter().enumerate() {
        let Some(options) = step.get("options").and_then(Value::as_array) else {
            continue;
        };
        if let Some(option) = options.iter().find(|o| o["id"] == handle_id) {
            let button = json!({ "id": option["id"], "label": option["title"], "buttonType": null, "beforeStep": null, "steps": [] });
            return Some((button, runtime_node_id(step, node), index));
        }
    }
    None
}

/// `findButtonInNodes(nodes, handleId)`: the earliest step's button with
/// that id in the first node that has one.
pub fn find_button_in_nodes(nodes: &[Value], handle_id: &str) -> Option<FoundRoutableButton> {
    for node in nodes {
        let first = [
            find_in_step_buttons,
            find_in_card_buttons,
            find_in_list_options,
        ]
        .into_iter()
        .filter_map(|find| find(node, handle_id))
        .min_by_key(|(_, _, index)| *index);
        if let Some((button, runtime_node_id, _)) = first {
            return Some(FoundRoutableButton {
                button,
                runtime_node_id,
            });
        }
    }
    None
}

fn route_fields(route: Option<&str>) -> Value {
    match route {
        Some(target) => json!({
            "buttonType": "startAnotherNode",
            "beforeStep": start_another_node_step_default_fn(&json!({ "nodeId": target, "viewOnly": true })),
        }),
        None => json!({ "buttonType": null, "beforeStep": null }),
    }
}

fn update_button_route(button: &Value, route: Option<&str>) -> Value {
    if button["buttonType"] == "openMiniApp" {
        return button.clone();
    }
    merge(button.clone(), route_fields(route))
}

fn replace_first(
    items: &[Value],
    mut replace: impl FnMut(&Value) -> Option<Value>,
) -> Option<(Vec<Value>, usize)> {
    for (index, item) in items.iter().enumerate() {
        if let Some(replacement) = replace(item) {
            let mut out = items.to_vec();
            out[index] = replacement;
            return Some((out, index));
        }
    }
    None
}

fn update_node_steps(
    node: &Value,
    update: impl FnMut(&Value) -> Option<Value>,
) -> Option<(Value, usize)> {
    let steps = node_steps(node)?;
    let (steps, index) = replace_first(steps, update)?;
    let mut out = node.clone();
    out["data"]["details"]["steps"] = Value::Array(steps);
    Some((out, index))
}

fn apply_step_buttons(
    node: &Value,
    handle_id: &str,
    route: Option<&str>,
) -> Option<(Value, usize)> {
    update_node_steps(node, |step| {
        let buttons = step.get("buttons")?.as_array()?;
        let (buttons, _) = replace_first(buttons, |b| {
            (b["id"] == handle_id).then(|| update_button_route(b, route))
        })?;
        let mut out = step.clone();
        out["buttons"] = Value::Array(buttons);
        Some(out)
    })
}

fn apply_card_buttons(
    node: &Value,
    handle_id: &str,
    route: Option<&str>,
) -> Option<(Value, usize)> {
    update_node_steps(node, |step| {
        let cards = step.get("cards")?.as_array()?;
        let (cards, _) = replace_first(cards, |card| {
            let (buttons, _) = replace_first(arr_field(card, "buttons"), |b| {
                (b["id"] == handle_id).then(|| update_button_route(b, route))
            })?;
            let mut out = card.clone();
            out["buttons"] = Value::Array(buttons);
            Some(out)
        })?;
        let mut out = step.clone();
        out["cards"] = Value::Array(cards);
        Some(out)
    })
}

fn apply_page_elements(
    node: &Value,
    handle_id: &str,
    route: Option<&str>,
) -> Option<(Value, usize)> {
    update_node_steps(node, |step| {
        let elements = step.get("elements")?.as_array()?;
        let (elements, _) = replace_first(elements, |e| {
            (e["type"] == "button" && e["id"] == handle_id).then(|| update_button_route(e, route))
        })?;
        let mut out = step.clone();
        out["elements"] = Value::Array(elements);
        Some(out)
    })
}

fn apply_quick_reply_settings(
    node: &Value,
    handle_id: &str,
    route: Option<&str>,
) -> Option<(Value, usize)> {
    let settings = node["data"]["details"]
        .get("quickReplySettings")
        .filter(|s| truthy(Some(s)))?;
    for key in ["followUp", "retry"] {
        if settings[key]["id"] != handle_id {
            continue;
        }
        let target = match route {
            Some(t) => json!({
                "buttonType": "startAnotherNode",
                "beforeStep": start_another_node_step_default_fn(&json!({ "nodeId": t, "viewOnly": true })),
            }),
            None => Value::Null,
        };
        let mut out = node.clone();
        out["data"]["details"]["quickReplySettings"][key]["target"] = target;
        return Some((out, usize::MAX));
    }
    None
}

/// `applyRouteInNode(node, handleId, route)`: the node with the earliest
/// matching handle rerouted, or `None` when no handle matched.
pub fn apply_route_in_node(
    source_node: &Value,
    handle_id: &str,
    route: Option<&str>,
) -> Option<Value> {
    [
        apply_step_buttons,
        apply_card_buttons,
        apply_page_elements,
        apply_quick_reply_settings,
    ]
    .into_iter()
    .filter_map(|apply| apply(source_node, handle_id, route))
    .min_by_key(|(_, index)| *index)
    .map(|(node, _)| node)
}

/// `applyRouteUpdatesInNodes(nodes, updates)`.
pub fn apply_route_updates_in_nodes(nodes: &[Value], updates: &[FlowRouteUpdate]) -> Vec<Value> {
    nodes
        .iter()
        .map(|node| {
            let id = str_field(node, "id");
            let mut updated = node.clone();
            for update in updates.iter().filter(|u| u.source_node_id == id) {
                if let Some(next) =
                    apply_route_in_node(&updated, &update.handle_id, update.route.as_deref())
                {
                    updated = next;
                }
            }
            updated
        })
        .collect()
}
