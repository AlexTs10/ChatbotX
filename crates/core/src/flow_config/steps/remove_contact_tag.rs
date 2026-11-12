//! Port of `packages/flow-config/src/steps/remove-contact-tag.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn remove_contact_tag_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("removeContactTag")).describe("Step type discriminator: \"removeContactTag\".")),
("tags", z::array(z::string().trim().min(1)).min(1))]));

default_fn!(pub fn remove_contact_tag_step_default_fn = json!({"id": create_id(), "stepType": "removeContactTag", "tags": []}));
