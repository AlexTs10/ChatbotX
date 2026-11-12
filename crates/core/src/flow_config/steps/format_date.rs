//! Port of `packages/flow-config/src/steps/format-date.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn format_timezone() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"contact":"contact","workspace":"workspace"}"#)
            .expect("valid JSON")
    });
    &V
}

schema!(pub fn format_date_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("formatDate")).describe("Step type discriminator: \"formatDate\".")),
("inputFieldId", z::string().trim().min(1).refine(format_date_step_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("format", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(format_date_step_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("timezone", z::enum_(&["contact", "workspace"]))]));

// HAND-PORT formatDateStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.formatDate,inputFieldId:"",format:"",outputFieldId:"",timezone:FormatTimezone.contact,...props})

fn format_date_step_schema_input_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn format_date_step_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `formatDateStepDefaultFn(props)`.
pub fn format_date_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "formatDate", "inputFieldId": "", "format": "", "outputFieldId": "", "timezone": "contact"}),
        props,
    )
}
