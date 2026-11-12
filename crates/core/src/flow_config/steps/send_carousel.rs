//! Port of `packages/flow-config/src/steps/send-carousel.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_carousel_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendCarousel")).describe("Step type discriminator: \"sendCarousel\".")),
("layout", card_layouts()),
("cards", z::array(send_card_step_schema()))]));

default_fn!(pub fn send_carousel_step_default_fn = json!({"id": create_id(), "stepType": "sendCarousel", "layout": "horizontal", "cards": [{"id": create_id(), "stepType": "sendCard", "title": "", "subtitle": "", "image": {"id": create_id(), "stepType": "sendImage", "mode": "file", "url": "", "buttons": []}, "buttons": []}]}));
