//! Port of `packages/flow-config/src/steps/send-gif.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_gif_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendGif")).describe("Step type discriminator: \"sendGif\".")),
("url", z::string().refine(send_gif_step_schema_url_refine, Some("Invalid URL"), None))]));

default_fn!(pub fn send_gif_step_default_fn = json!({"id": create_id(), "stepType": "sendGif", "url": ""}));

fn send_gif_step_schema_url_refine(value: &Value) -> bool {
    deps::url_with_variables_ok(value)
}
