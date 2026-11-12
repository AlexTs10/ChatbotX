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
("temperature", z::number().gte(0 as f64).lte(2 as f64).describe("Sampling temperature, 0-2 (Muse: 0-1).")),
("maxOutputTokens", z::number().int().gte(250 as f64).lte(4096 as f64).describe("Maximum tokens of reply text. Muse models may use up to 1024 extra reasoning tokens on top, billed as output.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]).super_refine(ai_analyze_image_schema_super_refine));

// HAND-PORT AIAnalyzeImageDefaultFn:
// (props={})=>{const provider=props.provider??"openai";const model=props.model??(provider==="openaiCompatible"?"":defaultAnalyzeModels[provider]);return{id:createId(),provider,...provider==="openaiCompatible"?{integrationId:"integrationId"in props?props.integrationId:""}:{},model,prompt:"",inputFieldId:"",outputFieldId:"",temperature:1,maxOutputTokens:1e3,...props,stepType:stepTypes.enum.aiAnalyzeImage,states:[successStateDefaultFn(),errorStateDefaultFn()]}}

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn ai_analyze_image_schema_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// (value,ctx)=>{if(!value.model.trim()){ctx.addIssue({code:"custom",path:["model"],message:"Model is required"})}if(value.provider==="openaiCompatible"&&!value.integrationId?.trim()){ctx.addIssue({code:"custom",path:["integrationId"],message:"Integration is required"})}if(value.provider==="muse"&&value.temperature>MUSE_MAX_TEMPERATURE){ctx.addIssue({code:"custom",path:["temperature"],message:`Muse supports temperature up to ${MUSE_MAX_TEMPERATURE}`})}}
fn ai_analyze_image_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn ai_analyze_image_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
