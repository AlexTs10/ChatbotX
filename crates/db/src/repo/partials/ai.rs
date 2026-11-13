//! Port of the chunking config and the AI agent action contract of `partials/ai.ts`.

use serde::{Deserialize, Serialize};

use super::shared::ZodError;

/// `aiFileChunkStrategies`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiFileChunkStrategy {
    Prose,
    Tabular,
    Markdown,
}

/// `aiFileChunkingConfigStrategies`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiFileChunkingConfigStrategy {
    Auto,
    Prose,
}

/// `AI_FILE_CHUNKING_LIMITS.chunkSize` (min, max, default).
pub const CHUNK_SIZE_LIMITS: (i64, i64, i64) = (200, 2000, 1000);
/// `AI_FILE_CHUNKING_LIMITS.overlapSize` (max, default).
pub const OVERLAP_SIZE_LIMITS: (i64, i64) = (500, 200);
/// `AI_FILE_CHUNKING_LIMITS.tabularHeaderRow` (min, max, default).
pub const TABULAR_HEADER_ROW_LIMITS: (i64, i64, i64) = (1, 20, 1);

/// `aiFileChunkingConfig` (a strict object: unknown keys are refused).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiFileChunkingConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<AiFileChunkingConfigStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabular_header_row: Option<i64>,
}

/// `ResolvedAIFileChunkingConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAiFileChunkingConfig {
    pub strategy: AiFileChunkingConfigStrategy,
    pub chunk_size: i64,
    pub overlap_size: i64,
    pub tabular_header_row: i64,
}

/// `aiFileChunkingConfig.safeParse`.
pub fn parse_ai_file_chunking_config(
    value: &serde_json::Value,
) -> Result<AiFileChunkingConfig, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `resolveAIFileChunkingConfig`: `None` (undefined), `null` or a malformed value give
/// the defaults; the overlap is at most half the chunk size.
pub fn resolve_ai_file_chunking_config(
    config: Option<&serde_json::Value>,
) -> ResolvedAiFileChunkingConfig {
    let _ = config;
    todo!("P3.2b")
}

/// The input of `estimateAIFileChunkCount`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkCountInput {
    pub strategy: AiFileChunkStrategy,
    pub extracted_chars: i64,
    pub chunk_size: i64,
    pub overlap_size: i64,
}

/// `estimateAIFileChunkCount`.
pub fn estimate_ai_file_chunk_count(input: ChunkCountInput) -> i64 {
    let _ = input;
    todo!("P3.2b")
}

/// `aiAgentActionSchema` (each variant is a strict object with an `id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiAgentAction {
    SendFlow {
        id: String,
        #[serde(rename = "flowId")]
        flow_id: String,
    },
    /// `assignedId` (`u_` / `t_` prefix), or the legacy `adminId`; exactly one of them.
    AssignConversation {
        id: String,
        #[serde(
            rename = "assignedId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        assigned_id: Option<String>,
        #[serde(rename = "adminId", default, skip_serializing_if = "Option::is_none")]
        admin_id: Option<String>,
    },
    RemoveAssignment {
        id: String,
    },
    TransferToHuman {
        id: String,
    },
    AddTag {
        id: String,
        #[serde(rename = "tagId")]
        tag_id: String,
    },
    RemoveTag {
        id: String,
        #[serde(rename = "tagId")]
        tag_id: String,
    },
    SetCustomField {
        id: String,
        #[serde(rename = "customFieldId")]
        custom_field_id: String,
    },
    ClearCustomField {
        id: String,
        #[serde(rename = "customFieldId")]
        custom_field_id: String,
    },
    MarkFollowUp {
        id: String,
    },
    RemoveFollowUp {
        id: String,
    },
    TransferToBot {
        id: String,
    },
    Archive {
        id: String,
    },
    BlockContact {
        id: String,
    },
}

/// `aiAgentActionRuleSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiAgentActionRule {
    pub id: String,
    pub when: String,
    pub actions: Vec<AiAgentAction>,
}

/// `aiAgentActionSchema.safeParse`.
pub fn parse_ai_agent_action(value: &serde_json::Value) -> Result<AiAgentAction, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `aiAgentActionRulesSchema.safeParse` (at most 20 rules; rule ids and action ids unique
/// in the agent).
pub fn parse_ai_agent_action_rules(
    value: &serde_json::Value,
) -> Result<Vec<AiAgentActionRule>, ZodError> {
    let _ = value;
    todo!("P3.2b")
}
