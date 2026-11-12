//! Port of `packages/flow-config/__tests__/mailer-lite-add-subscriber.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn mailerlite_add_subscriber_flow_contract_creates_safe_defaults_with_stable_state_ids() {
    let value = fc::mailer_lite_add_subscriber_default_fn();
    assert_match_object(
        &value,
        &json!({
            "stepType": "mailerLiteAddSubscriber",
            "emailField": "email",
            "status": "unconfirmed",
            "mergeFields": [],
        }),
    );
    let states = value["states"].as_array().unwrap();
    let types: Vec<&Value> = states.iter().map(|s| &s["stateType"]).collect();
    assert_eq!(types, vec!["success", "error"]);
    assert!(states.iter().all(|s| truthy(&s["id"])));
}

#[test]
fn mailerlite_add_subscriber_flow_contract_trims_optional_group_and_accepts_an_explicit_status() {
    let value = fc::mailer_lite_add_subscriber_schema()
        .parse(&spread(
            &fc::mailer_lite_add_subscriber_default_fn(),
            json!({ "groupId": " group-1 ", "status": "active" }),
        ))
        .unwrap();
    assert_eq!(value["groupId"], "group-1");
    assert_eq!(value["status"], "active");
}

#[test]
fn mailerlite_add_subscriber_flow_contract_rejects_duplicate_provider_targets() {
    let defaults = fc::mailer_lite_add_subscriber_default_fn();
    assert!(
        fc::mailer_lite_add_subscriber_schema()
            .parse(&spread(
                &defaults,
                json!({
                    "mergeFields": [
                        { "contactFieldId": "a", "mailerLiteField": "plan" },
                        { "contactFieldId": "b", "mailerLiteField": "plan" },
                    ],
                }),
            ))
            .is_err()
    );
}

#[test]
fn mailerlite_add_subscriber_flow_contract_accepts_mappings_to_mailerlite_default_fields() {
    let defaults = fc::mailer_lite_add_subscriber_default_fn();
    let merge_fields = json!([
        { "contactFieldId": "a", "mailerLiteField": "name" },
        { "contactFieldId": "b", "mailerLiteField": "last_name" },
        { "contactFieldId": "c", "mailerLiteField": "phone" },
    ]);
    let parsed = fc::mailer_lite_add_subscriber_schema()
        .parse(&spread(&defaults, json!({ "mergeFields": merge_fields })))
        .unwrap();
    assert_match_object(&parsed, &json!({ "mergeFields": merge_fields }));
}

#[test]
fn mailerlite_add_subscriber_flow_contract_is_registered_in_the_shared_action_union() {
    let defaults = fc::mailer_lite_add_subscriber_default_fn();
    assert!(any_ok(&fc::action_steps(), &defaults));
}
