//! Port of `packages/flow-config/src/steps/ai-text-to-speech.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn ai_text_to_speech_model_types = z::enum_(&["gpt-4o-mini-tts", "tts-1", "tts-1-hd"]));

schema!(pub fn ai_text_to_speech_voice_types = z::enum_(&["alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer", "verse", "marin", "cedar"]));

schema!(pub fn ai_text_to_speech_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiTextToSpeech")).describe("Step type discriminator: \"aiTextToSpeech\".")),
("provider", z::literal(json!("openai"))),
("model", ai_text_to_speech_model_types()),
("message", z::string().trim().min(1)),
("voiceType", ai_text_to_speech_voice_types()),
("voiceTone", z::string().trim().optional()),
("outputFieldId", z::string().trim().min(1).refine(ai_text_to_speech_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]).optional())]));

pub const AI_TEXT_TO_SPEECH_BASE64_ENCODING: &str = "base64";

pub const AI_TEXT_TO_SPEECH_DEFAULT_MIME_TYPE: &str = "audio/mpeg";

pub const AI_TEXT_TO_SPEECH_DEFAULT_EXTENSION: &str = "mp3";

pub const AI_TEXT_TO_SPEECH_MESSAGE_MAX_LENGTH: usize = 4096;

// HAND-PORT getAITextToSpeechAudioPath:
// ({storagePrefix,fileName})=>`${storagePrefix}/${fileName}`

// HAND-PORT AITextToSpeechDefaultFn:
// props=>({id:createId(),provider:"openai",model:aiTextToSpeechModelTypes.enum["gpt-4o-mini-tts"],message:"",voiceType:aiTextToSpeechVoiceTypes.enum.alloy,voiceTone:"",outputFieldId:"",...props,stepType:stepTypes.enum.aiTextToSpeech,states:[successStateDefaultFn(),errorStateDefaultFn()]})

// value=>!value.startsWith(RESERVED_FIELD_REFERENCE_PREFIX)||BOT_FIELD_REFERENCE_PATTERN.test(value)
fn ai_text_to_speech_schema_output_field_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn ai_text_to_speech_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
