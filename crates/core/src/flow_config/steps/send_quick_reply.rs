//! Port of `packages/flow-config/src/steps/send-quick-reply.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const MAX_QUICK_REPLIES: usize = 10;

schema!(pub fn send_quick_reply_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendQuickReply")).describe("Step type discriminator: \"sendQuickReply\".")),
("message", z::string().trim().min(1).max(1000)),
("buttons", z::array(button_step_schema()).super_refine(refine_reply_only_buttons))]));

// HAND-PORT sendQuickReplyStepDefaultFn:
// (props={})=>({message:"",buttons:[],...props,id:createId(),stepType:stepTypes.enum.sendQuickReply})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_quick_reply_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
