//! Port of `packages/flow-config/src/steps/set-messenger-user-persistent-menu.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn set_messenger_user_persistent_menu_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("setMessengerUserPersistentMenu")).describe("Step type discriminator: \"setMessengerUserPersistentMenu\".")),
("userPersistentMenuId", z::string().optional())]));

// HAND-PORT setMessengerUserPersistentMenuStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.setMessengerUserPersistentMenu,userPersistentMenuId:"",...props})
