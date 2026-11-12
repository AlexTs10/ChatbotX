//! Port of `packages/flow-config/src/steps/ai-generate-text.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn default_ai_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":"gpt-5.4-mini","gemini":"gemini-3.5-flash","claude":"claude-sonnet-4-6","deepseek":"deepseek-flash","openrouter":"openai/gpt-5.4-mini","muse":"muse-spark-1.3"}"#).expect("valid JSON")
    });
    &V
}

pub const MUSE_MAX_TEMPERATURE: usize = 1;

pub const MUSE_REASONING_HEADROOM_TOKENS: usize = 1024;

pub const MUSE_MAX_OUTPUT_TOKENS_DESCRIPTION: &str = "Maximum tokens of reply text. Muse models may use up to 1024 extra reasoning tokens on top, billed as output.";

schema!(pub fn ai_generate_text_native_provider_schema = z::enum_(&["openai", "gemini", "claude", "deepseek", "openrouter", "muse"]));

schema!(pub fn ai_generate_text_provider_schema = z::enum_(&["openai", "gemini", "claude", "deepseek", "openrouter", "muse", "openaiCompatible"]));

schema!(pub fn ai_generate_text_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiGenerateText")).describe("Step type discriminator: \"aiGenerateText\".")),
("provider", ai_generate_text_provider_schema()),
("integrationId", z::string().trim().optional()),
("model", z::string().trim()),
("system", z::string().trim().optional()),
("text", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(ai_generate_text_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("tools", z::array(z::string()).optional()),
("remember", z::boolean()),
("temperature", z::number().gte(0_f64).lte(2_f64).describe("Sampling temperature, 0-2 (Muse: 0-1).")),
("maxOutputTokens", z::number().int().gte(250_f64).lte(4096_f64).describe("Maximum tokens of reply text. Muse models may use up to 1024 extra reasoning tokens on top, billed as output.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(ai_generate_text_schema_super_refine));

fn ai_generate_text_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn ai_generate_text_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    ai_model_rules(value, ctx, true);
}

/// `aiGenerateTextDefaultFn(props)`.
pub fn ai_generate_text_default_fn(props: &Value) -> Value {
    let head = ai_step_head(props, |p| default_ai_models().get(p).cloned());
    spread3(
        merge(
            json!({ "id": create_id() }),
            merge(
                head,
                json!({
                    "system": "",
                    "text": "",
                    "outputFieldId": "",
                    "tools": [],
                    "remember": false,
                    "temperature": 1.0,
                    "maxOutputTokens": 250,
                }),
            ),
        ),
        props,
        json!({ "stepType": "aiGenerateText", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}
