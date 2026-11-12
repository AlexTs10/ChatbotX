//! Port of `packages/flow-config/src/steps/typing.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn typing_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("typing")).describe("Step type discriminator: \"typing\".")),
("seconds", z::number().coerce().gte(1 as f64).lte(60 as f64))]));

// HAND-PORT typingStepDefaultFn:
// props=>({id:createId(),seconds:2,...props,stepType:stepTypes.enum.typing})
