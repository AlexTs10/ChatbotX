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

// HAND-PORT aiGenerateTextAgentDefaultFn:
// (props={})=>{const provider=props.provider??"openai";return{id:createId(),provider,...provider==="openaiCompatible"?{integrationId:"integrationId"in props?props.integrationId:"",model:"model"in props?props.model:""}:{},aiAgentId:"",message:"",outputFieldId:"",rememberConversation:true,...props,stepType:stepTypes.enum.aiGenerateTextAgent,states:[successStateDefaultFn(),errorStateDefaultFn()]}}

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn ai_generate_text_agent_schema_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// (value,ctx)=>{if(value.provider!=="openaiCompatible"){return}if(!value.integrationId?.trim()){ctx.addIssue({code:"custom",path:["integrationId"],message:"Integration is required"})}if(!value.model?.trim()){ctx.addIssue({code:"custom",path:["model"],message:"Model is required"})}}
fn ai_generate_text_agent_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn ai_generate_text_agent_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
