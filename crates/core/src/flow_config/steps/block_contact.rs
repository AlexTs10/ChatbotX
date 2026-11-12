//! Port of `packages/flow-config/src/steps/block-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn block_contact_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("blockContact")).describe("Step type discriminator: \"blockContact\"."))]));

// HAND-PORT blockContactStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.blockContact,...props})
