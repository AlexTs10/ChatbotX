//! Port of `packages/flow-config/__tests__/authoring/set-custom-field-operations.test.ts`.

mod flow_config_authoring_support;
mod flow_config_support;
use flow_config_authoring_support::*;
use flow_config_support::{SafeParse, Value, json, spread};

use chatbotx_core::utils::custom_field_operation::{
    FieldOperationType, field_operation_codes, field_operation_definition,
};

const FIELDS: &[(&str, &str)] = &[
    ("number", "number"),
    ("datetime", "datetime"),
    ("text", "shortText"),
    ("flag", "boolean"),
];

fn ctx() -> FlowAuthoringContext {
    FlowAuthoringContext {
        custom_fields_by_name: FIELDS
            .iter()
            .enumerate()
            .map(|(index, (name, field_type))| {
                (
                    (*name).to_string(),
                    custom_field(&(3000 + index).to_string(), field_type),
                )
            })
            .collect(),
        ..FlowAuthoringContext::default()
    }
}

fn spec(custom_field_name: &str, operation: Option<&str>, value: Option<&str>) -> Value {
    let mut step = json!({ "type": "action", "action": "setCustomField", "customFieldName": custom_field_name });
    if let Some(operation) = operation {
        step["operation"] = json!(operation);
    }
    if let Some(value) = value {
        step["value"] = json!(value);
    }
    json!({ "formatVersion": 1, "name": "Flow", "steps": [step] })
}

fn compiled_operation(compiled: &CompiledFlow) -> Option<String> {
    let node = &compiled.nodes[0];
    if node["type"] != "performAction" {
        return None;
    }
    node["data"]["details"]["steps"][0]["operation"]
        .as_str()
        .map(str::to_string)
}

fn parse_spec(action: Value) -> SafeParse {
    fc::flow_spec_schema().safe_parse(&json!({
        "formatVersion": 1,
        "name": "Flow",
        "steps": [spread(&json!({ "type": "action", "action": "setCustomField" }), action)],
    }))
}

/// One compatible field name per operation, taken from the registry.
fn field_for(code: FieldOperationType) -> &'static str {
    let allowed: Vec<&str> = field_operation_definition(code)
        .allowed_types
        .iter()
        .map(|t| t.as_str())
        .collect();
    FIELDS
        .iter()
        .find(|(_, field_type)| allowed.contains(field_type))
        .map(|(name, _)| *name)
        .unwrap_or_else(|| panic!("no test field for {code}"))
}

// --- authoring setCustomField operation by apiName ---

#[test]
fn authoring_setcustomfield_operation_by_apiname_s_compiles_to_s() {
    for &code in field_operation_codes() {
        let definition = field_operation_definition(code);
        let value = definition.requires_operand.then_some("1");

        let compiled = compile_ok(
            &spec(field_for(code), Some(definition.api_name), value),
            &ctx(),
        );

        assert_eq!(
            compiled_operation(&compiled).as_deref(),
            Some(code.as_str()),
            "{}",
            definition.api_name
        );
    }
}

// adapted: an `undefined` operation property is an absent key
#[test]
fn authoring_setcustomfield_operation_by_apiname_an_omitted_operation_compiles_to_o01() {
    let compiled = compile_ok(&spec("number", None, Some("5")), &ctx());
    assert_eq!(compiled_operation(&compiled).as_deref(), Some("O01"));
}

#[test]
fn authoring_setcustomfield_operation_by_apiname_s_compiles_with_no_value() {
    for operation in ["clear", "setNow"] {
        let field = if operation == "setNow" {
            "datetime"
        } else {
            "number"
        };

        let compiled = compile_ok(&spec(field, Some(operation), None), &ctx());

        assert_eq!(
            compiled_operation(&compiled).as_deref(),
            Some(if operation == "setNow" { "O08" } else { "O06" })
        );
    }
}

#[test]
fn authoring_setcustomfield_operation_by_apiname_s_with_s_is_rejected_at_compile_with_invalidfieldoperation()
 {
    for (field, operation, value) in [
        ("flag", "increase", Some("1")),
        ("text", "formula", Some("1")),
        ("number", "setNow", None),
        ("flag", "append", Some("x")),
    ] {
        let errors = compile_errors(&spec(field, Some(operation), value), &ctx());
        let first = &errors[0];
        assert_eq!(first.code, "invalidFieldOperation", "{field} {operation}");
        assert_eq!(first.path, "steps[0].operation");
    }
}

// --- authoring setCustomField spec operands ---

#[test]
fn authoring_setcustomfield_spec_operands_s_rejects_a_blank_or_whitespace_operand() {
    for operation in ["increase", "decrease", "formula", "addDays"] {
        assert!(
            !parse_spec(json!({ "customFieldName": "F", "operation": operation, "value": "" }))
                .success
        );
        assert!(
            !parse_spec(json!({ "customFieldName": "F", "operation": operation, "value": "  " }))
                .success
        );
        assert!(!parse_spec(json!({ "customFieldName": "F", "operation": operation })).success);
    }
}

#[test]
fn authoring_setcustomfield_spec_operands_an_explicit_set_without_value_is_rejected_exactly_like_an_omitted_operation()
 {
    let explicit = parse_spec(json!({ "customFieldName": "F", "operation": "set" }));
    let omitted = parse_spec(json!({ "customFieldName": "F" }));

    assert!(!explicit.success);
    assert!(!omitted.success);
    assert_eq!(explicit.messages(), omitted.messages());
}

#[test]
fn authoring_setcustomfield_spec_operands_s_needs_no_value() {
    for operation in ["clear", "setNow"] {
        assert!(parse_spec(json!({ "customFieldName": "F", "operation": operation })).success);
    }
}

// --- flow-config operand rule and legacy exemption ---

#[test]
fn flow_config_operand_rule_and_legacy_exemption_only_increase_and_decrease_are_exempt_from_the_operand_rule()
 {
    let exempt: Vec<&str> = field_operation_codes()
        .iter()
        .copied()
        .filter(|&code| {
            fc::is_operand_blank(code, Some("")) && !fc::is_field_operand_missing(code, Some(""))
        })
        .map(FieldOperationType::as_str)
        .collect();

    assert_eq!(exempt, vec!["O04", "O05"]);
}

#[test]
fn flow_config_operand_rule_and_legacy_exemption_s_is_exempt_for_blank_input_but_not_for_new_authoring()
 {
    for code in [FieldOperationType::Increase, FieldOperationType::Decrease] {
        assert!(!fc::is_field_operand_missing(code, Some("")));
        assert!(fc::is_operand_blank(code, Some("  ")));
    }
}

#[test]
fn flow_config_operand_rule_and_legacy_exemption_stored_flow_json_operation_o01_blank_value_parses()
{
    let stored = spread(
        &fc::set_custom_field_step_default_fn(),
        json!({ "inputFieldId": "42", "operation": "O01", "value": "" }),
    );

    assert!(
        fc::set_custom_field_step_schema()
            .safe_parse(&stored)
            .success
    );
    assert!(
        fc::set_custom_field_action_schema()
            .safe_parse(&json!({ "customFieldId": "42", "operation": "O01", "value": "" }))
            .success
    );
}

#[test]
fn flow_config_operand_rule_and_legacy_exemption_the_operand_rule_reports_on_the_value_path() {
    let result = fc::set_custom_field_step_schema().safe_parse(&spread(
        &fc::set_custom_field_step_default_fn(),
        json!({ "inputFieldId": "42", "operation": "O07", "value": " " }),
    ));

    assert!(!result.success);
    assert_eq!(
        result.error.unwrap().issues[0].path_json(),
        json!(["value"])
    );
}
