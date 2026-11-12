//! Port of `packages/flow-config/src/steps/enable-bot.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn enable_bot_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("enableBot")).describe("Step type discriminator: \"enableBot\"."))]));

default_fn!(pub fn enable_bot_step_default_fn = json!({"id": create_id(), "stepType": "enableBot"}));
