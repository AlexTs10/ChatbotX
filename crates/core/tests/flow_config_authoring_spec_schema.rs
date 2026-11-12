//! Port of `packages/flow-config/__tests__/authoring/spec-schema.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn expect_invalid(value: &Value) -> Vec<Issue> {
    let result = fc::flow_spec_schema().safe_parse(value);
    assert!(!result.success, "Expected flow spec validation to fail");
    result.error.expect("error").issues
}

fn create_flow_spec(steps: Value) -> Value {
    json!({ "formatVersion": 1, "name": "Customer follow-up", "steps": steps })
}

fn create_three_level_nested_flow_spec(deep_step: Value) -> Value {
    create_flow_spec(json!([{
        "type": "branch",
        "cases": [{
            "when": [{ "field": "email", "operator": "equals", "value": "customer@example.com" }],
            "then": [{
                "type": "send",
                "text": "Would you like to continue?",
                "buttons": [{
                    "text": "Continue",
                    "then": [{
                        "type": "branch",
                        "cases": [{
                            "when": [{ "field": "firstName", "operator": "isNotEmpty" }],
                            "then": [deep_step],
                        }],
                    }],
                }],
            }],
        }],
    }]))
}

fn children<'a>(schema: &'a Value, key: &str) -> Vec<&'a Value> {
    match schema.get(key) {
        Some(Value::Object(map)) => map.values().collect(),
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

fn visit_json_schema(schema: &Value) {
    if let Some(Value::Object(properties)) = schema.get("properties") {
        for (property_name, property_schema) in properties {
            let description = property_schema.get("description");
            assert!(
                description.is_some_and(Value::is_string),
                "Property \"{property_name}\" is missing a description"
            );
            if let Some(Value::String(description)) = description {
                assert!(!description.trim().is_empty());
            }
            visit_json_schema(property_schema);
        }
    }
    if let Some(items) = schema.get("items").filter(|v| v.is_object()) {
        visit_json_schema(items);
    }
    for key in ["anyOf", "$defs", "allOf", "oneOf"] {
        for option in children(schema, key) {
            visit_json_schema(option);
        }
    }
}

fn find_json_schema_node<'a>(schema: &'a Value, description: &str) -> Option<&'a Value> {
    if schema.get("description").and_then(Value::as_str) == Some(description) {
        return Some(schema);
    }
    for property in children(schema, "properties") {
        if let Some(found) = find_json_schema_node(property, description) {
            return Some(found);
        }
    }
    if let Some(items) = schema.get("items")
        && let Some(found) = find_json_schema_node(items, description)
    {
        return Some(found);
    }
    for key in ["anyOf", "$defs", "allOf", "oneOf"] {
        for option in children(schema, key) {
            if let Some(found) = find_json_schema_node(option, description) {
                return Some(found);
            }
        }
    }
    None
}

fn has_json_schema_max_length(schema: Option<&Value>, max_length: u64) -> bool {
    let Some(schema) = schema else {
        return false;
    };
    schema.get("maxLength").and_then(Value::as_u64) == Some(max_length)
        || ["allOf", "anyOf", "oneOf"].iter().any(|key| {
            children(schema, key)
                .into_iter()
                .any(|o| has_json_schema_max_length(Some(o), max_length))
        })
}

fn has_issue(issues: &[Issue], message: Option<&str>, path: Value) -> bool {
    issues
        .iter()
        .any(|i| message.is_none_or(|m| i.message == m) && i.path_json() == path)
}

// --- flowSpecSchema ---

#[test]
fn flowspecschema_emits_json_schema_descriptions_for_every_property_including_recursive_steps() {
    let json_schema = z::to_json_schema(&fc::flow_spec_schema());
    assert!(json_schema.is_object());

    let description = json_schema["description"]
        .as_str()
        .expect("Expected flowSpecSchema to have a description");
    assert!(!description.trim().is_empty());
    visit_json_schema(&json_schema);
}

#[test]
fn flowspecschema_publishes_text_and_label_maximum_lengths_in_json_schema() {
    let json_schema = z::to_json_schema(&fc::flow_spec_schema());
    assert!(json_schema.is_object());

    let label_schema = find_json_schema_node(
        &json_schema,
        "Button label shown to the contact (max 20 characters).",
    );
    let text_schema = find_json_schema_node(
        &json_schema,
        "Text message body (max 1000 characters). Exactly one of text/imageUrl/fileUrl is required.",
    );

    assert!(has_json_schema_max_length(label_schema, 20));
    assert!(has_json_schema_max_length(text_schema, 1000));
}

#[test]
fn flowspecschema_publishes_the_sendviewcatalog_length_limits_to_agents() {
    let json_schema = z::to_json_schema(&fc::flow_spec_schema());

    let text_schema = find_json_schema_node(
        &json_schema,
        "Message text shown above the catalog button (max 1024 characters, counted in UTF-16 units so an emoji may count as 2). May include `{{variable}}` placeholders.",
    );
    let footer_schema = find_json_schema_node(
        &json_schema,
        "Optional small text under the message (max 60 characters, UTF-16 units; use an empty string for none). May include `{{variable}}` placeholders.",
    );

    assert!(has_json_schema_max_length(text_schema, 1024));
    assert!(has_json_schema_max_length(footer_schema, 60));
}

#[test]
fn flowspecschema_requires_exactly_one_send_content_kind() {
    let missing_content_issues = expect_invalid(&create_flow_spec(json!([{ "type": "send" }])));
    let multiple_content_issues = expect_invalid(&create_flow_spec(json!([{
        "type": "send",
        "text": "Hello",
        "imageUrl": "https://example.com/image.png",
    }])));

    for issues in [missing_content_issues, multiple_content_issues] {
        assert!(has_issue(
            &issues,
            Some("Exactly one of text, imageUrl, or fileUrl is required."),
            json!(["steps", 0])
        ));
    }
}

