//! Port of `packages/flow-config/src/nodes/condition.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn condition_node_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Node id (numeric string) that `edges[].source`/`target` refer to. Use a unique numeric string per node in the graph.")),
("position", z::object(vec![("x", z::number().describe("Horizontal canvas position in pixels.")),
("y", z::number().describe("Vertical canvas position in pixels."))]).describe("Top-left position of the node on the builder canvas.")),
("measured", z::object(vec![("width", z::number().describe("Rendered node width in pixels.")),
("height", z::number().describe("Rendered node height in pixels."))]).describe("Rendered node size on the canvas, in pixels. Layout only; the builder default is 288x100.")),
("type", z::literal(json!("condition")).describe("Node type \"condition\": branches the flow on contact conditions. `data.details.steps` holds exactly one condition step; its case ids and `otherwiseId` are the edge source handles.")),
("data", z::object(vec![("name", z::string().trim().min(1).max(255).describe("Node display name shown on the canvas (1-255 characters).")),
("isStartNode", z::boolean().describe("True for the node where the flow begins. Exactly one node should be the start node.")),
("details", z::object(vec![("steps", z::array(condition_step_schema()).min(1).max(1))]))]))]));

// HAND-PORT conditionNodeDefaultFn:
// props=>({...defaultNodeData(),type:nodeTypeSchema.enum.condition,...props.nodeProps,data:{name:"Condition",isStartNode:false,...props.dataProps,details:{steps:[conditionStepDefaultFn()],...props.detailProps}}})
