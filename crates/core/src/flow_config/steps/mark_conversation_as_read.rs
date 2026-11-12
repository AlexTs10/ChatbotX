//! Port of `packages/flow-config/src/steps/mark-conversation-as-read.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn mark_conversation_as_read_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("markConversationAsRead")).describe("Step type discriminator: \"markConversationAsRead\"."))]));

default_fn!(pub fn mark_conversation_as_read_step_default_fn = json!({"id": create_id(), "stepType": "markConversationAsRead"}));
