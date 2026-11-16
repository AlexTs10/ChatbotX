//! Port of the catalog tests of `packages/ai/__tests__/model-catalog.test.ts`
//! (Q.D6): the zod enums become the id lists of `chatbotx_business::ai::models`.
//! Not applicable (worker AI runtime, unit Q.W2): "normalizes legacy provider
//! model ids before runtime calls", "normalizes every safely migratable
//! persisted model id", "creates an image model for every newly exposed OpenAI
//! image id", "creates a Muse image model through the openai-compatible
//! imageModel()" (`server/factory.ts` builds SDK models for a generation call).

use chatbotx_business::ai::models::*;

type Options = &'static [(&'static str, &'static str)];

fn values(options: &[Options]) -> Vec<&'static str> {
    options
        .iter()
        .flat_map(|o| o.iter().map(|(_, value)| *value))
        .collect()
}

/// Port of: packages/ai/__tests__/model-catalog.test.ts:36 "AI model catalog > keeps provider defaults valid"
#[test]
fn ai_model_catalog_keeps_provider_defaults_valid() {
    for (provider, _, default_model) in AI_CHAT_PROVIDERS {
        let models = chat_models(provider).expect("provider");
        assert!(
            models.contains(default_model),
            "{provider}: {default_model}"
        );
    }
}

/// Port of: packages/ai/__tests__/model-catalog.test.ts:54 "AI model catalog > keeps option values valid"
#[test]
fn ai_model_catalog_keeps_option_values_valid() {
    let groups: &[(Options, &[&str])] = &[
        (OPENAI_MODEL_OPTIONS, OPENAI_MODELS),
        (OPENAI_ANALYZE_IMAGE_MODEL_OPTIONS, OPENAI_MODELS),
        (GEMINI_MODEL_OPTIONS, GEMINI_MODELS),
        (GEMINI_ANALYZE_IMAGE_MODEL_OPTIONS, GEMINI_MODELS),
        (CLAUDE_MODEL_OPTIONS, CLAUDE_MODELS),
        (CLAUDE_ANALYZE_IMAGE_MODEL_OPTIONS, CLAUDE_MODELS),
        (MUSE_MODEL_OPTIONS, MUSE_MODELS),
        (MUSE_ANALYZE_IMAGE_MODEL_OPTIONS, MUSE_MODELS),
        (MUSE_IMAGE_MODEL_OPTIONS, MUSE_IMAGE_MODELS),
        (OPENROUTER_MODEL_OPTIONS, OPENROUTER_MODELS),
        (OPENROUTER_ANALYZE_IMAGE_MODEL_OPTIONS, OPENROUTER_MODELS),
        (OPENROUTER_EXTRACT_FILE_MODEL_OPTIONS, OPENROUTER_MODELS),
        (OPENAI_IMAGE_MODEL_OPTIONS, OPENAI_IMAGE_MODELS),
    ];
    for (options, models) in groups {
        for (_, value) in *options {
            assert!(models.contains(value), "{value}");
        }
    }
}

/// Port of: packages/ai/__tests__/model-catalog.test.ts:78 "AI model catalog > does not accept retired or malformed native Claude model ids"
#[test]
fn ai_model_catalog_does_not_accept_retired_or_malformed_native_claude_model_ids() {
    let exposed = values(&[CLAUDE_MODEL_OPTIONS, CLAUDE_ANALYZE_IMAGE_MODEL_OPTIONS]);
    for id in [
        "claude-3-5-sonnet-20241022",
        "claude-3-5-haiku-20241022",
        "claude-3-opus-20240229",
        "claude-3-sonnet-20240229",
        "claude-3-haiku-20240307",
        "claude-opus-4.6",
        "claude-4.5-haiku-20251001",
        "claude-4.5-sonnet-20250929",
        "claude-4.5-opus-20251101",
    ] {
        assert!(!CLAUDE_MODELS.contains(&id), "{id}");
        assert!(!exposed.contains(&id), "{id}");
    }
}

/// Port of: packages/ai/__tests__/model-catalog.test.ts:102 "AI model catalog > does not expose known stale model ids in OpenRouter pickers"
#[test]
fn ai_model_catalog_does_not_expose_known_stale_model_ids_in_open_router_pickers() {
    let exposed = values(&[
        OPENROUTER_MODEL_OPTIONS,
        OPENROUTER_ANALYZE_IMAGE_MODEL_OPTIONS,
        OPENROUTER_EXTRACT_FILE_MODEL_OPTIONS,
    ]);
    for id in [
        "anthropic/claude-3-5-sonnet",
        "anthropic/claude-3-5-haiku",
        "google/gemini-2.0-flash",
        "meta-llama/llama-3.2-90b-vision-instruct",
    ] {
        assert!(!exposed.contains(&id), "{id}");
    }
}

/// Port of: packages/ai/__tests__/model-catalog.test.ts:151 "AI model catalog > keeps newly exposed vision models capability-safe"
#[test]
fn ai_model_catalog_keeps_newly_exposed_vision_models_capability_safe() {
    let openai = values(&[OPENAI_ANALYZE_IMAGE_MODEL_OPTIONS]);
    let gemini = values(&[GEMINI_ANALYZE_IMAGE_MODEL_OPTIONS]);
    for id in ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"] {
        assert!(openai.contains(&id), "{id}");
    }
    for id in ["gemini-3.8-flash", "gemini-3.7-flash", "gemini-3.6-flash"] {
        assert!(gemini.contains(&id), "{id}");
    }
}
