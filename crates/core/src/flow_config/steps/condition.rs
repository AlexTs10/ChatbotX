//! Port of `packages/flow-config/src/steps/condition.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn condition_filter_condition_schema = z::object(vec![("field", z::string().min(1).describe("Contact field to test: a static field name from `GET /v1/contacts/filter-fields`, or `customField` together with `customFieldId` (the custom field id) to test a workspace custom field.")),
("operator", z::string().min(1).describe("Operator valid for this field, as listed by `GET /v1/contacts/filter-fields`.")),
("value", z::union(vec![z::string(),
z::array(z::string()),
z::tuple(vec![z::string(),
z::string()])]).optional().describe("Comparison value. Omit for valueless operators (e.g. isEmpty). A two-element tuple is a between-range; an array is a multi-value match.")),
("customFieldId", z::string().regex(r#"^\d+$"#, "").optional()),
("topicId", z::string().regex(r#"^\d+$"#, "").optional()),
("leaderboardId", z::string().regex(r#"^\d+$"#, "").optional()),
("customFieldType", z::string().optional()),
("botFieldId", z::string().regex(r#"^\d+$"#, "").optional()),
("botFieldType", z::string().optional()),
("valueType", z::string().optional())]).refine(condition_filter_condition_schema_refine, Some("Pipeline item conditions are not supported in the Condition step, which would never match them."), Some(&["field"])));

schema!(pub fn condition_case_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Case id (numeric string); also the `sourceHandle` of the edge taken when this case matches.")),
("operator", z::enum_(&["and", "or"]).describe("Whether all (`and`) or any (`or`) of `conditions` must match. Cases are evaluated in order; the first match wins.")),
("conditions", z::array(condition_filter_condition_schema()).min(1).describe("Contact-filter conditions for this case (at least one).")),
("timezone", z::string().max(64).optional())]));

schema!(pub fn condition_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("condition")).describe("Step type discriminator: \"condition\".")),
("cases", z::array(condition_case_schema()).min(1).describe("Ordered cases; the first whose conditions match is taken.")),
("otherwiseId", z::string().regex(r#"^\d+$"#, "").describe("Id (numeric string) used as the `sourceHandle` of the edge taken when no case matches."))]));

default_fn!(pub fn condition_case_default_fn = json!({"id": create_id(), "operator": "and", "conditions": []}));

/// `conditionStepDefaultFn(props)`.
pub fn condition_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "condition", "cases": [{"id": create_id(), "operator": "and", "conditions": []}], "otherwiseId": create_id()}),
        props,
    )
}

fn condition_filter_condition_schema_refine(condition: &Value) -> bool {
    condition["field"] != "pipelineItem"
}
