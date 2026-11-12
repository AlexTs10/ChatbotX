//! Port of `packages/flow-config/src/steps/moosend-create-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn moosend_create_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("moosendCreateContact")).describe("Step type discriminator: \"moosendCreateContact\".")),
("listId", z::string().trim().min(1)),
("emailField", z::string().trim().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

default_fn!(pub fn moosend_create_contact_default_fn = json!({"id": create_id(), "stepType": "moosendCreateContact", "listId": "", "emailField": "email", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));
