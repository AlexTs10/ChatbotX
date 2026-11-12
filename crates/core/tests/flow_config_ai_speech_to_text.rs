//! Port of `packages/flow-config/__tests__/ai-speech-to-text.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn fields() -> Value {
    json!({ "inputFieldId": "1", "outputFieldId": "2" })
}

fn muse_step(overrides: Value) -> Value {
    spread(
        &fc::ai_speech_to_text_default_fn(&spread(&json!({ "provider": "muse" }), fields())),
        overrides,
    )
}

#[test]
fn aispeechtotext_step_schema_defaults_openai_to_whisper_1() {
    let step = fc::ai_speech_to_text_default_fn(&json!({}));
    assert_eq!(step["provider"], "openai");
    assert_eq!(step["model"], "whisper-1");
}

#[test]
fn aispeechtotext_step_schema_defaults_muse_to_muse_voice_transcribe_1_0() {
    let step = fc::ai_speech_to_text_default_fn(&json!({ "provider": "muse" }));
    assert_eq!(step["provider"], "muse");
    assert_eq!(step["model"], "muse-voice-transcribe-1.0");
}

#[test]
fn aispeechtotext_step_schema_keeps_a_legacy_openai_step_without_muse_fields_valid() {
    let legacy = spread(
        &json!({ "id": "1", "stepType": "aiSpeechToText", "provider": "openai", "model": "whisper-1" }),
        fields(),
    );
    assert!(fc::ai_speech_to_text_schema().safe_parse(&legacy).success);
}

#[test]
fn aispeechtotext_step_schema_accepts_a_muse_step_with_language_bias_and_keywords() {
    let parsed = fc::ai_speech_to_text_schema()
        .parse(&muse_step(
            json!({ "languageBias": ["Vietnamese"], "keywords": ["ChatbotX"] }),
        ))
        .unwrap();
    assert_match_object(
        &parsed,
        &json!({ "provider": "muse", "languageBias": ["Vietnamese"], "keywords": ["ChatbotX"] }),
    );
}

#[test]
fn aispeechtotext_step_schema_rejects_an_unknown_provider() {
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "provider": "gemini" })))
            .success
    );
}

#[test]
fn aispeechtotext_step_schema_rejects_a_muse_step_with_another_model() {
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "model": "whisper-1" })))
            .success
    );
}

#[test]
fn aispeechtotext_step_schema_rejects_muse_only_fields_on_an_openai_step() {
    let openai = fc::ai_speech_to_text_default_fn(&fields());
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&spread(&openai, json!({ "languageBias": ["English"] })))
            .success
    );
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&spread(&openai, json!({ "keywords": ["x"] })))
            .success
    );
}

#[test]
fn aispeechtotext_step_schema_drops_muse_only_fields_when_defaulting_an_openai_step() {
    // `{ provider: undefined, ...fields }`: an undefined key is an absent key.
    let step = fc::ai_speech_to_text_default_fn(&fields());
    assert!(step.get("languageBias").is_none());
    assert!(step.get("keywords").is_none());
}

#[test]
fn aispeechtotext_step_schema_rejects_a_language_outside_the_muse_list() {
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "languageBias": ["Klingon"] })))
            .success
    );
}

#[test]
fn aispeechtotext_step_schema_rejects_empty_keywords_and_oversized_arrays() {
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "keywords": ["  "] })))
            .success
    );
    let keywords: Vec<String> = (0..fc::MUSE_TRANSCRIBE_MAX_KEYWORDS + 1)
        .map(|index| format!("k{index}"))
        .collect();
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "keywords": keywords })))
            .success
    );
    let mut languages = fc::muse_voice_languages().enum_values();
    languages.push("English".to_owned());
    assert!(
        !fc::ai_speech_to_text_schema()
            .safe_parse(&muse_step(json!({ "languageBias": languages })))
            .success
    );
}

#[test]
fn aispeechtotext_step_schema_parses_a_flow_version_holding_a_muse_step_in_both_node_unions() {
    let step = muse_step(json!({ "languageBias": ["Vietnamese"] }));
    let perform_action = fc::perform_action_node_default_fn(
        &json!({ "nodeProps": { "id": "1" }, "detailProps": { "steps": [] } }),
    );
    let send_message =
        fc::send_message_node_default_fn(&json!({ "labelVersion": 1, "nodeProps": { "id": "2" } }));

    let with_steps = |node: &Value| {
        spread(
            node,
            json!({
                "data": spread(&node["data"], json!({ "details": spread(&node["data"]["details"], json!({ "steps": [step] })) })),
            }),
        )
    };
    let parsed = fc::flow_version_schema()
        .array()
        .parse(&json!([
            with_steps(&perform_action),
            with_steps(&send_message)
        ]))
        .unwrap();

    assert_eq!(parsed.as_array().unwrap().len(), 2);
}
