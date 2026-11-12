//! Port of `packages/flow-config/src/steps/send-video.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_video_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendVideo")).describe("Step type discriminator: \"sendVideo\".")),
("mode", upload_modes()),
("url", z::string().refine(send_video_step_schema_url_refine, Some("Invalid URL"), None)),
("buttons", z::array(button_step_schema()))]));

default_fn!(pub fn send_video_step_default_fn = json!({"id": create_id(), "stepType": "sendVideo", "mode": "file", "url": "", "buttons": []}));

// value=>containsVariablePlaceholder(value)||z.url().safeParse(value).success
fn send_video_step_schema_url_refine(value: &Value) -> bool {
    todo!("P3.4")
}
