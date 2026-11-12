//! Port of `packages/flow-config/src/steps/get-response-add-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn get_response_add_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("getResponseAddContact")).describe("Step type discriminator: \"getResponseAddContact\".")),
("campaignId", z::string().trim().min(1)),
("emailField", z::string().trim().min(1)),
("tags", z::array(z::string().trim().min(1)).optional()),
("dayOfCycle", z::string().trim().transform_opt(get_response_add_contact_schema_day_of_cycle_transform).optional()),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(get_response_add_contact_schema_super_refine));

default_fn!(pub fn get_response_add_contact_default_fn = json!({"id": create_id(), "stepType": "getResponseAddContact", "campaignId": "", "emailField": "email", "tags": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

fn get_response_add_contact_schema_day_of_cycle_transform(
    value: Option<Value>,
    _ctx: &mut RefineCtx,
) -> Option<Value> {
    value.filter(|v| truthy(Some(v)))
}

fn get_response_add_contact_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    report_duplicates(ctx, arr_field(value, "tags"), key_of, |i, k| {
        (
            path!["tags", i],
            format!("Duplicate GetResponse tag: \"{k}\""),
        )
    });
}
