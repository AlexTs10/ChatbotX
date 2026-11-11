//! Port of `packages/worker-config/__tests__/heavy-job-schema.test.ts`.
//!
//! The flow-config `*DefaultFn` step builders are the `*_default_step`
//! stand-ins of `chatbotx_jobs::queues::heavy` (no flow-config port yet).

use chatbotx_jobs::queues::heavy::{
    HeavyAnalyzeImageResult, HeavyExtractTextFromFileResult, HeavyJobAction, HeavyJobData,
    HeavyStepResult, ai_edit_image_default_step, ai_generate_image_default_step,
    ai_segment_image_default_step, ai_speech_to_text_default_step, ai_text_to_speech_default_step,
    heavy_completion_wait_timeout_ms, heavy_job_options,
};
use chatbotx_jobs::{Backoff, Retention};
use serde_json::{Value, json};

/// `{ ...stepBaseData, ...extra }`.
fn step_data(extra: Value) -> Value {
    let mut data = json!({
        "conversationId": "conversation-1",
        "contactInboxId": "contact-inbox-1",
    });
    for (k, v) in extra.as_object().expect("object") {
        data[k] = v.clone();
    }
    data
}

fn job(action: HeavyJobAction, data: Value) -> Value {
    json!({ "type": action.as_str(), "data": data })
}

fn assert_round_trip(job_data: Value) {
    let parsed = HeavyJobData::parse(&job_data).expect("payload parses");
    assert_eq!(parsed.to_value(), job_data);
}

fn segment_step(overrides: Value) -> Value {
    let mut base = json!({
        "id": "5",
        "inputFieldId": "custom-field-input",
        "target": "product box",
        "outputFieldId": "custom-field-output",
    });
    for (k, v) in overrides.as_object().expect("object") {
        base[k] = v.clone();
    }
    ai_segment_image_default_step(base)
}

fn generate_step() -> Value {
    ai_generate_image_default_step(json!({
        "id": "2",
        "prompt": "A quiet workspace",
        "outputFieldId": "custom-field-output",
    }))
}

