//! Port of `packages/flow-config/src/steps/get-data-from-json.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn get_data_from_json_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("getDataFromJson")).describe("Step type discriminator: \"getDataFromJson\".")),
("inputFieldId", z::string().trim().min(1).refine(get_data_from_json_step_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("mapping", z::array(z::object(vec![("jsonPath", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(get_data_from_json_step_schema_mapping_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None))]))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

default_fn!(pub fn get_data_from_json_step_default_fn = json!({"id": create_id(), "stepType": "getDataFromJson", "inputFieldId": "", "mapping": [{"jsonPath": "", "outputFieldId": ""}], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn get_data_from_json_step_schema_input_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn get_data_from_json_step_schema_mapping_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}
