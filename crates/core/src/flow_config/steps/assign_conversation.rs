//! Port of `packages/flow-config/src/steps/assign-conversation.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn assign_conversation_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("assignConversation")).describe("Step type discriminator: \"assignConversation\".")),
("assignedId", z::string())]));

// HAND-PORT assignConversationStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.assignConversation,assignedId:"",...props})
