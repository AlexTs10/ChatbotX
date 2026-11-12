//! Port of `packages/flow-config/src/steps/start-external-flow.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn start_external_flow_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("startExternalFlow")).describe("Step type discriminator: \"startExternalFlow\".")),
("flowId", z::string().regex(r#"^\d+$"#, ""))]));

// HAND-PORT startExternalFlowStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.startExternalFlow,flowId:"",...props})

// ---- stage 1 API (hand-ported in stage 2) ----

/// `startExternalFlowStepDefaultFn(props)`.
pub fn start_external_flow_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "startExternalFlow", "flowId": ""}),
        props,
    )
}
