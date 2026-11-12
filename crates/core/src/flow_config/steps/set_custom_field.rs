//! Port of `packages/flow-config/src/steps/set-custom-field.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isOperandBlank:
// (operation,value)=>FIELD_OPERATION_DEFINITIONS[operation].requiresOperand&&!value?.trim()

// HAND-PORT isFieldOperandMissing:
// (operation,value)=>!BLANK_OPERAND_LEGACY_OPERATIONS.includes(operation)&&isOperandBlank(operation,value)

// HAND-PORT withFieldOperationOperandRule:
// schema=>schema.superRefine(({operation,value},ctx)=>{if(isFieldOperandMissing(operation,value)){ctx.addIssue({code:"custom",path:["value"],message:"A value is required for this operation."})}})

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

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn set_custom_field_step_schema_input_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ({operation,value},ctx)=>{if(isFieldOperandMissing(operation,value)){ctx.addIssue({code:"custom",path:["value"],message:"A value is required for this operation."})}}
fn set_custom_field_step_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// operation=>operation||void 0
fn set_custom_field_action_schema_operation_preprocess(value: Option<Value>) -> Option<Value> {
    todo!("P3.4")
}

// ({operation,value},ctx)=>{if(isFieldOperandMissing(operation,value)){ctx.addIssue({code:"custom",path:["value"],message:"A value is required for this operation."})}}
fn set_custom_field_action_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

/// The registry's operation codes (`FieldOperationType` is re-exported).
pub use crate::utils::custom_field_operation::FieldOperationType;
