//! Port of `packages/flow-config/__tests__/openai-compatible-ai-steps.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn openai_compatible_ai_flow_steps_keeps_native_generate_text_parsing_unchanged() {
    let step = fc::ai_generate_text_default_fn(&json!({
        "model": "gpt-5.4-mini",
        "outputFieldId": "field-1",
        "text": "Summarize this",
    }));

    assert!(fc::ai_generate_text_schema().safe_parse(&step).success);
}

#[test]
fn openai_compatible_ai_flow_steps_accepts_openai_compatible_generate_text_with_integration_and_model()
 {
    let step = fc::ai_generate_text_default_fn(&json!({
        "provider": "openaiCompatible",
        "integrationId": "integration-1",
        "model": "local-model",
        "outputFieldId": "field-1",
        "text": "Summarize this",
    }));

    assert!(fc::ai_generate_text_schema().safe_parse(&step).success);
}

#[test]
fn openai_compatible_ai_flow_steps_rejects_openai_compatible_generate_text_without_integration_or_model()
 {
    let step = fc::ai_generate_text_default_fn(&json!({
        "provider": "openaiCompatible",
        "outputFieldId": "field-1",
        "text": "Summarize this",
    }));

    assert!(!fc::ai_generate_text_schema().safe_parse(&step).success);
}

#[test]
fn openai_compatible_ai_flow_steps_creates_openai_compatible_draft_defaults_without_native_model_lookup()
 {
    assert_match_object(
        &fc::ai_generate_text_default_fn(&json!({ "provider": "openaiCompatible" })),
        &json!({ "provider": "openaiCompatible", "integrationId": "", "model": "" }),
    );
}

#[test]
fn openai_compatible_ai_flow_steps_accepts_openai_compatible_analyze_image_and_extract_data_configs()
 {
    assert!(
        fc::ai_analyze_image_schema()
            .safe_parse(&fc::ai_analyze_image_default_fn(&json!({
                "provider": "openaiCompatible",
                "integrationId": "integration-1",
                "model": "vision-model",
                "inputFieldId": "image-field",
                "outputFieldId": "output-field",
                "prompt": "Describe this image",
            })))
            .success
    );

    assert!(
        fc::ai_extract_data_schema()
            .safe_parse(&fc::ai_extract_data_default_fn(&json!({
                "provider": "openaiCompatible",
                "integrationId": "integration-1",
                "model": "structured-model",
                "inputFieldId": "input-field",
                "extractFields": [{ "key": "email", "customFieldId": "email-field" }],
            })))
            .success
    );
}

#[test]
fn openai_compatible_ai_flow_steps_rejects_openai_compatible_analyze_image_and_extract_data_without_integration()
 {
    assert!(
        !fc::ai_analyze_image_schema()
            .safe_parse(&fc::ai_analyze_image_default_fn(&json!({
                "provider": "openaiCompatible",
                "model": "vision-model",
                "inputFieldId": "image-field",
                "outputFieldId": "output-field",
                "prompt": "Describe this image",
            })))
            .success
    );

    assert!(
        !fc::ai_extract_data_schema()
            .safe_parse(&fc::ai_extract_data_default_fn(&json!({
                "provider": "openaiCompatible",
                "model": "structured-model",
                "inputFieldId": "input-field",
                "extractFields": [{ "key": "email", "customFieldId": "email-field" }],
            })))
            .success
    );
}

#[test]
fn openai_compatible_ai_flow_steps_accepts_openai_compatible_generate_text_agent_with_integration_and_model()
 {
    let step = fc::ai_generate_text_agent_default_fn(&json!({
        "provider": "openaiCompatible",
        "integrationId": "integration-1",
        "model": "agent-model",
        "aiAgentId": "1",
        "message": "Reply from this message",
        "outputFieldId": "output-field",
    }));

    assert!(
        fc::ai_generate_text_agent_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn openai_compatible_ai_flow_steps_rejects_openai_compatible_generate_text_agent_without_integration_or_model()
 {
    let step = fc::ai_generate_text_agent_default_fn(&json!({
        "provider": "openaiCompatible",
        "aiAgentId": "1",
        "message": "Reply from this message",
        "outputFieldId": "output-field",
    }));

    assert!(
        !fc::ai_generate_text_agent_schema()
            .safe_parse(&step)
            .success
    );
}

#[test]
fn openai_compatible_ai_flow_steps_accepts_openai_compatible_delete_message_history_without_model_config()
 {
    let step = fc::ai_delete_message_history_default_fn(&json!({ "provider": "openaiCompatible" }));

    assert!(
        fc::ai_delete_message_history_schema()
            .safe_parse(&step)
            .success
    );
}
