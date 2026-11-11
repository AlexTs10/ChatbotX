//! Port of `packages/worker-config/__tests__/ai-agent-job-schema.test.ts`.

use chatbotx_jobs::queues::ai_agent::{AiJobAction, AiJobData};
use serde_json::{Value, json};

fn automated_response_job() -> Value {
    json!({
        "type": AiJobAction::ProcessAutomatedResponse.as_str(),
        "data": {
            "conversationId": "conversation-1",
            "contactInboxId": "contact-inbox-1",
            "messageId": "message-1",
        },
    })
}

fn comment_ai_reply_job() -> Value {
    json!({
        "type": AiJobAction::CommentAiReply.as_str(),
        "data": {
            "automationId": "automation-1",
            "integrationType": "messenger",
            "integrationIdentifier": "page-1",
            "workspaceId": "workspace-1",
            "conversationId": "conversation-1",
            "contactInboxId": "contact-inbox-1",
            "commentId": "comment-1",
            "agentId": "agent-1",
            "replyChannel": "public",
            "channelType": "messenger",
            "message": "hello",
        },
    })
}

fn story_reply_job() -> Value {
    json!({
        "type": AiJobAction::ProcessStoryReplyAutomation.as_str(),
        "data": {
            "workspaceId": "workspace-1",
            "conversationId": "conversation-1",
            "contactInboxId": "contact-inbox-1",
            "messageId": "message-1",
            "storyId": "story-1",
            "channelType": "instagram",
        },
    })
}

fn story_mention_job() -> Value {
    json!({
        "type": AiJobAction::ProcessStoryMentionAutomation.as_str(),
        "data": {
            "workspaceId": "workspace-1",
            "conversationId": "conversation-1",
            "contactInboxId": "contact-inbox-1",
            "messageId": "message-1",
            "channelType": "instagramFacebook",
        },
    })
}

fn assert_round_trip(job: Value) {
    let parsed = AiJobData::parse(&job).expect("payload parses");
    assert_eq!(parsed.to_value(), job);
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:54 "aiJobDataSchema Phase 1 reply jobs > parses processAutomatedResponse payloads"
#[test]
fn parses_payloads_process_automated_response() {
    assert_round_trip(automated_response_job());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:54 "aiJobDataSchema Phase 1 reply jobs > parses commentAIReply payloads"
#[test]
fn parses_payloads_comment_ai_reply() {
    assert_round_trip(comment_ai_reply_job());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:54 "aiJobDataSchema Phase 1 reply jobs > parses processStoryReplyAutomation payloads"
#[test]
fn parses_payloads_process_story_reply_automation() {
    assert_round_trip(story_reply_job());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:54 "aiJobDataSchema Phase 1 reply jobs > parses processStoryMentionAutomation payloads"
#[test]
fn parses_payloads_process_story_mention_automation() {
    assert_round_trip(story_mention_job());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:63 "aiJobDataSchema Phase 1 reply jobs > rejects model-shaped identifiers at the Redis boundary"
#[test]
fn rejects_model_shaped_identifiers_at_the_redis_boundary() {
    let mut job = automated_response_job();
    job["data"]["conversationId"] = json!({ "id": "conversation-1" });
    assert!(AiJobData::parse(&job).is_err());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:75 "aiJobDataSchema Phase 1 reply jobs > rejects a new comment AI reply without automationId"
#[test]
fn rejects_a_new_comment_ai_reply_without_automationid() {
    let mut legacy_data = comment_ai_reply_job()["data"].clone();
    legacy_data.as_object_mut().unwrap().remove("automationId");
    let job = json!({
        "type": AiJobAction::CommentAiReply.as_str(),
        "data": legacy_data,
    });
    assert!(AiJobData::parse(&job).is_err());
}

/// Port of: packages/worker-config/__tests__/ai-agent-job-schema.test.ts:87 "aiJobDataSchema Phase 1 reply jobs > rejects malformed story reply channels"
#[test]
fn rejects_malformed_story_reply_channels() {
    let mut job = story_reply_job();
    job["data"]["channelType"] = json!("messenger");
    assert!(AiJobData::parse(&job).is_err());
}
