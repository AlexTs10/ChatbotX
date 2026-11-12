//! Port of `packages/flow-config/src/steps/add-contact-tag.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn add_contact_tag_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("addContactTag")).describe("Step type discriminator: \"addContactTag\".")),
("tags", z::array(z::string().trim().min(1)).min(1))]));

// HAND-PORT addContactTagStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.addContactTag,tags:[],...props})
