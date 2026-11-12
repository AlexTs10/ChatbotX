//! Port of `packages/flow-config/__tests__/ai-model-catalog.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn models(provider: &Value) -> Vec<&str> {
    provider["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect()
}

#[test]
fn ai_extract_data_model_catalog_keeps_defaults_inside_provider_model_lists() {
    for provider in fc::ai_extract_data_models().as_object().unwrap().values() {
        assert!(models(provider).contains(&provider["default"].as_str().unwrap()));
    }
}

#[test]
fn ai_extract_data_model_catalog_does_not_expose_stale_openrouter_extract_models() {
    let openrouter = models(&fc::ai_extract_data_models()["openrouter"]);
    assert!(!openrouter.contains(&"anthropic/claude-3-5-sonnet"));
    assert!(!openrouter.contains(&"anthropic/claude-3-5-haiku"));
    assert!(!openrouter.contains(&"google/gemini-2.0-flash"));
}

#[test]
fn ai_extract_data_model_catalog_keeps_published_legacy_models_and_canonical_defaults_valid() {
    assert_eq!(
        fc::ai_generate_text_default_fn(&json!({ "provider": "deepseek" }))["model"],
        "deepseek-flash"
    );
    assert_eq!(
        fc::ai_generate_text_default_fn(&json!({ "model": "deepseek-v4-flash" }))["model"],
        "deepseek-v4-flash"
    );
    assert_eq!(
        fc::ai_extract_data_default_fn(&json!({ "provider": "openai" }))["model"],
        fc::ai_extract_data_models()["openai"]["default"]
    );
    assert_eq!(
        fc::ai_analyze_image_default_fn(&json!({ "provider": "openai" }))["model"],
        "gpt-5.4-mini"
    );
    assert_eq!(
        fc::ai_speech_to_text_default_fn(&json!({ "model": "whisper-1" }))["model"],
        "whisper-1"
    );
}

#[test]
fn ai_extract_data_model_catalog_defaults_muse_steps_to_muse_spark_1_3() {
    assert_eq!(
        fc::ai_generate_text_default_fn(&json!({ "provider": "muse" }))["model"],
        "muse-spark-1.3"
    );
    assert_eq!(
        fc::ai_extract_data_default_fn(&json!({ "provider": "muse" }))["model"],
        "muse-spark-1.3"
    );
    assert_eq!(
        fc::ai_analyze_image_default_fn(&json!({ "provider": "muse" }))["model"],
        "muse-spark-1.3"
    );
}
