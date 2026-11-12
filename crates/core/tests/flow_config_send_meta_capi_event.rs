//! Port of `packages/flow-config/__tests__/send-meta-capi-event.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use chatbotx_core::utils::meta_capi::{
    META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES, META_CAPI_CONTENT_TYPE_VALUES,
    META_PIXEL_STANDARD_EVENT_NAMES, MetaCapiActionSource,
};

fn with(patch: Value) -> Value {
    spread(&fc::send_meta_capi_event_default_fn(), patch)
}

fn parse(patch: Value) -> Result<Value, z::ZodError> {
    fc::send_meta_capi_event_schema().parse(&with(patch))
}

#[test]
fn send_meta_capi_event_flow_contract_accepts_the_default_leadsubmitted_step() {
    assert_match_object(
        &fc::send_meta_capi_event_schema()
            .parse(&fc::send_meta_capi_event_default_fn())
            .unwrap(),
        &json!({ "stepType": "sendMetaCapiEvent", "eventName": "LeadSubmitted", "actionSource": "business_messaging" }),
    );
}

#[test]
fn send_meta_capi_event_flow_contract_rejects_the_wrong_step_type() {
    assert!(parse(json!({ "stepType": "sendText" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_accepts_purchase_for_the_default_business_messaging_action_source_with_required_value_currency()
 {
    let parsed =
        parse(json!({ "eventName": "Purchase", "value": "10", "currency": "USD" })).unwrap();

    assert_eq!(parsed["eventName"], "Purchase");
}

#[test]
fn send_meta_capi_event_flow_contract_accepts_numeric_value_text() {
    assert_eq!(parse(json!({ "value": "10.5" })).unwrap()["value"], "10.5");
}

#[test]
fn send_meta_capi_event_flow_contract_rejects_non_numeric_value_text() {
    assert!(parse(json!({ "value": "abc" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_rejects_invalid_currency_codes() {
    assert!(parse(json!({ "currency": "US" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_normalizes_static_currency_codes_to_uppercase() {
    assert_eq!(
        parse(json!({ "currency": "usd" })).unwrap()["currency"],
        "USD"
    );
}

#[test]
fn send_meta_capi_event_flow_contract_accepts_optional_content_fields_and_clears_blanks() {
    let parsed = parse(json!({ "contentCategory": "Education", "contentName": "" })).unwrap();

    assert_eq!(parsed["contentCategory"], "Education");
    assert!(parsed.get("contentName").is_none());
}

#[test]
fn send_meta_capi_event_flow_contract_rejects_content_fields_over_200_characters() {
    assert!(parse(json!({ "contentName": rep("x", 201) })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_treats_cleared_value_and_currency_fields_as_unset() {
    let parsed = parse(json!({ "value": "", "currency": "   " })).unwrap();

    assert!(parsed.get("value").is_none());
    assert!(parsed.get("currency").is_none());
}

#[test]
fn send_meta_capi_event_flow_contract_creates_safe_defaults_with_success_and_error_states() {
    let value = fc::send_meta_capi_event_default_fn();

    assert_match_object(
        &value,
        &json!({ "stepType": "sendMetaCapiEvent", "eventName": "LeadSubmitted", "actionSource": "business_messaging" }),
    );
    let states = value["states"].as_array().unwrap();
    let types: Vec<&Value> = states.iter().map(|s| &s["stateType"]).collect();
    assert_eq!(types, vec!["success", "error"]);
    assert!(states.iter().all(|s| truthy(&s["id"])));
}

#[test]
fn send_meta_capi_event_flow_contract_is_registered_in_the_step_enum_and_shared_action_union() {
    let defaults = fc::send_meta_capi_event_default_fn();

    assert!(
        fc::step_types()
            .enum_values()
            .contains(&"sendMetaCapiEvent".to_owned())
    );
    assert!(any_ok(&fc::action_steps(), &defaults));
}

#[test]
fn send_meta_capi_event_flow_contract_business_messaging_event_catalog_accepts_standard_business_messaging_event_s()
 {
    for event_name in META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES {
        let mut patch = json!({ "actionSource": "business_messaging", "eventName": event_name });
        if *event_name == "Purchase" {
            patch = spread(&patch, json!({ "value": "10", "currency": "USD" }));
        }
        assert_eq!(parse(patch).unwrap()["eventName"], *event_name);
    }
}

#[test]
fn send_meta_capi_event_flow_contract_business_messaging_event_catalog_rejects_a_pixel_only_event_name_lead_for_business_messaging()
 {
    assert!(parse(json!({ "actionSource": "business_messaging", "eventName": "Lead" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_business_messaging_event_catalog_rejects_a_custom_event_name_for_business_messaging()
 {
    assert!(
        parse(json!({ "actionSource": "business_messaging", "eventName": "my-event" })).is_err()
    );
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_accepts_standard_pixel_event_s_for_email()
 {
    for event_name in META_PIXEL_STANDARD_EVENT_NAMES {
        let mut patch = json!({ "actionSource": "email", "eventName": event_name });
        if *event_name == "Purchase" {
            patch = spread(&patch, json!({ "value": "10", "currency": "USD" }));
        }
        assert_eq!(parse(patch).unwrap()["eventName"], *event_name);
    }
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_rejects_a_business_messaging_only_event_name_leadsubmitted_for_email()
 {
    assert!(parse(json!({ "actionSource": "email", "eventName": "LeadSubmitted" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_accepts_custom_event_name_s_for_email()
 {
    for event_name in [
        "Order.Completed".to_owned(),
        "my-event".to_owned(),
        "1x".to_owned(),
        rep("x", 50),
    ] {
        assert_eq!(
            parse(json!({ "actionSource": "email", "eventName": event_name })).unwrap()["eventName"],
            event_name.as_str()
        );
    }
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_rejects_a_custom_event_name_over_50_characters_for_email()
 {
    assert!(parse(json!({ "actionSource": "email", "eventName": rep("x", 51) })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_rejects_an_empty_custom_event_name_for_email()
 {
    assert!(parse(json!({ "actionSource": "email", "eventName": "" })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_pixel_event_catalog_non_messaging_action_sources_rejects_a_whitespace_only_custom_event_name_for_email()
 {
    assert!(parse(json!({ "actionSource": "email", "eventName": "   " })).is_err());
}

#[test]
fn send_meta_capi_event_flow_contract_action_source_accepts_action_source_s() {
    for action_source in MetaCapiActionSource::ALL {
        let parsed = parse(json!({
            "actionSource": action_source.as_str(),
            "eventName": "Purchase",
            "value": "10",
            "currency": "USD",
        }))
        .unwrap();

        assert_eq!(parsed["actionSource"], action_source.as_str());
    }
}

#[test]
fn send_meta_capi_event_flow_contract_action_source_rejects_excluded_action_source_s() {
    for action_source in ["website", "app"] {
        assert!(
            parse(json!({ "actionSource": action_source })).is_err(),
            "{action_source}"
        );
    }
}

#[test]
fn send_meta_capi_event_flow_contract_content_type_accepts_content_type_s() {
    for content_type in META_CAPI_CONTENT_TYPE_VALUES {
        assert_eq!(
            parse(json!({ "contentType": content_type })).unwrap()["contentType"],
            *content_type
        );
    }
}

#[test]
fn send_meta_capi_event_flow_contract_content_type_rejects_an_invalid_content_type() {
    assert!(parse(json!({ "contentType": "digital_good" })).is_err());
}

fn paths(patch: Value) -> Vec<String> {
    let result = fc::send_meta_capi_event_schema().safe_parse(&with(patch));
    assert!(!result.success);
    result.paths()
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_rejects_purchase_missing_both_value_and_currency()
 {
    let paths = paths(json!({ "eventName": "Purchase" }));
    assert!(paths.contains(&"value".to_owned()));
    assert!(paths.contains(&"currency".to_owned()));
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_rejects_purchase_missing_only_value()
 {
    assert!(
        paths(json!({ "eventName": "Purchase", "currency": "USD" })).contains(&"value".to_owned())
    );
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_rejects_purchase_missing_only_currency()
 {
    assert!(
        paths(json!({ "eventName": "Purchase", "value": "10" })).contains(&"currency".to_owned())
    );
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_accepts_purchase_with_both_value_and_currency()
 {
    let parsed =
        parse(json!({ "eventName": "Purchase", "value": "10", "currency": "USD" })).unwrap();

    assert_eq!(parsed["value"], "10");
    assert_eq!(parsed["currency"], "USD");
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_accepts_s_without_value_or_currency()
 {
    for event_name in ["AddToCart", "ViewContent", "LeadSubmitted"] {
        let parsed = parse(json!({ "eventName": event_name })).unwrap();

        assert!(parsed.get("value").is_none(), "{event_name}");
        assert!(parsed.get("currency").is_none(), "{event_name}");
    }
}

#[test]
fn send_meta_capi_event_flow_contract_purchase_value_currency_requirement_accepts_a_custom_event_name_without_value_or_currency()
 {
    let parsed = parse(json!({ "actionSource": "email", "eventName": "my-event" })).unwrap();

    assert!(parsed.get("value").is_none());
    assert!(parsed.get("currency").is_none());
}

#[test]
fn send_meta_capi_event_flow_contract_template_values_accepts_a_variable_value_verbatim() {
    let parsed =
        parse(json!({ "eventName": "Purchase", "value": "{{order_total}}", "currency": "USD" }))
            .unwrap();

    assert_eq!(parsed["value"], "{{order_total}}");
}

#[test]
fn send_meta_capi_event_flow_contract_template_values_does_not_uppercase_a_variable_currency() {
    let parsed =
        parse(json!({ "eventName": "Purchase", "value": "10", "currency": "{{currency}}" }))
            .unwrap();

    assert_eq!(parsed["currency"], "{{currency}}");
}

#[test]
fn send_meta_capi_event_flow_contract_template_values_accepts_template_contentids_verbatim() {
    assert_eq!(
        parse(json!({ "contentIds": "{{a}},{{b}}" })).unwrap()["contentIds"],
        "{{a}},{{b}}"
    );
}

#[test]
fn send_meta_capi_event_flow_contract_template_values_accepts_a_static_contentids_value_verbatim() {
    assert_eq!(
        parse(json!({ "contentIds": "SKU-1, SKU-2" })).unwrap()["contentIds"],
        "SKU-1, SKU-2"
    );
}

#[test]
fn send_meta_capi_event_flow_contract_template_values_treats_a_whitespace_only_contentids_value_as_unset()
 {
    assert!(
        parse(json!({ "contentIds": "   " }))
            .unwrap()
            .get("contentIds")
            .is_none()
    );
}

#[test]
fn send_meta_capi_event_flow_contract_parses_a_stored_step_from_before_actionsource_existed_with_defaults_applied()
 {
    let legacy_stored_step = json!({
        "id": "123456789012345678",
        "stepType": "sendMetaCapiEvent",
        "eventName": "LeadSubmitted",
        "value": "10",
        "currency": "USD",
        "contentCategory": "Education",
        "contentName": "Course",
        "states": fc::send_meta_capi_event_default_fn()["states"],
    });

    let parsed = fc::send_meta_capi_event_schema()
        .parse(&legacy_stored_step)
        .unwrap();

    assert_eq!(parsed["actionSource"], "business_messaging");
    assert_eq!(parsed["eventName"], "LeadSubmitted");
    assert_eq!(parsed["value"], "10");
    assert_eq!(parsed["currency"], "USD");
    assert_eq!(parsed["contentCategory"], "Education");
    assert_eq!(parsed["contentName"], "Course");
}