// ---------------------------------------------------------------------------
// heavyJobDataSchema > parses $type payloads (9 rows)
// ---------------------------------------------------------------------------

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses processAIFile payloads"
#[test]
fn parses_payloads_process_ai_file() {
    assert_round_trip(job(
        HeavyJobAction::ProcessAiFile,
        json!({ "aiFileId": "ai-file-1" }),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses processAIFile payloads"
#[test]
fn parses_payloads_process_ai_file_with_run() {
    assert_round_trip(job(
        HeavyJobAction::ProcessAiFile,
        json!({ "aiFileId": "ai-file-1", "runId": "run-1" }),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses aiEditImage payloads"
#[test]
fn parses_payloads_ai_edit_image() {
    let step = ai_edit_image_default_step(json!({
        "id": "1",
        "inputFieldId": "custom-field-input",
        "prompt": "Make it brighter",
        "outputFieldId": "custom-field-output",
    }));
    assert_round_trip(job(
        HeavyJobAction::AiEditImage,
        step_data(json!({ "step": step })),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses aiSegmentImage payloads"
#[test]
fn parses_payloads_ai_segment_image() {
    let step = ai_segment_image_default_step(json!({
        "id": "5",
        "inputFieldId": "custom-field-input",
        "target": "product box",
        "outputFieldId": "custom-field-output",
        "detailsOutputFieldId": "custom-field-details",
    }));
    assert_round_trip(job(
        HeavyJobAction::AiSegmentImage,
        step_data(json!({ "step": step })),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses aiGenerateImage payloads"
#[test]
fn parses_payloads_ai_generate_image() {
    assert_round_trip(job(
        HeavyJobAction::AiGenerateImage,
        step_data(json!({ "step": generate_step() })),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses aiSpeechToText payloads"
#[test]
fn parses_payloads_ai_speech_to_text() {
    let step = ai_speech_to_text_default_step(json!({
        "id": "3",
        "inputFieldId": "custom-field-audio",
        "outputFieldId": "custom-field-output",
    }));
    assert_round_trip(job(
        HeavyJobAction::AiSpeechToText,
        step_data(json!({ "step": step })),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses aiTextToSpeech payloads"
#[test]
fn parses_payloads_ai_text_to_speech() {
    let step = ai_text_to_speech_default_step(json!({
        "id": "4",
        "message": "Xin chao",
        "outputFieldId": "custom-field-output",
    }));
    assert_round_trip(job(
        HeavyJobAction::AiTextToSpeech,
        step_data(json!({ "step": step })),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses extractTextFromFile payloads"
#[test]
fn parses_payloads_extract_text_from_file() {
    assert_round_trip(job(
        HeavyJobAction::ExtractTextFromFile,
        json!({
            "workspaceId": "workspace-1",
            "conversationId": "conversation-1",
            "attachmentId": "attachment-1",
            "originPath": "workspace-1/files/manual.pdf",
            "mimeType": "application/pdf",
            "query": "refund policy",
        }),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:25 "heavyJobDataSchema > parses analyzeImage payloads"
#[test]
fn parses_payloads_analyze_image() {
    assert_round_trip(job(
        HeavyJobAction::AnalyzeImage,
        json!({
            "workspaceId": "workspace-1",
            "originPath": "workspace-1/images/photo.png",
            "mimeType": "image/png",
            "sizeBytes": 1024,
            "prompt": "Describe this image",
            "providerInfo": { "provider": "openai", "model": "gpt-4o-mini" },
        }),
    ));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:118 "heavyJobDataSchema > rejects model-shaped identifiers at the Redis boundary"
#[test]
fn rejects_model_shaped_identifiers_at_the_redis_boundary() {
    let step = ai_generate_image_default_step(json!({
        "prompt": "A quiet workspace",
        "outputFieldId": "custom-field-output",
    }));
    let input = job(
        HeavyJobAction::AiGenerateImage,
        json!({
            "conversationId": { "id": "conversation-1" },
            "contactInboxId": "contact-inbox-1",
            "step": step,
        }),
    );
    assert!(HeavyJobData::parse(&input).is_err());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:134 "heavyJobDataSchema > accepts a durable continuation for a flow-owned heavy step"
#[test]
fn accepts_a_durable_continuation_for_a_flow_owned_heavy_step() {
    let input = job(
        HeavyJobAction::AiGenerateImage,
        step_data(json!({
            "outcomeKey": "heavy-step-outcome-1",
            "continuation": {
                "flowExecutionKey": "flow-execution-1",
                "flowId": "flow-1",
                "flowVersionId": "flow-version-1",
                "nodeId": "node-1",
            },
            "step": generate_step(),
        })),
    );
    assert!(HeavyJobData::parse(&input).is_ok());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:157 "heavyJobDataSchema > keeps the inbox sender on a durable continuation instead of stripping it"
#[test]
fn keeps_the_inbox_sender_on_a_durable_continuation_instead_of() {
    let parsed = HeavyJobData::parse(&job(
        HeavyJobAction::AiGenerateImage,
        step_data(json!({
            "outcomeKey": "heavy-step-outcome-1",
            "continuation": {
                "flowExecutionKey": "flow-execution-1",
                "flowId": "flow-1",
                "sendFrom": "inbox",
                "sentByUserId": "user-1",
            },
            "step": generate_step(),
        })),
    ))
    .expect("payload parses");

    let HeavyJobData::AiGenerateImage(data) = parsed else {
        panic!("expected an aiGenerateImage job");
    };
    let continuation = data.continuation.expect("continuation kept");
    assert_eq!(continuation.send_from.as_deref(), Some("inbox"));
    assert_eq!(continuation.sent_by_user_id.as_deref(), Some("user-1"));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:182 "heavyJobDataSchema > validates full openai-compatible provider config for image analysis"
#[test]
fn validates_full_openai_compatible_provider_config_for_image() {
    let compatible = job(
        HeavyJobAction::AnalyzeImage,
        json!({
            "workspaceId": "workspace-1",
            "originPath": "workspace-1/images/photo.png",
            "mimeType": "image/png",
            "sizeBytes": 1024,
            "prompt": "Describe this image",
            "providerInfo": {
                "kind": "openaiCompatible",
                "integrationId": "integration-1",
                "model": "custom-model",
            },
        }),
    );
    let parsed = HeavyJobData::parse(&compatible).expect("payload parses");
    assert_eq!(
        parsed.to_value(),
        json!({
            "type": "analyzeImage",
            "data": {
                "workspaceId": "workspace-1",
                "originPath": "workspace-1/images/photo.png",
                "mimeType": "image/png",
                "sizeBytes": 1024,
                "prompt": "Describe this image",
                "providerInfo": {
                    "kind": "openaiCompatible",
                    "integrationId": "integration-1",
                    "model": "custom-model",
                },
            },
        })
    );

    let legacy = job(
        HeavyJobAction::AnalyzeImage,
        json!({
            "workspaceId": "workspace-1",
            "originPath": "workspace-1/images/photo.png",
            "mimeType": "image/png",
            "sizeBytes": 1024,
            "prompt": "Describe this image",
            "providerInfo": { "provider": "openai", "modelId": "gpt-4o-mini" },
        }),
    );
    assert!(HeavyJobData::parse(&legacy).is_err());
}

// ---------------------------------------------------------------------------
// heavy result schemas
// ---------------------------------------------------------------------------

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:232 "heavy result schemas > requires at least one output value for successful flow-step results"
#[test]
fn requires_at_least_one_output_value_for_successful_flow_step() {
    let output_values = json!([
        {
            "customFieldId": "custom-field-output",
            "value": "https://cdn.example.com/result.png",
        },
        { "customFieldId": "custom-field-details", "value": "{\"count\":1}" },
    ]);
    let ok = json!({ "status": "success", "outputValues": output_values });
    assert_eq!(
        HeavyStepResult::parse(&ok).expect("parses").to_value(),
        json!({ "status": "success", "outputValues": output_values })
    );

    assert!(HeavyStepResult::parse(&json!({ "status": "success" })).is_err());
    assert!(HeavyStepResult::parse(&json!({ "status": "success", "outputValues": [] })).is_err());
    assert!(
        HeavyStepResult::parse(&json!({
            "status": "success",
            "outputValues": [{ "customFieldId": "custom-field-output", "value": "" }],
        }))
        .is_err()
    );
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:260 "heavy result schemas > rejects the legacy single outputValue success shape"
#[test]
fn rejects_the_legacy_single_outputvalue_success_shape() {
    assert!(
        HeavyStepResult::parse(&json!({
            "status": "success",
            "outputValue": "https://cdn.example.com/result.png",
        }))
        .is_err()
    );
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:269 "heavy result schemas > accepts a skip result without output values"
#[test]
fn accepts_a_skip_result_without_output_values() {
    let parsed = HeavyStepResult::parse(&json!({ "status": "skip" })).expect("parses");
    assert_eq!(parsed.to_value(), json!({ "status": "skip" }));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:275 "heavy result schemas > requires errorMessage for failed flow-step results"
#[test]
fn requires_errormessage_for_failed_flow_step_results() {
    let parsed =
        HeavyStepResult::parse(&json!({ "status": "error", "errorMessage": "Provider failed" }))
            .expect("parses");
    assert_eq!(
        parsed.to_value(),
        json!({ "status": "error", "errorMessage": "Provider failed" })
    );

    assert!(HeavyStepResult::parse(&json!({ "status": "error" })).is_err());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:291 "heavy result schemas > keeps tool results bounded and separate from flow-step results"
#[test]
fn keeps_tool_results_bounded_and_separate_from_flow_step() {
    let extract = json!({ "snippets": ["Matched paragraph"], "truncated": true });
    assert_eq!(
        HeavyExtractTextFromFileResult::parse(&extract)
            .expect("parses")
            .to_value(),
        json!({ "snippets": ["Matched paragraph"], "truncated": true })
    );

    let analyze = json!({ "analysis": "A product photo" });
    assert_eq!(
        HeavyAnalyzeImageResult::parse(&analyze)
            .expect("parses")
            .to_value(),
        json!({ "analysis": "A product photo" })
    );
}

// ---------------------------------------------------------------------------
// heavy job policies
// ---------------------------------------------------------------------------

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:306 "heavy job policies > uses longer retry budget for AI file processing"
#[test]
fn uses_longer_retry_budget_for_ai_file_processing() {
    let opts = heavy_job_options(HeavyJobAction::ProcessAiFile);
    assert_eq!(opts.attempts, Some(3));
    assert_eq!(opts.backoff, Some(Backoff::Exponential(30_000)));
}

fn assert_paid_media_policy(action: HeavyJobAction) {
    let opts = heavy_job_options(action);
    assert_eq!(opts.attempts, Some(2));
    assert_eq!(opts.backoff, Some(Backoff::Exponential(30_000)));
    assert_eq!(opts.remove_on_complete, Some(Retention::KeepCount(200)));
    assert_eq!(opts.remove_on_fail, Some(Retention::KeepCount(500)));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:315 "heavy job policies > keeps paid media retries bounded with a longer backoff for aiGenerateImage"
#[test]
fn keeps_paid_media_retries_bounded_with_a_longer_backoff_for_ai_generate_image() {
    assert_paid_media_policy(HeavyJobAction::AiGenerateImage);
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:315 "heavy job policies > keeps paid media retries bounded with a longer backoff for aiEditImage"
#[test]
fn keeps_paid_media_retries_bounded_with_a_longer_backoff_for_ai_edit_image() {
    assert_paid_media_policy(HeavyJobAction::AiEditImage);
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:315 "heavy job policies > keeps paid media retries bounded with a longer backoff for aiSegmentImage"
#[test]
fn keeps_paid_media_retries_bounded_with_a_longer_backoff_for_ai_segment_image() {
    assert_paid_media_policy(HeavyJobAction::AiSegmentImage);
}

// ---------------------------------------------------------------------------
// aiSegmentImage heavy contract
// ---------------------------------------------------------------------------

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:341 "aiSegmentImage heavy contract > rejects a step whose details share the output field at the Redis boundary"
#[test]
fn rejects_a_step_whose_details_share_the_output_field_at_the() {
    let input = job(
        HeavyJobAction::AiSegmentImage,
        step_data(json!({
            "step": segment_step(json!({ "detailsOutputFieldId": "custom-field-output" })),
        })),
    );
    assert!(HeavyJobData::parse(&input).is_err());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:353 "aiSegmentImage heavy contract > rejects a segment job carrying another step type"
#[test]
fn rejects_a_segment_job_carrying_another_step_type() {
    let input = job(
        HeavyJobAction::AiSegmentImage,
        step_data(json!({ "step": generate_step() })),
    );
    assert!(HeavyJobData::parse(&input).is_err());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:369 "aiSegmentImage heavy contract > rejects a segment job without its conversation"
#[test]
fn rejects_a_segment_job_without_its_conversation() {
    let input = job(
        HeavyJobAction::AiSegmentImage,
        json!({ "contactInboxId": "contact-inbox-1", "step": segment_step(json!({})) }),
    );
    assert!(HeavyJobData::parse(&input).is_err());
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:378 "aiSegmentImage heavy contract > keeps the durable continuation on a segment job"
#[test]
fn keeps_the_durable_continuation_on_a_segment_job() {
    let parsed = HeavyJobData::parse(&job(
        HeavyJobAction::AiSegmentImage,
        step_data(json!({
            "outcomeKey": "heavy-step-outcome-1",
            "continuation": {
                "flowExecutionKey": "flow-execution-1",
                "flowId": "flow-1",
            },
            "step": segment_step(json!({})),
        })),
    ))
    .expect("payload parses");

    let HeavyJobData::AiSegmentImage(data) = parsed else {
        panic!("expected an aiSegmentImage job");
    };
    assert_eq!(data.outcome_key.as_deref(), Some("heavy-step-outcome-1"));
    assert_eq!(
        data.continuation
            .expect("continuation kept")
            .flow_execution_key,
        "flow-execution-1"
    );
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:398 "aiSegmentImage heavy contract > waits for both paid attempts, the backoff and the handoff"
#[test]
fn waits_for_both_paid_attempts_the_backoff_and_the_handoff() {
    // 2 x 120 s + 30 s backoff + 60 s QueueEvents handoff.
    assert_eq!(
        heavy_completion_wait_timeout_ms(HeavyJobAction::AiSegmentImage, 120_000),
        330_000
    );
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:408 "aiSegmentImage heavy contract > strips stray fields from a skip result"
#[test]
fn strips_stray_fields_from_a_skip_result() {
    let parsed = HeavyStepResult::parse(&json!({
        "status": "skip",
        "outputValues": [{ "customFieldId": "x", "value": "y" }],
    }))
    .expect("parses");
    assert_eq!(parsed.to_value(), json!({ "status": "skip" }));
}

/// Port of: packages/worker-config/__tests__/heavy-job-schema.test.ts:417 "aiSegmentImage heavy contract > rejects an output value without its custom field"
#[test]
fn rejects_an_output_value_without_its_custom_field() {
    assert!(
        HeavyStepResult::parse(&json!({
            "status": "success",
            "outputValues": [{ "customFieldId": "", "value": "https://cdn/x.png" }],
        }))
        .is_err()
    );
}
