//! Port of `packages/flow-config/src/steps/mark-conversation-as-unread.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn mark_conversation_as_unread_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("markConversationAsUnread")).describe("Step type discriminator: \"markConversationAsUnread\"."))]));

default_fn!(pub fn mark_conversation_as_unread_step_default_fn = json!({"id": create_id(), "stepType": "markConversationAsUnread"}));
