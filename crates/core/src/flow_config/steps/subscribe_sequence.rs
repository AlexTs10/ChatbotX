//! Port of `packages/flow-config/src/steps/subscribe-sequence.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn subscribe_sequence_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("subscribeSequence")).describe("Step type discriminator: \"subscribeSequence\".")),
("sequenceId", z::string().optional())]));

default_fn!(pub fn subscribe_sequence_step_default_fn = json!({"id": create_id(), "stepType": "subscribeSequence"}));
