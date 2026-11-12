//! Port of `packages/flow-config/src/steps/update-messenger-contact-data.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn update_messenger_contact_data_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("updateMessengerContactData")).describe("Step type discriminator: \"updateMessengerContactData\"."))]));

// HAND-PORT updateMessengerContactDataStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.updateMessengerContactData,...props})
