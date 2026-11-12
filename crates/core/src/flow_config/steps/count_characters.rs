//! Port of `packages/flow-config/src/steps/count-characters.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn count_characters_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("countCharacters")).describe("Step type discriminator: \"countCharacters\".")),
("inputFieldId", z::string().trim().min(1).refine(count_characters_step_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("outputFieldId", z::string().trim().min(1).refine(count_characters_step_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None))]));

// HAND-PORT countCharactersStepDefaultFn:
// props=>({id:createId(),stepType:stepTypes.enum.countCharacters,inputFieldId:"",outputFieldId:"",...props})

fn count_characters_step_schema_input_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn count_characters_step_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `countCharactersStepDefaultFn(props)`.
pub fn count_characters_step_default_fn(props: &Value) -> Value {
    spread_props(
        json!({"id": create_id(), "stepType": "countCharacters", "inputFieldId": "", "outputFieldId": ""}),
        props,
    )
}
