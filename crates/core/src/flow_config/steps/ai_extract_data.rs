//! Port of `packages/flow-config/src/steps/ai-extract-data.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn ai_extract_data_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":{"models":["gpt-6-astra","gpt-6-sol","gpt-6-luna","gpt-5.5-pro","gpt-5.5","gpt-5.4-pro","gpt-5.4","gpt-5.4-mini","gpt-5.4-nano","gpt-5.2","gpt-5.1","gpt-5-mini","gpt-5","gpt-5-nano","gpt-4.1-mini","gpt-4.1-nano","gpt-4.1","gpt-4o-mini","gpt-4o"],"default":"gpt-5.4-mini"},"gemini":{"models":["gemini-3.8-flash","gemini-3.7-flash","gemini-3.6-flash","gemini-3.5-flash-lite","gemini-3.5-flash","gemini-3.1-flash-lite","gemini-3.1-pro-preview"],"default":"gemini-3.5-flash"},"claude":{"models":["claude-fable-5-1","claude-opus-5-5","claude-sonnet-5","claude-fable-5","claude-opus-4-8","claude-opus-4-6","claude-sonnet-4-6","claude-sonnet-4-5-20250929","claude-haiku-4-5-20251001"],"default":"claude-haiku-4-5-20251001"},"openrouter":{"models":["openai/gpt-6-astra","openai/gpt-6-sol","openai/gpt-6-luna","openai/gpt-5.5-pro","openai/gpt-5.5","openai/gpt-5.4-pro","openai/gpt-5.4","openai/gpt-5.4-mini","openai/gpt-5.2-pro","openai/gpt-5.2","openai/gpt-5.1","openai/gpt-5","openai/gpt-4.1","openai/gpt-4o","openai/gpt-4o-mini","anthropic/claude-fable-5","anthropic/claude-fable-5.1","anthropic/claude-opus-5.5","anthropic/claude-sonnet-5","anthropic/claude-opus-4.8","anthropic/claude-opus-4.6","anthropic/claude-sonnet-4.6","anthropic/claude-sonnet-4.5","anthropic/claude-haiku-4.5","google/gemini-3.5-flash","google/gemini-3.8-flash","google/gemini-3.7-flash","google/gemini-3.6-flash","google/gemini-3.5-flash-lite","google/gemini-3.1-pro-preview","google/gemini-3.1-flash-lite","google/gemini-2.5-flash","google/gemini-2.5-flash-lite","google/gemini-2.5-pro"],"default":"openai/gpt-5.4-mini"},"muse":{"models":["muse-spark-1.3","muse-spark-1.2","muse-wonder-1.0"],"default":"muse-spark-1.3"}}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn ai_extract_data_native_provider_schema = z::enum_(&["openai", "gemini", "claude", "openrouter", "muse"]));

schema!(pub fn ai_extract_data_provider_schema = z::enum_(&["openai", "gemini", "claude", "openrouter", "muse", "openaiCompatible"]));

schema!(pub fn ai_extract_data_schema = z::discriminated_union("inputType", vec![z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiExtractData")).describe("Step type discriminator: \"aiExtractData\".")),
("extractFields", z::array(z::object(vec![("key", z::string().trim().min(1)),
("customFieldId", z::string().trim().min(1).refine(ai_extract_data_schema_0_extract_fields_custom_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("description", z::string().trim().optional())]))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional()),
("provider", ai_extract_data_provider_schema()),
("integrationId", z::string().trim().optional()),
("model", z::string().trim()),
("inputType", z::literal(json!("text"))),
("inputFieldId", z::string().trim().min(1)),
("file", z::object(vec![("attribute", z::string()),
("value", z::string())]).optional())]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiExtractData")).describe("Step type discriminator: \"aiExtractData\".")),
("extractFields", z::array(z::object(vec![("key", z::string().trim().min(1)),
("customFieldId", z::string().trim().min(1).refine(ai_extract_data_schema_0_extract_fields_custom_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("description", z::string().trim().optional())]))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional()),
("provider", ai_extract_data_provider_schema()),
("integrationId", z::string().trim().optional()),
("model", z::string().trim()),
("inputType", z::literal(json!("image"))),
("inputFieldId", z::string().trim().min(1))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiExtractData")).describe("Step type discriminator: \"aiExtractData\".")),
("extractFields", z::array(z::object(vec![("key", z::string().trim().min(1)),
("customFieldId", z::string().trim().min(1).refine(ai_extract_data_schema_0_extract_fields_custom_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("description", z::string().trim().optional())]))),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional()),
("provider", ai_extract_data_provider_schema()),
("integrationId", z::string().trim().optional()),
("model", z::string().trim()),
("inputType", z::literal(json!("file"))),
("inputFieldId", z::string().trim().min(1))])]).super_refine(ai_extract_data_schema_super_refine));

fn ai_extract_data_schema_0_extract_fields_custom_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn ai_extract_data_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    ai_model_rules(value, ctx, false);
}

/// `aiExtractDataDefaultFn(props)`.
pub fn ai_extract_data_default_fn(props: &Value) -> Value {
    let head = ai_step_head(props, |p| {
        ai_extract_data_models()
            .get(p)
            .map(|m| m["default"].clone())
    });
    spread3(
        merge(
            json!({ "id": create_id(), "inputType": "text", "inputFieldId": "" }),
            merge(head, json!({ "extractFields": [] })),
        ),
        props,
        json!({ "stepType": "aiExtractData", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}
