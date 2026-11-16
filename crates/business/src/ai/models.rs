//! `packages/ai/src/models/*.ts`: the model id enums and picker options of each
//! provider (generated from the TypeScript sources; keep in sync).

/// `claudeModels` (claude.ts).
pub const CLAUDE_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-sonnet-5",
    "claude-fable-5",
    "claude-opus-4-8",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-haiku-4-5-20251001",
    "claude-sonnet-4-5-20250929",
    "claude-opus-4-5-20251101",
];

/// `claudeAnalyzeImageModelOptions` (claude.ts): `(label, value)`.
pub const CLAUDE_ANALYZE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Claude Fable 5.1", "claude-fable-5-1"),
    ("Claude Opus 5.5", "claude-opus-5-5"),
    ("Claude Sonnet 5", "claude-sonnet-5"),
    ("Claude Fable 5", "claude-fable-5"),
    ("Claude Opus 4.8", "claude-opus-4-8"),
    ("Claude Opus 4.6", "claude-opus-4-6"),
    ("Claude Sonnet 4.6", "claude-sonnet-4-6"),
    ("Claude Haiku 4.5", "claude-haiku-4-5-20251001"),
];

/// `claudeModelOptions` (claude.ts): `(label, value)`.
pub const CLAUDE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Claude Fable 5.1", "claude-fable-5-1"),
    ("Claude Opus 5.5", "claude-opus-5-5"),
    ("Claude Sonnet 5", "claude-sonnet-5"),
    ("Claude Fable 5", "claude-fable-5"),
    ("Claude Opus 4.8", "claude-opus-4-8"),
    ("Claude Opus 4.6", "claude-opus-4-6"),
    ("Claude Sonnet 4.6", "claude-sonnet-4-6"),
    ("Claude Sonnet 4.5", "claude-sonnet-4-5-20250929"),
    ("Claude Haiku 4.5", "claude-haiku-4-5-20251001"),
];

/// `deepseekModels` (deepseek.ts).
pub const DEEPSEEK_MODELS: &[&str] = &["deepseek-flash", "deepseek-v4-flash", "deepseek-v4-pro"];

/// `deepseekModelOptions` (deepseek.ts): `(label, value)`.
pub const DEEPSEEK_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("DeepSeek Flash", "deepseek-flash"),
    ("DeepSeek V4 Pro", "deepseek-v4-pro"),
];

/// `geminiEmbeddingModels` (gemini.ts).
pub const GEMINI_EMBEDDING_MODELS: &[&str] = &["gemini-embedding-001"];

/// `geminiModels` (gemini.ts).
pub const GEMINI_MODELS: &[&str] = &[
    "gemini-3.8-flash",
    "gemini-3.7-flash",
    "gemini-3.6-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.5-flash",
    "gemini-3.1-flash-lite",
    "gemini-3.1-pro-preview",
];

/// `geminiImageModels` (gemini.ts).
pub const GEMINI_IMAGE_MODELS: &[&str] = &[
    "gemini-3.1-flash-lite-image",
    "gemini-3-pro-image",
    "gemini-3.1-flash-image",
];

/// `geminiAnalyzeImageModelOptions` (gemini.ts): `(label, value)`.
pub const GEMINI_ANALYZE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Gemini 3.8 Flash", "gemini-3.8-flash"),
    ("Gemini 3.7 Flash", "gemini-3.7-flash"),
    ("Gemini 3.6 Flash", "gemini-3.6-flash"),
    ("Gemini 3.5 Flash Lite", "gemini-3.5-flash-lite"),
    ("Gemini 3.5 Flash", "gemini-3.5-flash"),
    ("Gemini 3.1 Flash Lite", "gemini-3.1-flash-lite"),
    ("Gemini 3.1 Pro Preview", "gemini-3.1-pro-preview"),
];

