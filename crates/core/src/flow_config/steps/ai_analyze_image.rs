//! Port of `packages/flow-config/src/steps/ai-analyze-image.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_analyze_image_provider = z::enum_(&["openai", "gemini", "claude", "openrouter", "muse"]));

schema!(pub fn ai_analyze_image_provider_schema = z::enum_(&["openai", "gemini", "claude", "openrouter", "muse", "openaiCompatible"]));

pub fn default_analyze_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":"gpt-5.4-mini","gemini":"gemini-3.5-flash","claude":"claude-haiku-4-5-20251001","openrouter":"openai/gpt-5.4-mini","muse":"muse-spark-1.3"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn ai_analyze_image_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiAnalyzeImage")).describe("Step type discriminator: \"aiAnalyzeImage\".")),
("provider", ai_analyze_image_provider_schema()),
("integrationId", z::string().trim().optional()),
("model", z::string().trim()),
("prompt", z::string().trim().min(1)),
("inputFieldId", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(ai_analyze_image_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("temperature", z::number().gte(0_f64).lte(2_f64).describe("Sampling temperature, 0-2 (Muse: 0-1).")),
("maxOutputTokens", z::number().int().gte(250_f64).lte(4096_f64).describe("Maximum tokens of reply text. Muse models may use up to 1024 extra reasoning tokens on top, billed as output.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]).super_refine(ai_analyze_image_schema_super_refine));

fn ai_analyze_image_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn ai_analyze_image_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    ai_model_rules(value, ctx, true);
}

/// The `model` / `integrationId` / Muse temperature rules of the AI steps.
pub(crate) fn ai_model_rules(value: &Value, ctx: &mut RefineCtx, muse_temperature: bool) {
    if trimmed(&value["model"]).unwrap_or("").is_empty() {
        ctx.issue(path!["model"], "Model is required");
    }
    if value["provider"] == "openaiCompatible"
        && trimmed(&value["integrationId"]).unwrap_or("").is_empty()
    {
        ctx.issue(path!["integrationId"], "Integration is required");
    }
    if muse_temperature
        && value["provider"] == "muse"
        && value["temperature"]
            .as_f64()
            .is_some_and(|t| t > MUSE_MAX_TEMPERATURE as f64)
    {
        ctx.issue(
            path!["temperature"],
            format!(
                "Muse supports temperature up to {}",
                crate::js::number_to_string(MUSE_MAX_TEMPERATURE as f64)
            ),
        );
    }
}

/// `AIAnalyzeImageDefaultFn(props)`.
pub fn ai_analyze_image_default_fn(props: &Value) -> Value {
    let head = ai_step_head(props, |p| default_analyze_models().get(p).cloned());
    spread3(
        merge(
            json!({ "id": create_id() }),
            merge(
                head,
                json!({
                    "prompt": "",
                    "inputFieldId": "",
                    "outputFieldId": "",
                    "temperature": 1.0,
                    "maxOutputTokens": 1000,
                }),
            ),
        ),
        props,
        json!({ "stepType": "aiAnalyzeImage", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}
