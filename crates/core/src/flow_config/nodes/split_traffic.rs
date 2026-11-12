//! Port of `packages/flow-config/src/nodes/split-traffic.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn split_traffic_node_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Node id (numeric string) that `edges[].source`/`target` refer to. Use a unique numeric string per node in the graph.")),
("position", z::object(vec![("x", z::number().describe("Horizontal canvas position in pixels.")),
("y", z::number().describe("Vertical canvas position in pixels."))]).describe("Top-left position of the node on the builder canvas.")),
("measured", z::object(vec![("width", z::number().describe("Rendered node width in pixels.")),
("height", z::number().describe("Rendered node height in pixels."))]).describe("Rendered node size on the canvas, in pixels. Layout only; the builder default is 288x100.")),
("type", z::literal(json!("splitTraffic")).describe("Node type \"splitTraffic\". `data.details.steps` holds splitTraffic steps whose `cases[].value` percentages sum to 100; each branch is an edge with `sourceHandle` `<nodeId>-case-<index>` (the id of this node, index = 0-based position in `cases`).")),
("data", z::object(vec![("name", z::string().trim().min(1).max(255).describe("Node display name shown on the canvas (1-255 characters).")),
("isStartNode", z::boolean().describe("True for the node where the flow begins. Exactly one node should be the start node.")),
("details", z::object(vec![("steps", z::array(split_traffic_step_schema()).min(1))]))]))]));

// HAND-PORT splitTrafficNodeDefaultFn:
// props=>({...defaultNodeData(),type:nodeTypeSchema.enum.splitTraffic,...props.nodeProps,data:{name:"Split Traffic",isStartNode:false,...props.dataProps,details:{steps:[splitTrafficStepDefaultFn()],...props.detailProps}}})
