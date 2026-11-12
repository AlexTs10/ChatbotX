//! Port of `packages/flow-config/src/steps/base.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn base_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional."))]));
