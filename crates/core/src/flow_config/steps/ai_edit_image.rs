//! Port of `packages/flow-config/src/steps/ai-edit-image.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const AI_EDIT_IMAGE_DEFAULT_OPENAI_MODEL: &str = "gpt-image-2";

pub const AI_EDIT_IMAGE_FALLBACK_OPENAI_MODEL: &str = "gpt-image-1";

pub const AI_EDIT_IMAGE_DEFAULT_GEMINI_MODEL: &str = "gemini-3.1-flash-image";

pub const AI_EDIT_IMAGE_DEFAULT_MUSE_MODEL: &str = "muse-image-1.0";

pub const AI_EDIT_IMAGE_DEFAULT_OPENAI_SIZE: &str = "1024x1024";

pub const AI_EDIT_IMAGE_DEFAULT_GEMINI_SIZE: &str = "1:1";

pub const AI_EDIT_IMAGE_DEFAULT_MUSE_SIZE: &str = "auto";

pub const AI_EDIT_IMAGE_DEFAULT_OPENAI_QUALITY: &str = "low";

pub const AI_EDIT_IMAGE_DEFAULT_GEMINI_QUALITY: &str = "auto";

pub const AI_EDIT_IMAGE_DEFAULT_MUSE_QUALITY: &str = "auto";

schema!(pub fn ai_edit_image_provider = z::enum_(&["openai", "gemini", "muse"]));

schema!(pub fn ai_edit_image_quality = z::enum_(&["auto", "low", "medium", "high", "ld", "md", "hd"]));

pub fn ai_edit_image_default_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"openai":"gpt-image-2","gemini":"gemini-3.1-flash-image","muse":"muse-image-1.0"}"#,
        )
        .expect("valid JSON")
    });
    &V
}

pub fn ai_edit_image_default_sizes() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":"1024x1024","gemini":"1:1","muse":"auto"}"#)
            .expect("valid JSON")
    });
    &V
}

pub fn ai_edit_image_default_qualities() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":"low","gemini":"auto","muse":"auto"}"#)
            .expect("valid JSON")
    });
    &V
}

schema!(pub fn ai_edit_image_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiEditImage")).describe("Step type discriminator: \"aiEditImage\".")),
("provider", ai_edit_image_provider()),
("model", z::string().trim().min(1)),
("inputFieldId", z::string().trim().min(1)),
("prompt", z::string().trim().min(1)),
("size", z::string().trim().min(1)),
("quality", ai_edit_image_quality()),
("outputFieldId", z::string().trim().min(1).refine(ai_edit_image_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]));

// HAND-PORT aiEditImageDefaultFn:
// (props={})=>{const provider=props.provider??"openai";return{id:createId(),provider,model:props.model??aiEditImageDefaultModels[provider],inputFieldId:"",prompt:"",size:props.size??aiEditImageDefaultSizes[provider],quality:props.quality??aiEditImageDefaultQualities[provider],outputFieldId:"",...props,stepType:stepTypes.enum.aiEditImage,states:[successStateDefaultFn(),errorStateDefaultFn()]}}

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn ai_edit_image_schema_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn ai_edit_image_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
