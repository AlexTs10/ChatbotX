//! Port of `packages/flow-config/src/steps/clear-custom-field.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn clear_custom_field_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("clearCustomField")).describe("Step type discriminator: \"clearCustomField\".")),
("inputFieldId", z::string().trim().min(1).refine(clear_custom_field_step_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None))]));

// HAND-PORT clearCustomFieldStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.clearCustomField,inputFieldId:"",...props})

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn clear_custom_field_step_schema_input_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}
