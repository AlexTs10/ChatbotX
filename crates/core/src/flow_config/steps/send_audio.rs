//! Port of `packages/flow-config/src/steps/send-audio.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_audio_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendAudio")).describe("Step type discriminator: \"sendAudio\".")),
("mode", upload_modes()),
("url", z::string().refine(send_audio_step_schema_url_refine, Some("Invalid URL"), None)),
("buttons", z::array(button_step_schema()))]));

fn send_audio_step_schema_url_refine(value: &Value) -> bool {
    deps::url_with_variables_ok(value)
}

/// `sendAudioStepDefaultFn(props)`.
pub fn send_audio_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({ "id": create_id(), "mode": "file", "url": "", "buttons": [] }),
        props,
        json!({ "stepType": "sendAudio" }),
    )
}
