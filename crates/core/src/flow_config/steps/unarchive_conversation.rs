//! Port of `packages/flow-config/src/steps/unarchive-conversation.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn unarchive_conversation_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("unarchiveConversation")).describe("Step type discriminator: \"unarchiveConversation\"."))]));

default_fn!(pub fn unarchive_conversation_step_default_fn = json!({"id": create_id(), "stepType": "unarchiveConversation"}));
