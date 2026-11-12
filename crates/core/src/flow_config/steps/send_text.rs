//! Port of `packages/flow-config/src/steps/send-text.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_text_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendText")).describe("Step type discriminator: \"sendText\" (sends a text message).")),
("text", z::string().trim().min(1).refine(send_text_step_schema_text_refine, Some("sendTextTooLongForChannel"), None).describe("Message text. May include `{{variable}}` placeholders; the maximum length depends on the target channel.")),
("buttons", z::array(button_step_schema()).describe("Buttons attached to the message (use [] for none). Per-channel button limits apply."))]));

fn send_text_step_schema_text_refine(value: &Value) -> bool {
    count_message_characters(value.as_str()) <= SEND_TEXT_MAX
}

/// `sendTextStepDefaultFn(props)`.
pub fn send_text_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({ "text": "", "buttons": [] }),
        props,
        json!({ "id": create_id(), "stepType": "sendText" }),
    )
}
