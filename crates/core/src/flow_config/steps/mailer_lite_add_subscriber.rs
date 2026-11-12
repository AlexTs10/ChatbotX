//! Port of `packages/flow-config/src/steps/mailer-lite-add-subscriber.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn mailer_lite_field_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("mailerLiteField", z::string().trim().min(1))]));

schema!(pub fn mailer_lite_add_subscriber_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("mailerLiteAddSubscriber")).describe("Step type discriminator: \"mailerLiteAddSubscriber\".")),
("groupId", z::string().trim().transform(mailer_lite_add_subscriber_schema_group_id_transform).optional()),
("emailField", z::string().trim().min(1)),
("status", z::enum_(&["active", "unconfirmed"])),
("mergeFields", z::array(mailer_lite_field_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(mailer_lite_add_subscriber_schema_super_refine));

default_fn!(pub fn mailer_lite_add_subscriber_default_fn = json!({"id": create_id(), "stepType": "mailerLiteAddSubscriber", "emailField": "email", "status": "unconfirmed", "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// value=>value||void 0
fn mailer_lite_add_subscriber_schema_group_id_transform(
    value: Value,
    ctx: &mut RefineCtx,
) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{const targets=new Set;value.mergeFields.forEach((mapping,index)=>{if(targets.has(mapping.mailerLiteField)){ctx.addIssue({code:"custom",path:["mergeFields",index,"mailerLiteField"],message:"MailerLite field mappings must be unique"})}targets.add(mapping.mailerLiteField)})}
fn mailer_lite_add_subscriber_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
