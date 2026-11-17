//! Port of `packages/business/__tests__/mini-app-definition.test.ts`.

use std::collections::BTreeMap;

use chatbotx_business::BusinessError;
use chatbotx_business::mini_app::{
    DefinitionInput, build_new_definition, build_updated_definition,
};
use chatbotx_core::mini_app::{
    apply_custom_field_mappings, collect_custom_field_mappings, from_flow_json, parse_definition,
    starter_definition, to_flow_json,
};
use serde_json::{Value, json};

fn flow_json() -> Value {
    json!({ "version": "7.3", "screens": [{ "id": "WELCOME", "title": "Welcome", "terminal": true, "layout": {
        "type": "SingleColumnLayout", "children": [
            { "type": "TextInput", "name": "full_name", "label": "Full name" },
            { "type": "TextInput", "name": "email", "label": "Email" },
            { "type": "Footer", "label": "Done", "on-click-action": { "name": "complete", "payload": {} } }
        ] } }] })
}

fn mappings(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned)))
        .collect()
}

fn mapped(entries: &[(&str, &str)]) -> Value {
    let mut definition = parse_definition(&from_flow_json(&flow_json()).unwrap()).unwrap();
    let entries: Vec<_> = entries.iter().map(|(k, v)| (*k, Some(*v))).collect();
    apply_custom_field_mappings(&mut definition, &mappings(&entries));
    definition.to_value()
}

fn collected(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn input(
    definition: Option<Value>,
    flow: Option<Value>,
    custom: Option<BTreeMap<String, Option<String>>>,
) -> DefinitionInput {
    DefinitionInput {
        definition,
        flow_json: flow,
        custom_field_mappings: custom,
    }
}

#[test]
fn build_new_definition_imports_flow_json_and_applies_the_given_mappings() {
    let definition = build_new_definition(&input(
        None,
        Some(flow_json()),
        Some(mappings(&[("email", Some("9"))])),
    ))
    .unwrap();
    assert_eq!(to_flow_json(&definition)["screens"][0]["id"], "WELCOME");
    assert_eq!(
        collect_custom_field_mappings(&definition),
        collected(&[("email", "9")])
    );
}

#[test]
fn build_new_definition_stores_an_editor_definition_as_is() {
    let starter = starter_definition().to_value();
    let definition = build_new_definition(&input(Some(starter.clone()), None, None)).unwrap();
    assert_eq!(definition.to_value(), starter);
}

#[test]
fn build_new_definition_rejects_a_write_without_screens() {
    match build_new_definition(&input(None, None, None)) {
        Err(BusinessError::Exception { field, .. }) => {
            assert_eq!(field.as_deref(), Some("definition"))
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn build_updated_definition_changes_nothing_when_no_screens_or_mappings_are_given() {
    assert!(
        build_updated_definition(&mapped(&[]), &input(None, None, None))
            .unwrap()
            .is_none()
    );
}

#[test]
fn build_updated_definition_a_new_flow_json_keeps_existing_mappings_by_input_name() {
    let definition = build_updated_definition(
        &mapped(&[("full_name", "77")]),
        &input(
            None,
            Some(flow_json()),
            Some(mappings(&[("email", Some("9"))])),
        ),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        collect_custom_field_mappings(&definition),
        collected(&[("email", "9"), ("full_name", "77")])
    );
}

#[test]
fn build_updated_definition_mappings_alone_change_the_current_screens_null_removes_one() {
    let definition = build_updated_definition(
        &mapped(&[("full_name", "77"), ("email", "9")]),
        &input(None, None, Some(mappings(&[("full_name", None)]))),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        collect_custom_field_mappings(&definition),
        collected(&[("email", "9")])
    );
}

#[test]
fn build_updated_definition_an_editor_definition_replaces_the_screens_with_its_own_mappings() {
    let definition = build_updated_definition(
        &mapped(&[("full_name", "77")]),
        &input(Some(mapped(&[("email", "5")])), None, None),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        collect_custom_field_mappings(&definition),
        collected(&[("email", "5")])
    );
}
