//! Port of `packages/flow-config/src/steps/active-campaign-sync-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn active_campaign_field_value_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("activeCampaignFieldId", z::string().trim().min(1))]));

schema!(pub fn active_campaign_operation_types = z::enum_(&["createOrUpdateContact", "addContactToAutomation"]));

schema!(pub fn active_campaign_sync_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("activeCampaignSyncContact")).describe("Step type discriminator: \"activeCampaignSyncContact\".")),
("operation", active_campaign_operation_types().default(json!("createOrUpdateContact"))),
("emailField", z::string().trim().min(1)),
("phoneField", z::string().trim().min(1).optional()),
("automationId", z::string().trim().min(1).optional()),
("listIds", z::array(z::string().trim().min(1))),
("tagIds", z::array(z::string().trim().min(1))),
("fieldValues", z::array(active_campaign_field_value_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(active_campaign_sync_contact_schema_super_refine));

default_fn!(pub fn active_campaign_sync_contact_default_fn = json!({"id": create_id(), "stepType": "activeCampaignSyncContact", "operation": "createOrUpdateContact", "emailField": "email", "listIds": [], "tagIds": [], "fieldValues": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// (data,ctx)=>{if(data.operation==="addContactToAutomation"&&!data.automationId?.trim()){ctx.addIssue({code:z.ZodIssueCode.custom,message:"ActiveCampaign automation is required",path:["automationId"]})}if(data.operation==="addContactToAutomation"){return}const listIds=new Set;for(let i=0;i<data.listIds.length;i++){if(listIds.has(data.listIds[i])){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate ActiveCampaign list: "${data.listIds[i]}"`,path:["listIds",i]})}listIds.add(data.listIds[i])}const tagIds=new Set;for(let i=0;i<data.tagIds.length;i++){if(tagIds.has(data.tagIds[i])){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate ActiveCampaign tag: "${data.tagIds[i]}"`,path:["tagIds",i]})}tagIds.add(data.tagIds[i])}const fieldIds=new Set;for(let i=0;i<data.fieldValues.length;i++){const fieldId=data.fieldValues[i].activeCampaignFieldId;if(fieldIds.has(fieldId)){ctx.addIssue({code:z.ZodIssueCode.custom,message:`Duplicate ActiveCampaign field mapping: "${fieldId}"`,path:["fieldValues",i,"activeCampaignFieldId"]})}fieldIds.add(fieldId)}}
fn active_campaign_sync_contact_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}
