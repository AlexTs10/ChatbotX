//! Port of `packages/flow-config/src/steps/add-contact-notes.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn add_contact_notes_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("addContactNotes")).describe("Step type discriminator: \"addContactNotes\".")),
("content", z::string().trim().max(1000))]));

// HAND-PORT addContactNotesStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.addContactNotes,content:"",...props})
