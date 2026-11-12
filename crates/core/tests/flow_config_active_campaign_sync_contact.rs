//! Port of `packages/flow-config/__tests__/active-campaign-sync-contact.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn activecampaignsynccontactschema_defaults_to_email_only_sync_and_is_registered_in_action_steps() {
    let value = fc::active_campaign_sync_contact_default_fn();

    assert_match_object(
        &value,
        &json!({
            "operation": "createOrUpdateContact",
            "emailField": "email",
            "listIds": [],
            "tagIds": [],
            "fieldValues": [],
        }),
    );
    assert!(value.get("phoneField").is_none());
    assert!(value.get("automationId").is_none());
    assert!(
        fc::active_campaign_sync_contact_schema()
            .safe_parse(&value)
            .success
    );
    assert!(any_ok(&fc::action_steps(), &value));
}

#[test]
fn activecampaignsynccontactschema_rejects_duplicate_list_and_tag_ids_after_normalization() {
    let value = spread(
        &fc::active_campaign_sync_contact_default_fn(),
        json!({ "listIds": ["1", " 1 "], "tagIds": ["2", " 2 "] }),
    );

    assert!(
        !fc::active_campaign_sync_contact_schema()
            .safe_parse(&value)
            .success
    );
}

#[test]
fn activecampaignsynccontactschema_rejects_duplicate_activecampaign_field_mappings() {
    let value = spread(
        &fc::active_campaign_sync_contact_default_fn(),
        json!({
            "fieldValues": [
                { "contactFieldId": "field-1", "activeCampaignFieldId": "9" },
                { "contactFieldId": "field-2", "activeCampaignFieldId": "9" },
            ],
        }),
    );

    assert!(
        !fc::active_campaign_sync_contact_schema()
            .safe_parse(&value)
            .success
    );
}

#[test]
fn activecampaignsynccontactschema_requires_an_automation_for_add_contact_to_automation() {
    let value = without(
        &spread(
            &fc::active_campaign_sync_contact_default_fn(),
            json!({ "operation": "addContactToAutomation" }),
        ),
        &["automationId"],
    );

    assert!(
        !fc::active_campaign_sync_contact_schema()
            .safe_parse(&value)
            .success
    );
    assert!(
        fc::active_campaign_sync_contact_schema()
            .safe_parse(&spread(&value, json!({ "automationId": "42" })))
            .success
    );
}