/// `geminiModelOptions` (gemini.ts): `(label, value)`.
pub const GEMINI_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Gemini 3.8 Flash", "gemini-3.8-flash"),
    ("Gemini 3.7 Flash", "gemini-3.7-flash"),
    ("Gemini 3.6 Flash", "gemini-3.6-flash"),
    ("Gemini 3.5 Flash Lite", "gemini-3.5-flash-lite"),
    ("Gemini 3.5 Flash", "gemini-3.5-flash"),
    ("Gemini 3.1 Pro Preview", "gemini-3.1-pro-preview"),
    ("Gemini 3.1 Flash Lite", "gemini-3.1-flash-lite"),
];

/// `geminiImageModelOptions` (gemini.ts): `(label, value)`.
pub const GEMINI_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Gemini 3.1 Flash Lite Image", "gemini-3.1-flash-lite-image"),
    ("Gemini 3 Pro Image", "gemini-3-pro-image"),
    ("Gemini 3.1 Flash Image", "gemini-3.1-flash-image"),
];

/// `museModels` (muse.ts).
pub const MUSE_MODELS: &[&str] = &["muse-spark-1.3", "muse-spark-1.2", "muse-wonder-1.0"];

/// `museImageModels` (muse.ts).
pub const MUSE_IMAGE_MODELS: &[&str] = &["muse-image-1.0"];

/// `museModelOptions` (muse.ts): `(label, value)`.
pub const MUSE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Muse Spark 1.3", "muse-spark-1.3"),
    ("Muse Spark 1.2", "muse-spark-1.2"),
    ("Muse Wonder 1.0 (Preview)", "muse-wonder-1.0"),
];

/// `museAnalyzeImageModelOptions` (muse.ts): `(label, value)`.
pub const MUSE_ANALYZE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("Muse Spark 1.3", "muse-spark-1.3"),
    ("Muse Spark 1.2", "muse-spark-1.2"),
];

/// `museImageModelOptions` (muse.ts): `(label, value)`.
pub const MUSE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[("Muse Image 1.0", "muse-image-1.0")];

/// `openaiEmbeddingModels` (openai.ts).
pub const OPENAI_EMBEDDING_MODELS: &[&str] = &["text-embedding-ada-002", "text-embedding-3-small"];

/// `openaiModels` (openai.ts).
pub const OPENAI_MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-4-turbo",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    "gpt-4.1",
    "gpt-4",
    "gpt-4o-mini",
    "gpt-4o",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5-pro",
    "gpt-5.1",
    "gpt-5.2-pro",
    "gpt-5.2",
    "gpt-5",
    "gpt-5.5",
    "gpt-5.4-pro",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.4-nano",
    "o4-mini",
];

/// `openaiImageModels` (openai.ts).
pub const OPENAI_IMAGE_MODELS: &[&str] = &[
    "gpt-image-2.5-sunburst",
    "gpt-image-2.5-flare",
    "gpt-image-2",
    "gpt-image-1.5",
    "gpt-image-1",
    "gpt-image-1-mini",
];

/// `openAITTSModelTypes` (openai.ts).
pub const OPENAI_TTS_MODEL_TYPES: &[&str] = &["gpt-4o-mini-tts", "tts-1", "tts-1-hd"];

/// `openAITTSVoiceTypes` (openai.ts).
pub const OPENAI_TTS_VOICE_TYPES: &[&str] = &[
    "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer", "verse",
    "marin", "cedar",
];

/// `openaiAnalyzeImageModelOptions` (openai.ts): `(label, value)`.
pub const OPENAI_ANALYZE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-6 Astra", "gpt-6-astra"),
    ("GPT-6 Sol", "gpt-6-sol"),
    ("GPT-6 Luna", "gpt-6-luna"),
    ("GPT-5.5", "gpt-5.5"),
    ("GPT-5.4 Pro", "gpt-5.4-pro"),
    ("GPT-5.4", "gpt-5.4"),
    ("GPT-5.4 mini", "gpt-5.4-mini"),
    ("GPT-5.4 nano", "gpt-5.4-nano"),
    ("GPT-5.2", "gpt-5.2"),
    ("GPT-5.1", "gpt-5.1"),
    ("GPT-5 mini", "gpt-5-mini"),
    ("GPT-4.1 mini", "gpt-4.1-mini"),
    ("GPT-4.1", "gpt-4.1"),
    ("GPT-4o mini", "gpt-4o-mini"),
    ("GPT-4o", "gpt-4o"),
    ("o4-mini", "o4-mini"),
];

