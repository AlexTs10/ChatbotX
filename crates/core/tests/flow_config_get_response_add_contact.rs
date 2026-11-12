//! Port of `packages/flow-config/__tests__/get-response-add-contact.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn getresponseaddcontactschema_defaults_to_email_and_success_error_states_and_is_registered() {
    let value = fc::get_response_add_contact_default_fn();

    assert_match_object(
        &value,
        &json!({
            "stepType": "getResponseAddContact",
            "emailField": "email",
            "campaignId": "",
            "tags": [],
        }),
    );
    assert!(value.get("dayOfCycle").is_none());
    assert_eq!(value["states"].as_array().unwrap().len(), 2);
    let with_campaign = spread(&value, json!({ "campaignId": "campaign-1" }));
    assert!(
        fc::get_response_add_contact_schema()
            .safe_parse(&with_campaign)
            .success
    );
    assert!(any_ok(&fc::action_steps(), &with_campaign));
}

#[test]
fn getresponseaddcontactschema_trims_optional_day_of_cycle_to_undefined() {
    let result = fc::get_response_add_contact_schema()
        .parse(&spread(
            &fc::get_response_add_contact_default_fn(),
            json!({ "campaignId": "campaign-1", "dayOfCycle": " " }),
        ))
        .unwrap();

    assert!(result.get("dayOfCycle").is_none());
}

#[test]
fn getresponseaddcontactschema_rejects_duplicate_tag_ids_after_normalization() {
    let value = spread(
        &fc::get_response_add_contact_default_fn(),
        json!({ "campaignId": "campaign-1", "tags": ["tag-1", " tag-1 "] }),
    );

    assert!(
        !fc::get_response_add_contact_schema()
            .safe_parse(&value)
            .success
    );
}
