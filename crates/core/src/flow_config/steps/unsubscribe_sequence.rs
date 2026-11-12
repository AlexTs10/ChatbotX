//! Port of `packages/flow-config/src/steps/unsubscribe-sequence.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn unsubscribe_sequence_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("unsubscribeSequence")).describe("Step type discriminator: \"unsubscribeSequence\".")),
("sequenceId", z::string().optional())]));

default_fn!(pub fn unsubscribe_sequence_step_default_fn = json!({"id": create_id(), "stepType": "unsubscribeSequence"}));
