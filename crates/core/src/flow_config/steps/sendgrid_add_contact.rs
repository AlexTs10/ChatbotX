//! Port of `packages/flow-config/src/steps/sendgrid-add-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_grid_merge_field_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("sendGridField", z::string().trim().min(1))]));

schema!(pub fn send_grid_add_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("sendGridAddContact")).describe("Step type discriminator: \"sendGridAddContact\".")),
("listId", z::string().transform(send_grid_add_contact_schema_list_id_transform).optional()),
("emailField", z::string().trim().min(1)),
("phoneField", z::string().transform(send_grid_add_contact_schema_list_id_transform).optional()),
("mergeFields", z::array(send_grid_merge_field_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(send_grid_add_contact_schema_super_refine));

default_fn!(pub fn send_grid_add_contact_default_fn = json!({"id": create_id(), "stepType": "sendGridAddContact", "emailField": "email", "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// value=>value.trim()||void 0
fn send_grid_add_contact_schema_list_id_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (data,ctx)=>{const seen=new Set;for(let index=0;index<data.mergeFields.length;index++){const target=data.mergeFields[index].sendGridField;if(seen.has(target)){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate SendGrid field mapping: "${target}"`,path:["mergeFields",index,"sendGridField"]})}seen.add(target)}}
fn send_grid_add_contact_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
