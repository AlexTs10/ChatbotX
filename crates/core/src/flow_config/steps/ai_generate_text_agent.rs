//! Port of `packages/flow-config/src/steps/ai-generate-text-agent.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_generate_text_agent_provider = z::enum_(&["openai", "gemini", "claude", "deepseek", "openrouter", "muse", "openaiCompatible"]));

schema!(pub fn ai_generate_text_agent_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiGenerateTextAgent")).describe("Step type discriminator: \"aiGenerateTextAgent\".")),
("provider", ai_generate_text_agent_provider().catch_value(/* () => value */ json!("openai"))),
("integrationId", z::string().trim().optional()),
("model", z::string().trim().optional()),
("aiAgentId", z::string().regex(r#"^\d+$"#, "")),
("message", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(ai_generate_text_agent_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("rememberConversation", z::boolean()),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]).super_refine(ai_generate_text_agent_schema_super_refine));

fn ai_generate_text_agent_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `aiGenerateTextAgentDefaultFn(props)`.
pub fn ai_generate_text_agent_default_fn(props: &Value) -> Value {
    let provider = props
        .get("provider")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!("openai"));
    let mut base = json!({
        "id": create_id(),
        "provider": provider,
        "aiAgentId": "",
        "message": "",
        "outputFieldId": "",
        "rememberConversation": true,
    });
    if base["provider"] == "openaiCompatible" {
        base["integrationId"] = props
            .get("integrationId")
            .cloned()
            .unwrap_or_else(|| json!(""));
        base["model"] = props.get("model").cloned().unwrap_or_else(|| json!(""));
    }
    spread3(
        base,
        props,
        json!({ "stepType": "aiGenerateTextAgent", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}

fn ai_generate_text_agent_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    if value["provider"] != "openaiCompatible" {
        return;
    }
    if trimmed(&value["integrationId"]).unwrap_or("").is_empty() {
        ctx.issue(path!["integrationId"], "Integration is required");
    }
    if trimmed(&value["model"]).unwrap_or("").is_empty() {
        ctx.issue(path!["model"], "Model is required");
    }
}
