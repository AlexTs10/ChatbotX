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

fn ai_edit_image_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `aiEditImageDefaultFn(props)`.
pub fn ai_edit_image_default_fn(props: &Value) -> Value {
    let provider = props
        .get("provider")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!("openai"));
    let p = provider.as_str().unwrap_or("").to_owned();
    let pick = |key: &str, table: &Value| {
        props
            .get(key)
            .filter(|v| !v.is_null())
            .cloned()
            .or_else(|| table.get(&p).cloned())
    };
    let mut base = json!({ "id": create_id(), "provider": provider, "inputFieldId": "", "prompt": "", "outputFieldId": "" });
    for (key, table) in [
        ("model", ai_edit_image_default_models()),
        ("size", ai_edit_image_default_sizes()),
        ("quality", ai_edit_image_default_qualities()),
    ] {
        if let Some(v) = pick(key, table) {
            base[key] = v;
        }
    }
    spread3(
        base,
        props,
        json!({ "stepType": "aiEditImage", "states": [success_state_default_fn(), error_state_default_fn()] }),
    )
}
