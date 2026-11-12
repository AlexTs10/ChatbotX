//! Port of `packages/flow-config/__tests__/ai-muse-steps.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn muse_ai_flow_steps_parses_every_ai_step_with_the_muse_provider() {
    let steps = [
        fc::ai_generate_text_schema()
            .parse(&fc::ai_generate_text_default_fn(&json!({
                "provider": "muse",
                "text": "Summarize",
                "outputFieldId": "1",
            })))
            .unwrap(),
        fc::ai_analyze_image_schema()
            .parse(&fc::ai_analyze_image_default_fn(&json!({
                "provider": "muse",
                "prompt": "Describe",
                "inputFieldId": "1",
                "outputFieldId": "2",
            })))
            .unwrap(),
        fc::ai_extract_data_schema()
            .parse(&fc::ai_extract_data_default_fn(
                &json!({ "provider": "muse", "inputFieldId": "1" }),
            ))
            .unwrap(),
        fc::ai_delete_message_history_schema()
            .parse(&fc::ai_delete_message_history_default_fn(
                &json!({ "provider": "muse" }),
            ))
            .unwrap(),
    ];

    for step in steps {
        assert_eq!(step["provider"], "muse");
    }
}

#[test]
fn muse_ai_flow_steps_rejects_a_muse_temperature_above_the_muse_maximum() {
    let text = fc::ai_generate_text_default_fn(
        &json!({ "provider": "muse", "text": "Summarize", "outputFieldId": "1" }),
    );
    let image = fc::ai_analyze_image_default_fn(&json!({
        "provider": "muse",
        "prompt": "Describe",
        "inputFieldId": "1",
        "outputFieldId": "2",
    }));

    for result in [
        fc::ai_generate_text_schema().safe_parse(&spread(&text, json!({ "temperature": 1.5 }))),
        fc::ai_analyze_image_schema().safe_parse(&spread(&image, json!({ "temperature": 1.5 }))),
    ] {
        assert!(!result.success);
        assert_eq!(result.issues()[0].path_json(), json!(["temperature"]));
    }
    assert!(
        fc::ai_generate_text_schema()
            .safe_parse(&spread(
                &text,
                json!({ "temperature": fc::MUSE_MAX_TEMPERATURE })
            ))
            .success
    );
    assert!(
        fc::ai_generate_text_schema()
            .safe_parse(&spread(
                &text,
                json!({ "provider": "openai", "model": "gpt-5.4-mini", "temperature": 1.5 })
            ))
            .success
    );
}

#[test]
fn muse_ai_flow_steps_keeps_muse_on_the_agent_step_instead_of_catching_to_openai() {
    let step = fc::ai_generate_text_agent_schema()
        .parse(&fc::ai_generate_text_agent_default_fn(&json!({
            "provider": "muse",
            "aiAgentId": "1",
            "message": "Hello",
            "outputFieldId": "1",
        })))
        .unwrap();

    assert_eq!(step["provider"], "muse");
}

#[test]
fn muse_ai_flow_steps_defaults_the_muse_image_steps_to_muse_image_1_0_without_size_or_quality() {
    let generated = fc::ai_generate_image_schema()
        .parse(&fc::ai_generate_image_default_fn(&json!({
            "provider": "muse",
            "prompt": "A red apple",
            "outputFieldId": "1",
        })))
        .unwrap();
    let edited = fc::ai_edit_image_schema()
        .parse(&fc::ai_edit_image_default_fn(&json!({
            "provider": "muse",
            "inputFieldId": "1",
            "prompt": "Make it blue",
            "outputFieldId": "2",
        })))
        .unwrap();

    let expected =
        json!({ "provider": "muse", "model": "muse-image-1.0", "size": "auto", "quality": "auto" });
    assert_match_object(&generated, &expected);
    assert_match_object(&edited, &expected);
}
