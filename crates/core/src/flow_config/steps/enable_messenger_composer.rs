//! Port of `packages/flow-config/src/steps/enable-messenger-composer.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn enable_messenger_composer_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("enableMessengerComposer")).describe("Step type discriminator: \"enableMessengerComposer\"."))]));

// HAND-PORT enableMessengerComposerStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.enableMessengerComposer,...props})
