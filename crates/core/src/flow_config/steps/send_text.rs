//! Port of `packages/flow-config/src/steps/send-text.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_text_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendText")).describe("Step type discriminator: \"sendText\" (sends a text message).")),
("text", z::string().trim().min(1).refine(send_text_step_schema_text_refine, Some("sendTextTooLongForChannel"), None).describe("Message text. May include `{{variable}}` placeholders; the maximum length depends on the target channel.")),
("buttons", z::array(button_step_schema()).describe("Buttons attached to the message (use [] for none). Per-channel button limits apply."))]));

// HAND-PORT sendTextStepDefaultFn:
// (props={})=>({text:"",buttons:[],...props,id:createId(),stepType:stepTypes.enum.sendText})

// value=>countMessageCharacters(value)<=SEND_TEXT_MAX
fn send_text_step_schema_text_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_text_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
