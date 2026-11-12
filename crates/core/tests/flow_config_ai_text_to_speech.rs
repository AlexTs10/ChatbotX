//! Port of `packages/flow-config/__tests__/ai-text-to-speech.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn ai_text_to_speech_flow_contract_keeps_legacy_over_limit_messages_readable() {
    let legacy_step = fc::ai_text_to_speech_default_fn(&json!({
        "message": rep("a", fc::AI_TEXT_TO_SPEECH_MESSAGE_MAX_LENGTH + 1),
        "outputFieldId": "field-1",
    }));

    assert!(
        fc::ai_text_to_speech_schema()
            .safe_parse(&legacy_step)
            .success
    );
}
