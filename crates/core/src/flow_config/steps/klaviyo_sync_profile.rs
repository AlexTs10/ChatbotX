//! Port of `packages/flow-config/src/steps/klaviyo-sync-profile.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn klaviyo_property_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("klaviyoProperty", z::string().trim().min(1))]));

schema!(pub fn klaviyo_sync_profile_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("klaviyoSyncProfile")).describe("Step type discriminator: \"klaviyoSyncProfile\".")),
("listId", z::string().trim().transform(klaviyo_sync_profile_schema_list_id_transform).optional()),
("emailField", z::string().trim().min(1)),
("titleField", z::string().trim().transform(klaviyo_sync_profile_schema_list_id_transform).optional()),
("orgField", z::string().trim().transform(klaviyo_sync_profile_schema_list_id_transform).optional()),
("mergeFields", z::array(klaviyo_property_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(klaviyo_sync_profile_schema_super_refine));

default_fn!(pub fn klaviyo_sync_profile_default_fn = json!({"id": create_id(), "stepType": "klaviyoSyncProfile", "emailField": "email", "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// value=>value||void 0
fn klaviyo_sync_profile_schema_list_id_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// (value,ctx)=>{const properties=new Set;value.mergeFields.forEach((mapping,index)=>{if(properties.has(mapping.klaviyoProperty)){ctx.addIssue({code:"custom",path:["mergeFields",index,"klaviyoProperty"],message:"Klaviyo property mappings must be unique"})}properties.add(mapping.klaviyoProperty)})}
fn klaviyo_sync_profile_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
