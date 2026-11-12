//! Port of `packages/flow-config/src/steps/ai-speech-to-text.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_speech_to_text_provider_schema = z::enum_(&["openai", "muse"]));

pub const MUSE_TRANSCRIPTION_MODEL: &str = "muse-voice-transcribe-1.0";

pub fn default_speech_to_text_models() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"openai":"whisper-1","muse":"muse-voice-transcribe-1.0"}"#)
            .expect("valid JSON")
    });
    &V
}

schema!(pub fn muse_voice_languages = z::enum_(&["Arabic", "Bengali", "Dutch", "English", "French", "German", "Hebrew", "Hindi", "Indonesian", "Italian", "Japanese", "Kannada", "Korean", "Malay", "Mandarin Chinese", "Marathi", "Polish", "Portuguese", "Spanish", "Tagalog", "Tamil", "Telugu", "Thai", "Turkish", "Vietnamese"]));

pub const MUSE_TRANSCRIBE_MAX_KEYWORDS: usize = 50;

pub const MUSE_TRANSCRIBE_MAX_KEYWORD_LENGTH: usize = 100;

schema!(pub fn ai_speech_to_text_open_ai_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiSpeechToText")).describe("Step type discriminator: \"aiSpeechToText\".")),
("inputFieldId", z::string().trim().min(1).describe("Custom field holding the audio file URL.")),
("outputFieldId", z::string().trim().min(1).refine(ai_speech_to_text_open_ai_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None).describe("Custom field that receives the transcript.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional()),
("provider", z::literal(json!("openai"))),
("model", z::string().trim().min(1).describe("OpenAI model, e.g. \"whisper-1\".")),
("languageBias", z::never().optional()),
("keywords", z::never().optional())]).describe("OpenAI transcription. Accepts flac, mp3, mp4, m4a, ogg, wav and webm audio."));

schema!(pub fn ai_speech_to_text_muse_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiSpeechToText")).describe("Step type discriminator: \"aiSpeechToText\".")),
("inputFieldId", z::string().trim().min(1).describe("Custom field holding the audio file URL.")),
("outputFieldId", z::string().trim().min(1).refine(ai_speech_to_text_open_ai_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None).describe("Custom field that receives the transcript.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional()),
("provider", z::literal(json!("muse"))),
("model", z::literal(json!("muse-voice-transcribe-1.0"))),
("languageBias", z::array(muse_voice_languages()).max(25).optional().describe("Languages to bias recognition toward; omit for automatic detection.")),
("keywords", z::array(z::string().trim().min(1).max(100)).max(50).optional().describe("Names or domain terms that guide spelling."))]).describe("Muse Voice Transcribe. Accepts only WAV (RIFF, PCM 16-bit, mono, 16 or 24 kHz), at most 10 minutes and under 32 MiB. Any other audio (ogg/opus voice notes, mp3, stereo, 44.1/48 kHz) goes to the error state; nothing is converted."));

schema!(pub fn ai_speech_to_text_schema = z::discriminated_union("provider", vec![ai_speech_to_text_open_ai_schema(),
ai_speech_to_text_muse_schema()]));

fn ai_speech_to_text_open_ai_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `AISpeechToTextDefaultFn(props)`: Muse-only fields never carry over to
/// an OpenAI step.
pub fn ai_speech_to_text_default_fn(props: &Value) -> Value {
    let base = json!({ "id": create_id(), "inputFieldId": "", "outputFieldId": "" });
    let fixed = json!({
        "stepType": "aiSpeechToText",
        "states": [success_state_default_fn(), error_state_default_fn()],
    });
    let models = default_speech_to_text_models();
    let model_or = |p: &Value, d: &str| {
        p.get("model")
            .filter(|m| !m.is_null())
            .cloned()
            .unwrap_or_else(|| models[d].clone())
    };
    if props["provider"] == "muse" {
        let mut out = spread_props(base, props);
        out["provider"] = json!("muse");
        out["model"] = model_or(props, "muse");
        return spread_props(out, &fixed);
    }
    let mut rest = props.clone();
    if let Some(obj) = rest.as_object_mut() {
        obj.remove("keywords");
        obj.remove("languageBias");
    }
    let mut out = spread_props(base, &rest);
    out["provider"] = json!("openai");
    out["model"] = model_or(&rest, "openai");
    spread_props(out, &fixed)
}
