//! Port of `packages/flow-config/src/steps/generate-code.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn generate_code_type() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"NUMERIC_LENGTH":"NUMERIC_LENGTH","NUMERIC_VALUE":"NUMERIC_VALUE","ALPHANUMERIC_LENGTH":"ALPHANUMERIC_LENGTH"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn generate_code_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("generateCode")).describe("Step type discriminator: \"generateCode\".")),
("type", z::enum_(&["NUMERIC_LENGTH", "NUMERIC_VALUE", "ALPHANUMERIC_LENGTH"])),
("min", z::number().coerce().int().gte(0 as f64).lte(9007199254740990 as f64)),
("max", z::number().coerce().int().gte(0 as f64).lte(9007199254740991 as f64)),
("outputFieldId", z::string().trim().min(1).refine(generate_code_step_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None))]).refine(generate_code_step_schema_refine, Some("Max must be greater than or equal to Min"), Some(&["max"])));

default_fn!(pub fn generate_code_step_default_fn = json!({"id": create_id(), "stepType": "generateCode", "type": "NUMERIC_LENGTH", "min": 0, "max": 100, "outputFieldId": ""}));

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn generate_code_step_schema_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// data=>data.min<=data.max
fn generate_code_step_schema_refine(value: &Value) -> bool {
    todo!("P3.4")
}
