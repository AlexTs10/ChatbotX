//! Port of `packages/flow-config/src/steps/archive-conversation.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn archive_conversation_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("archiveConversation")).describe("Step type discriminator: \"archiveConversation\"."))]));

// HAND-PORT archiveConversationStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.archiveConversation,...props})