/// `openaiModelOptions` (openai.ts): `(label, value)`.
pub const OPENAI_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-6 Astra", "gpt-6-astra"),
    ("GPT-6 Sol", "gpt-6-sol"),
    ("GPT-6 Luna", "gpt-6-luna"),
    ("GPT-5.5", "gpt-5.5"),
    ("GPT-5.4 Pro", "gpt-5.4-pro"),
    ("GPT-5.4", "gpt-5.4"),
    ("GPT-5.4 Mini", "gpt-5.4-mini"),
    ("GPT-5.4 Nano", "gpt-5.4-nano"),
    ("GPT-5.2 Pro", "gpt-5.2-pro"),
    ("GPT-5.2", "gpt-5.2"),
    ("GPT-5.1", "gpt-5.1"),
    ("GPT-5 Pro", "gpt-5-pro"),
    ("GPT-5", "gpt-5"),
    ("GPT-5 Mini", "gpt-5-mini"),
    ("GPT-5 Nano", "gpt-5-nano"),
    ("GPT-4.1", "gpt-4.1"),
    ("GPT-4.1 Mini", "gpt-4.1-mini"),
    ("GPT-4.1 Nano", "gpt-4.1-nano"),
    ("GPT-4o", "gpt-4o"),
    ("GPT-4o Mini", "gpt-4o-mini"),
    ("GPT-4", "gpt-4"),
    ("GPT-4 Turbo", "gpt-4-turbo"),
];

/// `openaiImageModelOptions` (openai.ts): `(label, value)`.
pub const OPENAI_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT Image 2.5 Sunburst", "gpt-image-2.5-sunburst"),
    ("GPT Image 2.5 Flare", "gpt-image-2.5-flare"),
    ("GPT Image 2", "gpt-image-2"),
    ("GPT Image 1.5", "gpt-image-1.5"),
    ("GPT Image 1", "gpt-image-1"),
    ("GPT Image 1 Mini", "gpt-image-1-mini"),
];

/// `openAITTSModelOptions` (openai.ts): `(label, value)`.
pub const OPENAI_TTS_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-4o mini TTS", "gpt-4o-mini-tts"),
    ("TTS-1", "tts-1"),
    ("TTS-1 HD", "tts-1-hd"),
];

/// `openrouterModels` (openrouter.ts).
pub const OPENROUTER_MODELS: &[&str] = &[
    "openai/gpt-6-astra",
    "openai/gpt-6-sol",
    "openai/gpt-6-luna",
    "openai/gpt-5.5-pro",
    "openai/gpt-5.5",
    "openai/gpt-5.4-pro",
    "openai/gpt-5.4",
    "openai/gpt-5.4-mini",
    "openai/gpt-5.4-nano",
    "openai/gpt-5.2-pro",
    "openai/gpt-5.2-chat",
    "openai/gpt-5.2",
    "openai/gpt-5.1",
    "openai/gpt-5-pro",
    "openai/gpt-5",
    "openai/gpt-5-mini",
    "openai/gpt-5-nano",
    "openai/gpt-4.1",
    "openai/gpt-4.1-mini",
    "openai/gpt-4.1-nano",
    "openai/gpt-4o",
    "openai/gpt-4o-mini",
    "anthropic/claude-fable-5",
    "anthropic/claude-fable-5.1",
    "anthropic/claude-opus-5.5",
    "anthropic/claude-sonnet-5",
    "anthropic/claude-opus-4.8",
    "anthropic/claude-opus-4.6",
    "anthropic/claude-sonnet-4.6",
    "anthropic/claude-opus-4.5",
    "anthropic/claude-sonnet-4.5",
    "anthropic/claude-haiku-4.5",
    "google/gemini-3.5-flash",
    "google/gemini-3.8-flash",
    "google/gemini-3.7-flash",
    "google/gemini-3.6-flash",
    "google/gemini-3.5-flash-lite",
    "google/gemini-3.1-pro-preview",
    "google/gemini-3.1-flash-lite",
    "google/gemini-3-flash-preview",
    "google/gemini-2.5-flash-lite",
    "google/gemini-2.5-flash",
    "google/gemini-2.5-pro",
    "meta-llama/llama-4-maverick",
    "meta-llama/llama-4-scout",
    "meta-llama/llama-3.3-70b-instruct",
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-flash",
    "deepseek/deepseek-v4-flash",
    "deepseek/deepseek-v3.2",
    "deepseek/deepseek-r1",
    "deepseek/deepseek-chat",
    "qwen/qwen3-max",
    "qwen/qwen3.7-max",
    "qwen/qwen3.7-plus",
    "qwen/qwen3-coder-plus",
    "qwen/qwen3-vl-32b-instruct",
    "qwen/qwen-2.5-72b-instruct",
    "mistralai/mistral-large-2512",
    "mistralai/mistral-medium-3-5",
    "mistralai/mistral-large",
    "x-ai/grok-4.20",
    "x-ai/grok-4.3",
];

