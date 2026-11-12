//! Port of `packages/flow-config/src/nodes/base.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn node_type_schema = z::enum_(&["sendMessage", "startFlow", "performAction", "condition", "sendMail", "splitTraffic", "wait", "followUp", "landingPage", "addNotes"]));

schema!(pub fn base_node_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Node id (numeric string) that `edges[].source`/`target` refer to. Use a unique numeric string per node in the graph.")),
("position", z::object(vec![("x", z::number().describe("Horizontal canvas position in pixels.")),
("y", z::number().describe("Vertical canvas position in pixels."))]).describe("Top-left position of the node on the builder canvas.")),
("measured", z::object(vec![("width", z::number().describe("Rendered node width in pixels.")),
("height", z::number().describe("Rendered node height in pixels."))]).describe("Rendered node size on the canvas, in pixels. Layout only; the builder default is 288x100."))]));

schema!(pub fn base_node_data_schema = z::object(vec![("name", z::string().trim().min(1).max(255).describe("Node display name shown on the canvas (1-255 characters).")),
("isStartNode", z::boolean().describe("True for the node where the flow begins. Exactly one node should be the start node."))]));

pub fn default_node_measured() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"width":288,"height":100}"#).expect("valid JSON")
    });
    &V
}

/// `defaultNodeData()`: fresh id, default position and size.
pub fn default_node_data() -> Value {
    json!({ "id": create_id(), "position": { "x": 100, "y": 300 }, "measured": default_node_measured() })
}

/// The shared shape of the `*NodeDefaultFn(props)` factories.
/// `is_start_after` puts `isStartNode: false` after `dataProps` (addNotes).
pub(crate) fn node_default(
    props: &Value,
    node_type: &str,
    name: &str,
    details: Value,
    detail_props: bool,
    is_start_after: bool,
) -> Value {
    let node = spread3(
        default_node_data(),
        &json!({ "type": node_type }),
        Value::Null,
    );
    let node = spread_props(node, props.get("nodeProps").unwrap_or(&Value::Null));
    let mut data = json!({ "name": name, "isStartNode": false });
    data = spread_props(data, props.get("dataProps").unwrap_or(&Value::Null));
    if is_start_after {
        data["isStartNode"] = json!(false);
    }
    let details = if detail_props {
        spread_props(details, props.get("detailProps").unwrap_or(&Value::Null))
    } else {
        details
    };
    data["details"] = details;
    let mut node = node;
    node["data"] = data;
    node
}
