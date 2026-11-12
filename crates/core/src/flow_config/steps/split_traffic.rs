//! Port of `packages/flow-config/src/steps/split-traffic.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn split_traffic_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("splitTraffic")).describe("Step type discriminator: \"splitTraffic\".")),
("cases", z::array(z::object(vec![("value", z::number().int().gte(0 as f64).lte(100 as f64).describe("Percentage (0-100) of contacts sent down this branch.")),
("nodeId", z::string().regex(r#"^\d+$"#, "").nullable().optional().describe("Not used for routing; leave null. Branches are wired with edges whose `sourceHandle` is `<nodeId>-case-<index>` (the id of the containing node, not the step), where index is the 0-based position in `cases`."))])).refine(split_traffic_step_schema_cases_refine, Some("The total sum must equal 100%."), Some(&["cases"])).describe("Branches of the split. The `value` percentages must sum to exactly 100."))]));

default_fn!(pub fn split_traffic_step_default_fn = json!({"id": create_id(), "stepType": "splitTraffic", "cases": [{"value": 50, "nodeId": null}, {"value": 50, "nodeId": null}]}));

// data=>{const total=data.reduce((acc,curr)=>acc+curr.value,0);return total===100}
fn split_traffic_step_schema_cases_refine(value: &Value) -> bool {
    todo!("P3.4")
}