/// `openrouterModelOptions` (openrouter.ts): `(label, value)`.
pub const OPENROUTER_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-6 Astra", "openai/gpt-6-astra"),
    ("GPT-6 Sol", "openai/gpt-6-sol"),
    ("GPT-6 Luna", "openai/gpt-6-luna"),
    ("GPT-5.5 Pro", "openai/gpt-5.5-pro"),
    ("GPT-5.5", "openai/gpt-5.5"),
    ("GPT-5.4 Pro", "openai/gpt-5.4-pro"),
    ("GPT-5.4", "openai/gpt-5.4"),
    ("GPT-5.4 Mini", "openai/gpt-5.4-mini"),
    ("GPT-5.2 Pro", "openai/gpt-5.2-pro"),
    ("GPT-5.2", "openai/gpt-5.2"),
    ("GPT-5.1", "openai/gpt-5.1"),
    ("GPT-5", "openai/gpt-5"),
    ("GPT-5 Mini", "openai/gpt-5-mini"),
    ("GPT-4o", "openai/gpt-4o"),
    ("GPT-4o Mini", "openai/gpt-4o-mini"),
    ("Claude Fable 5", "anthropic/claude-fable-5"),
    ("Claude Fable 5.1", "anthropic/claude-fable-5.1"),
    ("Claude Opus 5.5", "anthropic/claude-opus-5.5"),
    ("Claude Sonnet 5", "anthropic/claude-sonnet-5"),
    ("Claude Opus 4.8", "anthropic/claude-opus-4.8"),
    ("Claude Opus 4.6", "anthropic/claude-opus-4.6"),
    ("Claude Sonnet 4.6", "anthropic/claude-sonnet-4.6"),
    ("Claude Sonnet 4.5", "anthropic/claude-sonnet-4.5"),
    ("Claude Haiku 4.5", "anthropic/claude-haiku-4.5"),
    ("Gemini 3.5 Flash", "google/gemini-3.5-flash"),
    ("Gemini 3.8 Flash", "google/gemini-3.8-flash"),
    ("Gemini 3.7 Flash", "google/gemini-3.7-flash"),
    ("Gemini 3.6 Flash", "google/gemini-3.6-flash"),
    ("Gemini 3.5 Flash Lite", "google/gemini-3.5-flash-lite"),
    ("Gemini 3.1 Pro Preview", "google/gemini-3.1-pro-preview"),
    ("Gemini 3.1 Flash Lite", "google/gemini-3.1-flash-lite"),
    ("Gemini 2.5 Flash", "google/gemini-2.5-flash"),
    ("Gemini 2.5 Flash Lite", "google/gemini-2.5-flash-lite"),
    ("Gemini 2.5 Pro", "google/gemini-2.5-pro"),
    ("Llama 4 Maverick", "meta-llama/llama-4-maverick"),
    ("Llama 4 Scout", "meta-llama/llama-4-scout"),
    ("Llama 3.3 70B", "meta-llama/llama-3.3-70b-instruct"),
    ("DeepSeek V4 Pro", "deepseek/deepseek-v4-pro"),
    ("DeepSeek Flash", "deepseek/deepseek-flash"),
    ("DeepSeek V4 Flash", "deepseek/deepseek-v4-flash"),
    ("DeepSeek V3.2", "deepseek/deepseek-v3.2"),
    ("DeepSeek R1", "deepseek/deepseek-r1"),
    ("Qwen3 Max", "qwen/qwen3-max"),
    ("Qwen3.7 Max", "qwen/qwen3.7-max"),
    ("Qwen3 Coder Plus", "qwen/qwen3-coder-plus"),
    ("Mistral Large 3", "mistralai/mistral-large-2512"),
    ("Mistral Medium 3.5", "mistralai/mistral-medium-3-5"),
    ("Grok 4.20", "x-ai/grok-4.20"),
    ("Grok 4.3", "x-ai/grok-4.3"),
];

