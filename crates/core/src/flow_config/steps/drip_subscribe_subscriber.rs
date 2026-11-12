//! Port of `packages/flow-config/src/steps/drip-subscribe-subscriber.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn drip_merge_field_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("dripField", z::string().trim().min(1))]));

schema!(pub fn drip_subscribe_subscriber_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("dripSubscribeSubscriber")).describe("Step type discriminator: \"dripSubscribeSubscriber\".")),
("accountId", z::string().trim().min(1)),
("emailField", z::string().trim().min(1)),
("phoneField", z::string().trim().min(1).optional()),
("tags", z::array(z::string().trim().min(1))),
("mergeFields", z::array(drip_merge_field_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(drip_subscribe_subscriber_schema_super_refine));

default_fn!(pub fn drip_subscribe_subscriber_default_fn = json!({"id": create_id(), "stepType": "dripSubscribeSubscriber", "accountId": "", "emailField": "email", "tags": [], "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// (data,ctx)=>{const seen=new Set;for(let i=0;i<data.mergeFields.length;i++){const{dripField}=data.mergeFields[i];if(seen.has(dripField)){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate Drip field mapping: "${dripField}"`,path:["mergeFields",i,"dripField"]})}seen.add(dripField)}const tagsSeen=new Set;for(let i=0;i<data.tags.length;i++){if(tagsSeen.has(data.tags[i])){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate tag: "${data.tags[i]}"`,path:["tags",i]})}tagsSeen.add(data.tags[i])}}
fn drip_subscribe_subscriber_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
