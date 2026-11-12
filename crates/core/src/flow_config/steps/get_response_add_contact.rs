//! Port of `packages/flow-config/src/steps/get-response-add-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn get_response_add_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("getResponseAddContact")).describe("Step type discriminator: \"getResponseAddContact\".")),
("campaignId", z::string().trim().min(1)),
("emailField", z::string().trim().min(1)),
("tags", z::array(z::string().trim().min(1)).optional()),
("dayOfCycle", z::string().trim().transform(get_response_add_contact_schema_day_of_cycle_transform).optional()),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(get_response_add_contact_schema_super_refine));

default_fn!(pub fn get_response_add_contact_default_fn = json!({"id": create_id(), "stepType": "getResponseAddContact", "campaignId": "", "emailField": "email", "tags": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// value=>value||void 0
fn get_response_add_contact_schema_day_of_cycle_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{const tagIds=new Set;value.tags?.forEach((tagId,index)=>{if(tagIds.has(tagId)){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate GetResponse tag: "${tagId}"`,path:["tags",index]})}tagIds.add(tagId)})}
fn get_response_add_contact_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
