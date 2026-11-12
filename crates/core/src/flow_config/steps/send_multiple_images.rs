//! Port of `packages/flow-config/src/steps/send-multiple-images.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_multiple_images_item_schema = z::object(vec![("id", z::string()),
("mode", upload_modes()),
("url", z::string().refine(send_multiple_images_item_schema_url_refine, Some("Invalid URL"), None))]));

default_fn!(pub fn send_multiple_images_item_default_fn = json!({"id": create_id(), "mode": "file", "url": ""}));

schema!(pub fn send_multiple_images_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendMultipleImages")).describe("Step type discriminator: \"sendMultipleImages\".")),
("images", z::array(send_multiple_images_item_schema()).min(2).max(10))]));

default_fn!(pub fn send_multiple_images_step_default_fn = json!({"id": create_id(), "stepType": "sendMultipleImages", "images": [{"id": create_id(), "mode": "file", "url": ""}, {"id": create_id(), "mode": "file", "url": ""}]}));

fn send_multiple_images_item_schema_url_refine(value: &Value) -> bool {
    deps::url_with_variables_ok(value)
}
