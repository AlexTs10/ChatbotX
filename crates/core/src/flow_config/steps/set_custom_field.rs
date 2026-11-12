//! Port of `packages/flow-config/src/steps/set-custom-field.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn set_custom_field_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("setCustomField")).describe("Step type discriminator: \"setCustomField\".")),
("inputFieldId", z::string().trim().min(1).refine(set_custom_field_step_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("operation", z::enum_(&["O01", "O02", "O03", "O04", "O05", "O06", "O07", "O08", "O09", "O10", "O11", "O12"])),
("value", z::string().trim()),
("timezone", z::string().max(64).optional())]).super_refine(set_custom_field_step_schema_super_refine));

schema!(pub fn set_custom_field_action_schema = z::object(vec![("customFieldId", z::string().min(1)),
("operation", z::preprocess(set_custom_field_action_schema_operation_preprocess, z::enum_(&["O01", "O02", "O03", "O04", "O05", "O06", "O07", "O08", "O09", "O10", "O11", "O12"]).default(json!("O01")))),
("value", z::string().default(json!(""))),
("timezone", z::string().max(64).optional())]).super_refine(set_custom_field_action_schema_super_refine));

default_fn!(pub fn set_custom_field_step_default_fn = json!({"id": create_id(), "stepType": "setCustomField", "value": "", "inputFieldId": "", "operation": "O01"}));

fn set_custom_field_step_schema_input_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// The registry's operation codes (`FieldOperationType` is re-exported).
pub use crate::utils::custom_field_operation::FieldOperationType;

const BLANK_OPERAND_LEGACY_OPERATIONS: &[FieldOperationType] =
    &[FieldOperationType::Increase, FieldOperationType::Decrease];

/// `isOperandBlank`: the operation needs an operand and the value is blank.
pub fn is_operand_blank(operation: FieldOperationType, value: Option<&str>) -> bool {
    crate::utils::custom_field_operation::field_operation_definition(operation).requires_operand
        && value.map(crate::js::trim).unwrap_or("").is_empty()
}

/// `isFieldOperandMissing`: like `isOperandBlank`, increase/decrease exempt.
pub fn is_field_operand_missing(operation: FieldOperationType, value: Option<&str>) -> bool {
    !BLANK_OPERAND_LEGACY_OPERATIONS.contains(&operation) && is_operand_blank(operation, value)
}

/// `withFieldOperationOperandRule(schema)`.
pub fn with_field_operation_operand_rule(schema: Schema) -> Schema {
    schema.super_refine(operand_rule)
}

fn operand_rule(data: &Value, ctx: &mut RefineCtx) {
    let Some(operation) = data["operation"]
        .as_str()
        .and_then(FieldOperationType::parse)
    else {
        return;
    };
    if is_field_operand_missing(operation, data["value"].as_str()) {
        ctx.issue(path!["value"], "A value is required for this operation.");
    }
}

fn set_custom_field_step_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    operand_rule(data, ctx);
}

fn set_custom_field_action_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    operand_rule(data, ctx);
}

fn set_custom_field_action_schema_operation_preprocess(value: Option<Value>) -> Option<Value> {
    value.filter(|v| truthy(Some(v)))
}
