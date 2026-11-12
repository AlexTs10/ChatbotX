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

fn active_campaign_sync_contact_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    if data["operation"] == "addContactToAutomation"
        && trimmed(&data["automationId"]).unwrap_or("").is_empty()
    {
        ctx.issue(
            path!["automationId"],
            "ActiveCampaign automation is required",
        );
    }
    if data["operation"] == "addContactToAutomation" {
        return;
    }
    report_duplicates(ctx, arr_field(data, "listIds"), key_of, |i, k| {
        (
            path!["listIds", i],
            format!("Duplicate ActiveCampaign list: \"{k}\""),
        )
    });
    report_duplicates(ctx, arr_field(data, "tagIds"), key_of, |i, k| {
        (
            path!["tagIds", i],
            format!("Duplicate ActiveCampaign tag: \"{k}\""),
        )
    });
    report_duplicates(
        ctx,
        arr_field(data, "fieldValues"),
        |v| key_of(&v["activeCampaignFieldId"]),
        |i, k| {
            (
                path!["fieldValues", i, "activeCampaignFieldId"],
                format!("Duplicate ActiveCampaign field mapping: \"{k}\""),
            )
        },
    );
}
