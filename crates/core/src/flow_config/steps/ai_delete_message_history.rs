//! Port of `packages/flow-config/src/steps/ai-delete-message-history.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_delete_message_history_provider = z::enum_(&["openai", "gemini", "claude", "deepseek", "openrouter", "muse", "openaiCompatible"]));

schema!(pub fn ai_delete_message_history_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiDeleteMessageHistory")).describe("Step type discriminator: \"aiDeleteMessageHistory\".")),
("provider", ai_delete_message_history_provider().catch_value(/* () => value */ json!("openai"))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]));

// HAND-PORT aiDeleteMessageHistoryDefaultFn:
// (props={})=>({id:createId(),provider:props.provider??"openai",...props,stepType:stepTypes.enum.aiDeleteMessageHistory,states:[successStateDefaultFn(),errorStateDefaultFn()]})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn ai_delete_message_history_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
