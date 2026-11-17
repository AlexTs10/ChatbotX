//! Port of `packages/mini-app/__tests__/serialize.test.ts`.

mod mini_app_fixtures;

use chatbotx_core::mini_app::{
    from_flow_json, locate_issues, parse_definition, to_flow_json, to_nested_expression, validate,
};
use mini_app_fixtures::{children, two_screen_definition, two_screen_flow_json};
use serde_json::{Value, json};

fn flow(definition: &Value) -> Value {
    to_flow_json(&parse_definition(definition).expect("definition"))
}

fn with_if(text: &str) -> Value {
    let mut d = two_screen_definition();
    children(&mut d, 0).insert(1, json!({ "id": "n_if", "type": "If",
        "props": { "condition": "${form.full_name} == 'x'" },
        "slots": { "then": [{ "id": "n_t", "type": "TextBody", "props": { "text": text } }], "else": [] } }));
    d
}

#[test]
fn to_flow_json_produces_meta_flow_json_with_generated_payloads_and_no_editor_ids() {
    assert_eq!(flow(&two_screen_definition()), two_screen_flow_json());
}

#[test]
fn to_flow_json_serializes_containers_into_metas_slot_properties() {
    let json = flow(&with_if("yes"));
    assert_eq!(
        json["screens"][0]["layout"]["children"][1],
        json!({ "type": "If", "condition": "${form.full_name} == 'x'", "then": [{ "type": "TextBody", "text": "yes" }] })
    );
}

#[test]
fn to_flow_json_only_puts_inputs_from_screens_on_the_path_into_a_complete_payload() {
    let mut d = two_screen_definition();
    d["screens"].as_array_mut().unwrap().push(json!({ "key": "s_orphan", "id": "ORPHAN", "title": "",
        "children": [{ "id": "n_o", "type": "TextInput", "props": { "name": "orphan", "label": "Orphan" } }] }));
    let footer = &flow(&d)["screens"][1]["layout"]["children"][2];
    assert_eq!(
        footer["on-click-action"]["payload"],
        json!({ "full_name": "${screen.WELCOME.form.full_name}", "terms": "${form.terms}" })
    );
}

#[test]
fn from_flow_json_round_trips_flow_json_produced_by_to_flow_json() {
    let imported = from_flow_json(&two_screen_flow_json()).expect("import");
    assert_eq!(flow(&imported), two_screen_flow_json());
}

#[test]
fn from_flow_json_rejects_endpoint_powered_flows_and_unsupported_actions() {
    let mut endpoint = two_screen_flow_json();
    endpoint["data_api_version"] = json!("3.0");
    assert_eq!(from_flow_json(&endpoint).unwrap_err().code, "unsupported_endpoint");
    let mut exchange = two_screen_flow_json();
    exchange["screens"][0]["layout"]["children"][2]["on-click-action"] = json!({ "name": "data_exchange", "payload": {} });
    assert_eq!(from_flow_json(&exchange).unwrap_err().code, "unsupported_action");
}

#[test]
fn from_flow_json_rejects_navigation_to_unknown_screens() {
    let mut broken = two_screen_flow_json();
    broken["screens"][0]["layout"]["children"][2]["on-click-action"] =
        json!({ "name": "navigate", "next": { "type": "screen", "name": "NOPE" } });
    let error = from_flow_json(&broken).unwrap_err();
    assert_eq!(error.code, "unknown_screen");
    assert_eq!(error.path, "screens[0].layout.children[2].on-click-action.next.name");
}

#[test]
fn locate_issues_maps_editor_ids_to_flow_json_paths() {
    let mut definition = from_flow_json(&two_screen_flow_json()).expect("import");
    definition["screens"][0]["children"][1]["props"]["label"] = json!("This label is far too long for Meta");
    let issues = locate_issues(&definition, &validate(&definition).issues);
    let issue = &issues[0];
    assert_eq!(issue["code"], "property_too_long");
    assert_eq!(issue["screenId"], "WELCOME");
    assert_eq!(issue["path"], "screens[0].layout.children[1]");
    assert_eq!(issue["property"], "label");
}

#[test]
fn to_nested_expression_wraps_text_mixed_with_references_escaping_quotes() {
    assert_eq!(
        to_nested_expression("Hi ${form.name}, it's ${form.day}!"),
        "`'Hi ' ${form.name} ', it\\'s ' ${form.day} '!'`"
    );
}

#[test]
fn to_nested_expression_leaves_plain_text_whole_references_and_nested_expressions_alone() {
    assert_eq!(to_nested_expression("Plain"), "Plain");
    assert_eq!(to_nested_expression("${form.name}"), "${form.name}");
    assert_eq!(to_nested_expression("`'a' ${form.b}`"), "`'a' ${form.b}`");
}

#[test]
fn to_nested_expression_keeps_if_conditions_as_written() {
    let json = flow(&with_if("Hi ${form.full_name}"));
    let if_json = &json["screens"][0]["layout"]["children"][1];
    assert_eq!(if_json["condition"], "${form.full_name} == 'x'");
    assert_eq!(if_json["then"][0]["text"], "`'Hi ' ${form.full_name}`");
}

#[test]
fn text_input_date_types_sends_a_date_input_to_whats_app_as_a_pattern_checked_text_input() {
    let mut d = two_screen_definition();
    d["screens"][0]["children"][1]["props"]["input-type"] = json!("datetime");
    let component = &flow(&d)["screens"][0]["layout"]["children"][1];
    let pattern = component["pattern"].as_str().unwrap();
    assert_eq!(component["type"], "TextInput");
    assert_eq!(component["input-type"], "text");
    assert_eq!(component["helper-text"], "YYYY-MM-DD HH:mm");
    let re = regex::Regex::new(pattern).unwrap();
    assert!(re.is_match("2026-10-14 14:30"));
    assert!(re.is_match("2026-10-14T14:30"));
    assert!(!re.is_match("14/10/2026"));
}
