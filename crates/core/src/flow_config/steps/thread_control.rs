//! Port of `packages/flow-config/src/steps/thread-control.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn thread_control_step_actions = z::enum_(&["release", "pass"]));

schema!(pub fn thread_control_step_target_roles = z::enum_(&["ai_agent", "ctwa", "customer_service", "escalation", "marketing", "utility"]));

pub const THREAD_CONTROL_STEP_METADATA_MAX_LENGTH: usize = 2000;

schema!(pub fn thread_control_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("threadControl")).describe("Step type discriminator: \"threadControl\".")),
("action", thread_control_step_actions()),
("targetRole", thread_control_step_target_roles().optional().describe("Only for `pass`, WhatsApp only (Messenger refuses any role but `ai_agent`): the role to hand the conversation to. Omit it, or use `escalation`, for the escalation partner.")),
("metadata", z::string().max(2000).optional().describe("Only for `pass`: a note delivered as-is to the receiving app. Supports variables like {{first_name}}.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

// HAND-PORT threadControlStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.threadControl,action:threadControlStepActions.enum.release,states:[successStateDefaultFn(),errorStateDefaultFn()],...props})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn thread_control_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
