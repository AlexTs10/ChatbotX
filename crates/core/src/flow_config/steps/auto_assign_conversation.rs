//! Port of `packages/flow-config/src/steps/auto-assign-conversation.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn auto_assign_conversation_rule() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"ALL_TIME":"ALL_TIME","LAST_HOUR":"LAST_HOUR","LAST_8HOURS":"LAST_8HOURS","LAST_24HOURS":"LAST_24HOURS"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn auto_assign_conversation_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("autoAssignConversation")).describe("Step type discriminator: \"autoAssignConversation\".")),
("assignedIds", z::array(z::string())),
("rule", z::enum_(&["ALL_TIME", "LAST_HOUR", "LAST_8HOURS", "LAST_24HOURS"])),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

// HAND-PORT autoAssignConversationStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.autoAssignConversation,assignedIds:[],rule:AutoAssignConversationRule.ALL_TIME,states:[successStateDefaultFn(),errorStateDefaultFn()],...props})

/// `autoAssignConversationStepDefaultFn(props)`.
pub fn auto_assign_conversation_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "autoAssignConversation", "assignedIds": [], "rule": "ALL_TIME", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}),
        props,
    )
}
