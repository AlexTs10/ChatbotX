//! Port of `packages/flow-config/src/steps/send-audio.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_audio_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendAudio")).describe("Step type discriminator: \"sendAudio\".")),
("mode", upload_modes()),
("url", z::string().refine(send_audio_step_schema_url_refine, Some("Invalid URL"), None)),
("buttons", z::array(button_step_schema()))]));

// HAND-PORT sendAudioStepDefaultFn:
// props=>({id:createId(),mode:uploadModes.enum.file,url:"",buttons:[],...props,stepType:stepTypes.enum.sendAudio})

// value=>containsVariablePlaceholder(value)||z.url().safeParse(value).success
fn send_audio_step_schema_url_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_audio_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
