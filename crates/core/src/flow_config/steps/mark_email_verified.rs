//! Port of `packages/flow-config/src/steps/mark-email-verified.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn mark_email_verified_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("markEmailVerified")).describe("Step type discriminator: \"markEmailVerified\"."))]));

default_fn!(pub fn mark_email_verified_step_default_fn = json!({"id": create_id(), "stepType": "markEmailVerified"}));
