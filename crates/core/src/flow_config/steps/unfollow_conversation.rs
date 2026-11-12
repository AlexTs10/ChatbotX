//! Port of `packages/flow-config/src/steps/unfollow-conversation.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn unfollow_conversation_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("unfollowConversation")).describe("Step type discriminator: \"unfollowConversation\"."))]));

default_fn!(pub fn unfollow_conversation_step_default_fn = json!({"id": create_id(), "stepType": "unfollowConversation"}));
