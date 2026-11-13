//! Port of `apps/builder/__tests__/contact-filter-query-schema.test.ts`
//! (P3.6-D2 slice contact filter schema). Ported: `listContactsRequest.parse`
//! of the `contactFilter` search param becomes `parse_lenient_param` (the
//! other request keys are parsed by the API handler).

use chatbotx_business::contact_filter::parse_lenient_param;
use serde_json::{Value, json};

fn parse(criteria: &Value) -> Option<Value> {
    parse_lenient_param(Some(&Value::String(criteria.to_string()))).expect("parse")
}

#[test]
fn list_contacts_request_contact_filter_search_param_parses_a_json_contact_filter_query_value() {
    let criteria = json!({
        "operator": "and",
        "conditions": [{"field": "inbox", "operator": "eq", "value": ["123"]}],
    });
    assert_eq!(parse(&criteria), Some(criteria));
}

#[test]
fn list_contacts_request_contact_filter_search_param_falls_back_when_contact_filter_query_value_is_invalid()
 {
    let parsed = parse_lenient_param(Some(&json!("{invalid"))).expect("parse");
    assert_eq!(parsed, None);
}

#[test]
fn list_contacts_request_contact_filter_search_param_accepts_valueless_operators_without_a_value() {
    let parsed = parse(&json!({
        "operator": "and",
        "conditions": [{"field": "fullName", "operator": "isNotEmpty"}],
    }))
    .expect("filter");
    assert_eq!(
        parsed["conditions"][0],
        json!({"field": "fullName", "operator": "isNotEmpty"})
    );
}

#[test]
fn list_contacts_request_contact_filter_search_param_accepts_locale_and_timezone_select_values() {
    let parsed = parse(&json!({
        "operator": "and",
        "conditions": [
            {"field": "locale", "operator": "eq", "value": ["vi_VN"]},
            {"field": "timezone", "operator": "eq", "value": ["Asia/Ho_Chi_Minh"]},
        ],
    }))
    .expect("filter");
    assert_eq!(parsed["conditions"].as_array().map(Vec::len), Some(2));
}

#[test]
fn list_contacts_request_contact_filter_search_param_rejects_disabled_operators_for_a_field() {
    let parsed = parse(&json!({
        "operator": "and",
        "conditions": [{"field": "blocked", "operator": "contains", "value": "true"}],
    }));
    assert_eq!(parsed, None);
}
