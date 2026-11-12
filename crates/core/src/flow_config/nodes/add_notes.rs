//! Port of `packages/flow-config/src/nodes/add-notes.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn add_notes_node_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Node id (numeric string) that `edges[].source`/`target` refer to. Use a unique numeric string per node in the graph.")),
("position", z::object(vec![("x", z::number().describe("Horizontal canvas position in pixels.")),
("y", z::number().describe("Vertical canvas position in pixels."))]).describe("Top-left position of the node on the builder canvas.")),
("measured", z::object(vec![("width", z::number().describe("Rendered node width in pixels.")),
("height", z::number().describe("Rendered node height in pixels."))]).describe("Rendered node size on the canvas, in pixels. Layout only; the builder default is 288x100.")),
("type", z::literal(json!("addNotes")).describe("Node type \"addNotes\": a canvas annotation node. `data.details.beforeStep` holds one addNotes step whose `text` (max 1000 chars) is the note. It is documentation for flow editors only and does not post a note to the conversation at runtime.")),
("data", z::object(vec![("name", z::string().trim().min(1).max(255).describe("Node display name shown on the canvas (1-255 characters).")),
("isStartNode", z::boolean().describe("True for the node where the flow begins. Exactly one node should be the start node.")),
("details", z::object(vec![("beforeStep", add_notes_step_schema())]))]))]));

// HAND-PORT addNotesNodeDefaultFn:
// props=>({...defaultNodeData(),type:nodeTypeSchema.enum.addNotes,...props.nodeProps,data:{name:"Add Notes",...props.dataProps,isStartNode:false,details:{beforeStep:addNotesStepDefaultFn(),...props.detailProps}}})