#[test]
fn flowspecschema_enforces_action_specific_required_fields() {
    let missing_tags_issues = expect_invalid(&create_flow_spec(
        json!([{ "type": "action", "action": "addTags", "tagNames": [] }]),
    ));
    let missing_custom_field_issues = expect_invalid(&create_flow_spec(
        json!([{ "type": "action", "action": "setCustomField" }]),
    ));

    assert!(has_issue(
        &missing_tags_issues,
        Some("action \"addTags\" requires a non-empty tagNames"),
        json!(["steps", 0, "tagNames"])
    ));
    assert!(has_issue(
        &missing_custom_field_issues,
        Some("action \"setCustomField\" requires customFieldName"),
        json!(["steps", 0, "customFieldName"])
    ));
    assert!(has_issue(
        &missing_custom_field_issues,
        Some("action \"setCustomField\" requires value"),
        json!(["steps", 0, "value"])
    ));
}

#[test]
fn flowspecschema_parses_three_nested_branch_button_levels_and_preserves_a_deep_error_path() {
    assert!(
        fc::flow_spec_schema()
            .safe_parse(&create_three_level_nested_flow_spec(
                json!({ "type": "addNote", "note": "Internal note" })
            ))
            .success
    );

    let issues = expect_invalid(&create_three_level_nested_flow_spec(
        json!({ "type": "addNote", "note": "" }),
    ));

    assert!(has_issue(
        &issues,
        None,
        json!([
            "steps", 0, "cases", 0, "then", 0, "buttons", 0, "then", 0, "cases", 0, "then", 0,
            "note"
        ])
    ));
}

#[test]
fn flowspecschema_enforces_flow_spec_limits() {
    let too_many_buttons: Vec<Value> = (0..4)
        .map(|index| json!({ "text": format!("Option {}", index + 1) }))
        .collect();

    for invalid_spec in [
        create_flow_spec(json!([{ "type": "send", "text": "x".repeat(1001) }])),
        create_flow_spec(json!([{ "type": "send", "text": "Hello", "buttons": too_many_buttons }])),
        create_flow_spec(
            json!([{ "type": "send", "text": "Hello", "buttons": [{ "text": "x".repeat(21) }] }]),
        ),
        spread(
            &create_flow_spec(json!([{ "type": "send", "text": "Hello" }])),
            json!({ "name": "x".repeat(256) }),
        ),
    ] {
        assert!(!fc::flow_spec_schema().safe_parse(&invalid_spec).success);
    }
}

// --- flowSpecStepTypes ---

#[test]
fn flowspecsteptypes_derives_one_entry_per_flowstepspecschema_member_each_with_a_non_empty_description()
 {
    let step_types = fc::flow_spec_step_types().as_array().unwrap();
    assert!(!step_types.is_empty());
    for step_type in step_types {
        assert!(!step_type["type"].as_str().unwrap().is_empty());
        assert!(!step_type["description"].as_str().unwrap().is_empty());
    }
}

#[test]
fn flowspecsteptypes_covers_every_step_type_the_dsl_union_declares() {
    let mut types: Vec<&str> = fc::flow_spec_step_types()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    types.sort();
    let mut expected = vec![
        "action",
        "addNote",
        "branch",
        "goto",
        "pipeline",
        "send",
        "sendProducts",
        "sendTemplate",
        "sendViewCatalog",
        "startFlow",
        "wait",
    ];
    expected.sort();
    assert_eq!(types, expected);
}

// --- setCustomField action operation ---

fn parse_action(action: Value) -> SafeParse {
    fc::flow_spec_schema().safe_parse(&json!({
        "formatVersion": 1,
        "name": "Flow",
        "steps": [spread(&json!({ "type": "action", "action": "setCustomField" }), action)],
    }))
}

#[test]
fn setcustomfield_action_operation_clear_and_setnow_need_no_value() {
    assert!(parse_action(json!({ "customFieldName": "F", "operation": "clear" })).success);
    assert!(parse_action(json!({ "customFieldName": "F", "operation": "setNow" })).success);
}

#[test]
fn setcustomfield_action_operation_an_operation_that_needs_an_operand_rejects_a_missing_one() {
    assert!(!parse_action(json!({ "customFieldName": "F", "operation": "addDays" })).success);
}

#[test]
fn setcustomfield_action_operation_new_authoring_input_rejects_a_blank_operand_for_increase_and_decrease()
 {
    for operation in ["increase", "decrease"] {
        assert!(!parse_action(json!({ "customFieldName": "F", "operation": operation })).success);
        assert!(
            parse_action(json!({ "customFieldName": "F", "operation": operation, "value": "2" }))
                .success
        );
    }
}

#[test]
fn setcustomfield_action_operation_an_explicit_set_needs_a_value_just_like_an_omitted_operation() {
    assert!(!parse_action(json!({ "customFieldName": "F", "operation": "set" })).success);
    assert!(
        parse_action(json!({ "customFieldName": "F", "operation": "set", "value": "x" })).success
    );
}

#[test]
fn setcustomfield_action_operation_an_unknown_operation_name_is_rejected() {
    assert!(
        !parse_action(json!({ "customFieldName": "F", "operation": "multiply", "value": "1" }))
            .success
    );
}

#[test]
fn setcustomfield_action_operation_without_an_operation_the_value_stays_required() {
    assert!(!parse_action(json!({ "customFieldName": "F" })).success);
}
