//! The `aiAgent` queue (`packages/worker-config/src/queues/ai-agent`):
//! `AIJobAction` and `aiJobDataSchema` (the Phase 1 reply jobs).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::heavy::SchemaError;

/// `AIJobAction` (the reply jobs the frozen tests use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiJobAction {
    ProcessAutomatedResponse,
    CommentAiReply,
    ProcessStoryReplyAutomation,
    ProcessStoryMentionAutomation,
}

impl AiJobAction {
    /// `"processAutomatedResponse"`, `"commentAIReply"`,
    /// `"processStoryReplyAutomation"`, `"processStoryMentionAutomation"`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// `processAutomatedResponse` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedResponseData {
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub message_id: String,
}

/// `commentAIReply` data. `automation_id` is required for new jobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentAiReplyData {
    pub automation_id: String,
    pub integration_type: String,
    pub integration_identifier: String,
    pub workspace_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub comment_id: String,
    pub agent_id: String,
    /// `"public"` / `"private"`.
    pub reply_channel: String,
    pub channel_type: String,
    pub message: String,
}

/// Channel of a story automation: Instagram variants only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoryChannelType {
    Instagram,
    InstagramFacebook,
}

/// `processStoryReplyAutomation` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryReplyData {
    pub workspace_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub message_id: String,
    pub story_id: String,
    pub channel_type: StoryChannelType,
}

/// `processStoryMentionAutomation` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryMentionData {
    pub workspace_id: String,
    pub conversation_id: String,
    pub contact_inbox_id: String,
    pub message_id: String,
    pub channel_type: StoryChannelType,
}

/// `aiJobDataSchema`: `{ type, data }` of an aiAgent job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum AiJobData {
    #[serde(rename = "processAutomatedResponse")]
    ProcessAutomatedResponse(AutomatedResponseData),
    #[serde(rename = "commentAIReply")]
    CommentAiReply(CommentAiReplyData),
    #[serde(rename = "processStoryReplyAutomation")]
    ProcessStoryReplyAutomation(StoryReplyData),
    #[serde(rename = "processStoryMentionAutomation")]
    ProcessStoryMentionAutomation(StoryMentionData),
}

impl AiJobData {
    /// `aiJobDataSchema.parse` (string identifiers only).
    pub fn parse(value: &Value) -> Result<AiJobData, SchemaError> {
        todo!()
    }

    /// The JSON written to the queue.
    pub fn to_value(&self) -> Value {
        todo!()
    }
}
