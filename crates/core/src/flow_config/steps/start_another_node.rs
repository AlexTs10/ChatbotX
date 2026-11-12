//! Port of `packages/flow-config/src/steps/start-another-node.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn start_another_node_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("startAnotherNode")).describe("Step type discriminator: \"startAnotherNode\".")),
("nodeId", z::string().regex(r#"^\d+$"#, "")),
("viewOnly", z::boolean().optional())]));

// HAND-PORT startAnotherNodeStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.startAnotherNode,nodeId:"",...props})

// ---- stage 1 API (hand-ported in stage 2) ----

/// `startAnotherNodeStepDefaultFn(props)`.
pub fn start_another_node_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "startAnotherNode", "nodeId": ""}),
        props,
    )
}
