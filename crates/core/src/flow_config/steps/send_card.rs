//! Port of `packages/flow-config/src/steps/send-card.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_card_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendCard")).describe("Step type discriminator: \"sendCard\".")),
("title", z::string().trim().min(1).refine(send_card_step_schema_title_refine, Some("Card titles must be 80 characters or fewer."), None)),
("subtitle", z::string().trim().refine(send_card_step_schema_subtitle_refine, Some("Card subtitles must be 80 characters or fewer."), None).optional()),
("image", z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendImage")).describe("Step type discriminator: \"sendImage\".")),
("mode", upload_modes()),
("url", z::union(vec![z::string().refine(send_card_step_schema_image_url_0_refine, Some("Invalid URL"), None),
z::literal(json!(""))])),
("buttons", z::array(button_step_schema()))]).optional()),
("buttons", z::array(button_step_schema()).max(3))]));

default_fn!(pub fn send_card_step_default_fn = json!({"id": create_id(), "stepType": "sendCard", "title": "", "subtitle": "", "image": {"id": create_id(), "stepType": "sendImage", "mode": "file", "url": "", "buttons": []}, "buttons": []}));

// value=>countMessageCharacters(value)<=80
fn send_card_step_schema_title_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>countMessageCharacters(value)<=80
fn send_card_step_schema_subtitle_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>containsVariablePlaceholder(value)||z.url().safeParse(value).success
fn send_card_step_schema_image_url_0_refine(value: &Value) -> bool {
    todo!("P3.4")
}
