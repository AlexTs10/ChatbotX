//! Port of `packages/flow-config/__tests__/external-request.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn external_request_flow_contract_creates_safe_defaults_with_stable_state_ids() {
    let value = fc::external_request_step_default_fn();
    assert_match_object(
        &value,
        &json!({ "stepType": "callApi", "method": "GET", "url": "", "headers": [] }),
    );
    assert!(value.get("body").is_none());
    let states = value["states"].as_array().unwrap();
    let types: Vec<&Value> = states.iter().map(|s| &s["stateType"]).collect();
    assert_eq!(types, vec!["success", "error"]);
    assert!(states.iter().all(|s| truthy(&s["id"])));
}

fn valid_base() -> Value {
    spread(
        &fc::external_request_step_default_fn(),
        json!({
            "url": "https://api.example.com/endpoint",
            "mapping": [{ "jsonPath": "data.id", "outputFieldId": "field-1" }],
        }),
    )
}

fn ok(value: &Value) -> bool {
    fc::external_request_step_schema().safe_parse(value).success
}

#[test]
fn external_request_flow_contract_get_without_a_body_is_valid() {
    assert!(ok(&valid_base()));
}

#[test]
fn external_request_flow_contract_post_without_a_body_is_invalid() {
    assert!(!ok(&spread(&valid_base(), json!({ "method": "POST" }))));
}

#[test]
fn external_request_flow_contract_post_with_an_empty_jsonbody_is_invalid() {
    assert!(!ok(&spread(
        &valid_base(),
        json!({ "method": "POST", "body": { "bodyType": "json", "jsonBody": "" } })
    )));
}

#[test]
fn external_request_flow_contract_post_with_a_valid_jsonbody_is_valid() {
    assert!(ok(&spread(
        &valid_base(),
        json!({ "method": "POST", "body": { "bodyType": "json", "jsonBody": "{\"foo\":\"bar\"}" } })
    )));
}

#[test]
fn external_request_flow_contract_get_with_a_body_is_invalid() {
    assert!(!ok(&spread(
        &valid_base(),
        json!({ "method": "GET", "body": { "bodyType": "allContactData" } })
    )));
}

#[test]
fn external_request_flow_contract_delete_with_a_body_is_invalid() {
    assert!(!ok(&spread(
        &valid_base(),
        json!({ "method": "DELETE", "body": { "bodyType": "json", "jsonBody": "{\"foo\":\"bar\"}" } })
    )));
}

#[test]
fn external_request_flow_contract_rejects_duplicate_header_keys_case_insensitively() {
    assert!(!ok(&spread(
        &valid_base(),
        json!({ "headers": [{ "key": "Authorization", "value": "a" }, { "key": "authorization", "value": "b" }] })
    )));
}

#[test]
fn external_request_flow_contract_rejects_duplicate_formencoded_field_keys() {
    assert!(!ok(&spread(
        &valid_base(),
        json!({
            "method": "POST",
            "body": {
                "bodyType": "formEncoded",
                "formFields": [{ "key": "name", "value": "a" }, { "key": "name", "value": "b" }],
            },
        })
    )));
}

#[test]
fn external_request_flow_contract_is_registered_in_the_shared_action_union() {
    assert!(any_ok(&fc::action_steps(), &valid_base()));
}
