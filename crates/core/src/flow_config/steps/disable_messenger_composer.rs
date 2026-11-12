//! Port of `packages/flow-config/src/steps/disable-messenger-composer.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn disable_messenger_composer_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("disableMessengerComposer")).describe("Step type discriminator: \"disableMessengerComposer\"."))]));

// HAND-PORT disableMessengerComposerStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.disableMessengerComposer,...props})
