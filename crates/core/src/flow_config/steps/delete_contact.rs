//! Port of `packages/flow-config/src/steps/delete-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn delete_contact_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("deleteContact")).describe("Step type discriminator: \"deleteContact\"."))]));

// HAND-PORT deleteContactStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.deleteContact,...props})
