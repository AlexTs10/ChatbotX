//! Port of `packages/flow-config/src/steps/start-external-node.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn start_external_node_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("startExternalNode")).describe("Step type discriminator: \"startExternalNode\".")),
("flowId", z::string().regex(r#"^\d+$"#, "")),
("nodeId", z::string().regex(r#"^\d+$"#, ""))]));

// HAND-PORT startExternalNodeStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.startExternalNode,flowId:"",nodeId:"",...props})

schema!(pub fn start_external_step_action_schema = z::object(vec![("flowId", z::string().regex(r#"^\d+$"#, "")),
("nodeId", z::string().regex(r#"^\d+$"#, "")),
("type", z::literal(json!("startExternalStep")))]));

/// `startExternalNodeStepDefaultFn(props)`.
pub fn start_external_node_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "startExternalNode", "flowId": "", "nodeId": ""}),
        props,
    )
}