/// `openrouterAnalyzeImageModelOptions` (openrouter.ts): `(label, value)`.
pub const OPENROUTER_ANALYZE_IMAGE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-6 Astra", "openai/gpt-6-astra"),
    ("GPT-6 Sol", "openai/gpt-6-sol"),
    ("GPT-6 Luna", "openai/gpt-6-luna"),
    ("GPT-5.5 Pro", "openai/gpt-5.5-pro"),
    ("GPT-5.5", "openai/gpt-5.5"),
    ("GPT-5.4 Pro", "openai/gpt-5.4-pro"),
    ("GPT-5.4", "openai/gpt-5.4"),
    ("GPT-5.4 Mini", "openai/gpt-5.4-mini"),
    ("GPT-5.2 Pro", "openai/gpt-5.2-pro"),
    ("GPT-5.2", "openai/gpt-5.2"),
    ("GPT-5.1", "openai/gpt-5.1"),
    ("GPT-5", "openai/gpt-5"),
    ("GPT-4.1", "openai/gpt-4.1"),
    ("GPT-4o", "openai/gpt-4o"),
    ("GPT-4o Mini", "openai/gpt-4o-mini"),
    ("Claude Fable 5", "anthropic/claude-fable-5"),
    ("Claude Fable 5.1", "anthropic/claude-fable-5.1"),
    ("Claude Opus 5.5", "anthropic/claude-opus-5.5"),
    ("Claude Sonnet 5", "anthropic/claude-sonnet-5"),
    ("Claude Opus 4.8", "anthropic/claude-opus-4.8"),
    ("Claude Opus 4.6", "anthropic/claude-opus-4.6"),
    ("Claude Sonnet 4.6", "anthropic/claude-sonnet-4.6"),
    ("Claude Sonnet 4.5", "anthropic/claude-sonnet-4.5"),
    ("Claude Haiku 4.5", "anthropic/claude-haiku-4.5"),
    ("Gemini 3.5 Flash", "google/gemini-3.5-flash"),
    ("Gemini 3.8 Flash", "google/gemini-3.8-flash"),
    ("Gemini 3.7 Flash", "google/gemini-3.7-flash"),
    ("Gemini 3.6 Flash", "google/gemini-3.6-flash"),
    ("Gemini 3.5 Flash Lite", "google/gemini-3.5-flash-lite"),
    ("Gemini 3.1 Pro Preview", "google/gemini-3.1-pro-preview"),
    ("Gemini 3.1 Flash Lite", "google/gemini-3.1-flash-lite"),
    ("Gemini 2.5 Flash", "google/gemini-2.5-flash"),
    ("Gemini 2.5 Pro", "google/gemini-2.5-pro"),
    ("Qwen3 VL 32B", "qwen/qwen3-vl-32b-instruct"),
];

