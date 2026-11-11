//! Port of `packages/worker-config/__tests__/resolve-step-input-job.test.ts`.

use chatbotx_jobs::queues::resolve_step_input::{
    RESOLVE_STEP_INPUT_ACTION, ResolveStepInput, ResolveStepInputReason,
    build_resolve_step_input_job,
};
use serde_json::json;

fn skip_tap() -> ResolveStepInput {
    ResolveStepInput {
        conversation_id: "conv-1".to_owned(),
        contact_inbox_id: "ci-1".to_owned(),
        challenge_id: "11612473309626368".to_owned(),
        reason: ResolveStepInputReason::UserSkipped,
        message_id: Some("msg-1".to_owned()),
        message_created_at: Some("2026-11-09T10:00:00.000Z".to_owned()),
    }
}

fn dedup_id(input: &ResolveStepInput) -> Option<String> {
    build_resolve_step_input_job(input).opts.dedup.map(|d| d.id)
}

/// Port of: packages/worker-config/__tests__/resolve-step-input-job.test.ts:17 "buildResolveStepInputJob > names the job and carries the payload unchanged"
#[test]
fn names_the_job_and_carries_the_payload_unchanged() {
    let job = build_resolve_step_input_job(&skip_tap());

    assert_eq!(job.name, RESOLVE_STEP_INPUT_ACTION);
    assert_eq!(RESOLVE_STEP_INPUT_ACTION, "resolveStepInput");
    assert_eq!(
        job.data,
        json!({
            "type": "resolveStepInput",
            "data": {
                "conversationId": "conv-1",
                "contactInboxId": "ci-1",
                "challengeId": "11612473309626368",
                "reason": "user_skipped",
                "messageId": "msg-1",
                "messageCreatedAt": "2026-11-09T10:00:00.000Z",
            },
        })
    );
}

/// Port of: packages/worker-config/__tests__/resolve-step-input-job.test.ts:27 "buildResolveStepInputJob > a message-less resolution carries no message identity"
#[test]
fn a_message_less_resolution_carries_no_message_identity() {
    let without_message = ResolveStepInput {
        message_id: None,
        message_created_at: None,
        ..skip_tap()
    };

    let job = build_resolve_step_input_job(&without_message);

    let data = job.data["data"].as_object().expect("data object");
    assert!(!data.contains_key("messageId"));
}

/// Port of: packages/worker-config/__tests__/resolve-step-input-job.test.ts:35 "buildResolveStepInputJob > collapses duplicates per conversation and challenge, only while a job is active"
#[test]
fn collapses_duplicates_per_conversation_and_challenge_only() {
    let first = build_resolve_step_input_job(&skip_tap());
    let same_challenge = dedup_id(&ResolveStepInput {
        message_id: Some("msg-2".to_owned()),
        ..skip_tap()
    });
    let other_challenge = dedup_id(&ResolveStepInput {
        challenge_id: "11612473309626369".to_owned(),
        ..skip_tap()
    });
    let other_conversation = dedup_id(&ResolveStepInput {
        conversation_id: "conv-2".to_owned(),
        ..skip_tap()
    });

    let first_dedup = first.opts.dedup.clone().expect("deduplication set");
    let first_id = Some(first_dedup.id.clone());
    assert_eq!(first_dedup.id, "step-input-conv-1-11612473309626368");
    // "only while a job is active": simple mode (no ttl, no debounce).
    assert_eq!(first_dedup.ttl_ms, None);
    assert_eq!(same_challenge, first_id);
    assert_ne!(other_challenge, first_id);
    assert_ne!(other_conversation, first_id);
}

/// Port of: packages/worker-config/__tests__/resolve-step-input-job.test.ts:64 "buildResolveStepInputJob > never uses a fixed jobId: a retained completed job would drop a later tap"
#[test]
fn never_uses_a_fixed_jobid_a_retained_completed_job_would_drop() {
    assert!(
        build_resolve_step_input_job(&skip_tap())
            .opts
            .job_id
            .is_none()
    );
}
