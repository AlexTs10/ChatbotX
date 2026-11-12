//! Port of `packages/flow-config/src/steps/set-messenger-persona.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn set_messenger_persona_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("setMessengerPersona")).describe("Step type discriminator: \"setMessengerPersona\".")),
("personaId", z::string().optional())]));

// HAND-PORT setMessengerPersonaStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.setMessengerPersona,personaId:"",...props})

/// `setMessengerPersonaStepDefaultFn(props)`.
pub fn set_messenger_persona_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "setMessengerPersona", "personaId": ""}),
        props,
    )
}
