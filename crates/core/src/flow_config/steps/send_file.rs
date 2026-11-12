//! Port of `packages/flow-config/src/steps/send-file.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_file_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendFile")).describe("Step type discriminator: \"sendFile\".")),
("mode", upload_modes()),
("url", z::string().refine(send_file_step_schema_url_refine, Some("Invalid URL"), None)),
("buttons", z::array(button_step_schema()))]));

default_fn!(pub fn send_file_step_default_fn = json!({"id": create_id(), "stepType": "sendFile", "mode": "file", "url": "", "buttons": []}));

// value=>containsVariablePlaceholder(value)||z.url().safeParse(value).success
fn send_file_step_schema_url_refine(value: &Value) -> bool {
    todo!("P3.4")
}