/// `openrouterExtractFileModelOptions` (openrouter.ts): `(label, value)`.
pub const OPENROUTER_EXTRACT_FILE_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("GPT-6 Astra", "openai/gpt-6-astra"),
    ("GPT-6 Sol", "openai/gpt-6-sol"),
    ("GPT-6 Luna", "openai/gpt-6-luna"),
    ("GPT-5.5 Pro", "openai/gpt-5.5-pro"),
    ("GPT-5.5", "openai/gpt-5.5"),
    ("GPT-5.4 Pro", "openai/gpt-5.4-pro"),
    ("GPT-5.4", "openai/gpt-5.4"),
    ("GPT-5.4 Mini", "openai/gpt-5.4-mini"),
    ("GPT-5.2 Pro", "openai/gpt-5.2-pro"),
    ("GPT-5.2", "openai/gpt-5.2"),
    ("GPT-5.1", "openai/gpt-5.1"),
    ("GPT-5", "openai/gpt-5"),
    ("GPT-4.1", "openai/gpt-4.1"),
    ("GPT-4o", "openai/gpt-4o"),
    ("GPT-4o Mini", "openai/gpt-4o-mini"),
    ("Claude Fable 5", "anthropic/claude-fable-5"),
    ("Claude Fable 5.1", "anthropic/claude-fable-5.1"),
    ("Claude Opus 5.5", "anthropic/claude-opus-5.5"),
    ("Claude Sonnet 5", "anthropic/claude-sonnet-5"),
    ("Claude Opus 4.8", "anthropic/claude-opus-4.8"),
    ("Claude Opus 4.6", "anthropic/claude-opus-4.6"),
    ("Claude Sonnet 4.6", "anthropic/claude-sonnet-4.6"),
    ("Claude Sonnet 4.5", "anthropic/claude-sonnet-4.5"),
    ("Claude Haiku 4.5", "anthropic/claude-haiku-4.5"),
    ("Gemini 3.5 Flash", "google/gemini-3.5-flash"),
    ("Gemini 3.8 Flash", "google/gemini-3.8-flash"),
    ("Gemini 3.7 Flash", "google/gemini-3.7-flash"),
    ("Gemini 3.6 Flash", "google/gemini-3.6-flash"),
    ("Gemini 3.5 Flash Lite", "google/gemini-3.5-flash-lite"),
    ("Gemini 3.1 Pro Preview", "google/gemini-3.1-pro-preview"),
    ("Gemini 3.1 Flash Lite", "google/gemini-3.1-flash-lite"),
    ("Gemini 2.5 Flash", "google/gemini-2.5-flash"),
    ("Gemini 2.5 Flash Lite", "google/gemini-2.5-flash-lite"),
    ("Gemini 2.5 Pro", "google/gemini-2.5-pro"),
];

/// `aiProviders` (`schemas/ai-model.ts`).
pub const AI_PROVIDERS: &[&str] = &[
    "openai",
    "gemini",
    "claude",
    "deepseek",
    "openrouter",
    "muse",
];

/// The chat model ids of `provider` (`<provider>Models`).
pub fn chat_models(provider: &str) -> Option<&'static [&'static str]> {
    Some(match provider {
        "openai" => OPENAI_MODELS,
        "gemini" => GEMINI_MODELS,
        "claude" => CLAUDE_MODELS,
        "deepseek" => DEEPSEEK_MODELS,
        "openrouter" => OPENROUTER_MODELS,
        "muse" => MUSE_MODELS,
        _ => return None,
    })
}

/// `aiChatProviders` (`models/registry.ts`): provider, picker options,
/// default model, in fallback order.
/// `(provider, picker options, default model)`.
pub type ChatProvider = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

/// `aiChatProviders` list.
pub const AI_CHAT_PROVIDERS: &[ChatProvider] = &[
    ("gemini", GEMINI_MODEL_OPTIONS, "gemini-3.5-flash"),
    ("openai", OPENAI_MODEL_OPTIONS, "gpt-5.4-mini"),
    ("claude", CLAUDE_MODEL_OPTIONS, "claude-sonnet-4-6"),
    ("deepseek", DEEPSEEK_MODEL_OPTIONS, "deepseek-flash"),
    (
        "openrouter",
        OPENROUTER_MODEL_OPTIONS,
        "openai/gpt-5.4-mini",
    ),
    ("muse", MUSE_MODEL_OPTIONS, "muse-spark-1.3"),
];
