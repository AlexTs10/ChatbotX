//! Port of `packages/flow-config/src/steps/opt-in-email.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn opt_in_email_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("optInEmail")).describe("Step type discriminator: \"optInEmail\"."))]));

default_fn!(pub fn opt_in_email_step_default_fn = json!({"id": create_id(), "stepType": "optInEmail"}));
