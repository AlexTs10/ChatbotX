//! Port of `packages/flow-config/src/steps/ai-generate-image.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_generate_image_quality = z::enum_(&["auto", "hd", "md", "ld"]));

schema!(pub fn image_aspect_ratio = z::enum_(&["auto", "1:1", "4:3", "3:4", "16:9", "9:16"]));

pub const IMAGE_AUTO_VALUE: &str = "auto";

pub const IMAGE_BASE64_ENCODING: &str = "base64";

pub const IMAGE_DEFAULT_EXTENSION: &str = "png";

pub const IMAGE_DEFAULT_MIME_TYPE: &str = "image/png";

pub fn default_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"openai":"gpt-image-2","gemini":"gemini-3.1-flash-image","muse":"muse-image-1.0"}"#,
        )
        .expect("valid JSON")
    });
    &V
}

schema!(pub fn ai_generate_image_provider = z::enum_(&["openai", "gemini", "muse"]));

schema!(pub fn ai_generate_image_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiGenerateImage")).describe("Step type discriminator: \"aiGenerateImage\".")),
("provider", ai_generate_image_provider()),
("model", z::string().trim().min(1)),
("prompt", z::string().trim().min(1)),
("quality", ai_generate_image_quality()),
("size", z::string().trim().min(1)),
("outputFieldId", z::string().trim().min(1).refine(ai_generate_image_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]));

fn ai_generate_image_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `getAIGeneratedImagePath`.
pub fn get_ai_generated_image_path(
    storage_prefix: &str,
    file_name: &str,
    conversation_id: &str,
) -> String {
    format!("{storage_prefix}/conversations/{conversation_id}/{file_name}")
}

/// `aiGenerateImageDefaultFn(props)`.
pub fn ai_generate_image_default_fn(props: &Value) -> Value {
    let provider = props
        .get("provider")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!("openai"));
    let model = props
        .get("model")
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| {
            provider
                .as_str()
                .and_then(|p| default_models().get(p))
                .cloned()
        });
    let mut base = json!({
        "id": create_id(),
        "provider": provider,
        "prompt": "",
        "size": "auto",
        "quality": "auto",
        "outputFieldId": "",
    });
    if let Some(model) = model {
        base["model"] = model;
    }
    spread3(
        base,
        props,
        json!({ "stepType": "aiGenerateImage", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}
