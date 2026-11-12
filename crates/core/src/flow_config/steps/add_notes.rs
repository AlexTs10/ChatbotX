//! Port of `packages/flow-config/src/steps/add-notes.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn add_notes_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("addNotes")).describe("Step type discriminator: \"addNotes\".")),
("text", z::string().trim().max(1000))]));

// HAND-PORT addNotesStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.addNotes,text:"",...props})

/// `addNotesStepDefaultFn(props)`.
pub fn add_notes_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "addNotes", "text": ""}),
        props,
    )
}
