//! Port of `packages/flow-config/src/steps/make.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn make_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("make")).describe("Step type discriminator: \"make\".")),
("events", z::array(z::string().trim().min(1).max(100)).min(1).max(20)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

default_fn!(pub fn make_step_default_fn = json!({"id": create_id(), "stepType": "make", "events": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));
