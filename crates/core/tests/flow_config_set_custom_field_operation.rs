//! Port of `packages/flow-config/__tests__/set-custom-field-operation.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use std::any::TypeId;

fn step(operation: &str, value: &str) -> SafeParse {
    fc::set_custom_field_step_schema().safe_parse(&spread(
        &fc::set_custom_field_step_default_fn(),
        json!({ "inputFieldId": "1", "operation": operation, "value": value }),
    ))
}

#[test]
fn fieldoperationtype_is_the_registry_s_enum_so_existing_imports_keep_working() {
    // `toBe` on the enum object: the flow-config name is the registry type.
    assert_eq!(
        TypeId::of::<fc::FieldOperationType>(),
        TypeId::of::<chatbotx_core::utils::custom_field_operation::FieldOperationType>()
    );
}

#[test]
fn setcustomfieldstepschema_operand_rule_legacy_s_keeps_parsing_with_a_blank_value_stored_flows() {
    for operation in ["O01", "O02", "O03", "O04", "O05"] {
        assert!(step(operation, "").success, "{operation}");
    }
}

#[test]
fn setcustomfieldstepschema_operand_rule_s_requires_an_operand() {
    for operation in ["O07", "O09", "O10", "O11", "O12"] {
        assert!(!step(operation, "").success, "{operation}");
        assert!(!step(operation, "  ").success, "{operation}");
        assert!(step(operation, "1").success, "{operation}");
    }
}

#[test]
fn setcustomfieldstepschema_operand_rule_s_needs_no_operand() {
    for operation in ["O06", "O08"] {
        assert!(step(operation, "").success, "{operation}");
    }
}

#[test]
fn setcustomfieldstepschema_operand_rule_rejects_an_unknown_operation_code() {
    assert!(!step("O99", "1").success);
}

fn parse(action: Value) -> SafeParse {
    fc::set_custom_field_action_schema()
        .safe_parse(&spread(&json!({ "customFieldId": "42" }), action))
}

#[test]
fn setcustomfieldactionschema_trigger_action_a_legacy_blank_operation_j_defaults_to_set() {
    for action in [
        json!({ "value": "x" }),
        json!({ "operation": "", "value": "x" }),
    ] {
        let result = parse(action);
        assert!(result.success);
        assert_eq!(result.data.unwrap()["operation"], "O01");
    }
}

#[test]
fn setcustomfieldactionschema_trigger_action_a_missing_value_defaults_to_blank() {
    let result = parse(json!({ "operation": "O06" }));
    assert!(result.success);
    assert_eq!(result.data.unwrap()["value"], "");
}

#[test]
fn setcustomfieldactionschema_trigger_action_requires_an_operand_for_a_formula() {
    assert!(!parse(json!({ "operation": "O07" })).success);
}

#[test]
fn setcustomfieldactionschema_trigger_action_requires_a_field_id_and_a_known_operation() {
    assert!(
        !fc::set_custom_field_action_schema()
            .safe_parse(&json!({ "value": "x" }))
            .success
    );
    assert!(!parse(json!({ "operation": "O99" })).success);
}
