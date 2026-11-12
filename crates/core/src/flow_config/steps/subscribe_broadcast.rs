//! Port of `packages/flow-config/src/steps/subscribe-broadcast.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn subscribe_broadcast_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("subscribeBroadcast")).describe("Step type discriminator: \"subscribeBroadcast\"."))]));

default_fn!(pub fn subscribe_broadcast_step_default_fn = json!({"id": create_id(), "stepType": "subscribeBroadcast"}));
