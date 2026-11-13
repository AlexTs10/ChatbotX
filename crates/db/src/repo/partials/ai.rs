//! Port of the chunking config and the AI agent action contract of `partials/ai.ts`.

use serde::{Deserialize, Serialize};

use serde_json::Value;

use super::shared::{Issues, ZodError, ZodIssue, deserialize, js_length};

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
    let config: AiFileChunkingConfig = deserialize(value)?;
    let mut issues = Issues::default();
    let (chunk_min, chunk_max, _) = CHUNK_SIZE_LIMITS;
    let (overlap_max, _) = OVERLAP_SIZE_LIMITS;
    let (row_min, row_max, _) = TABULAR_HEADER_ROW_LIMITS;
    check_range(
        &mut issues,
        "chunkSize",
        config.chunk_size,
        chunk_min,
        chunk_max,
    );
    check_range(
        &mut issues,
        "overlapSize",
        config.overlap_size,
        0,
        overlap_max,
    );
    check_range(
        &mut issues,
        "tabularHeaderRow",
        config.tabular_header_row,
        row_min,
        row_max,
    );
    issues.finish(config)
}

/// `z.number().int().min(min).max(max).optional()` (the `int` part is the serde `i64`).
fn check_range(issues: &mut Issues, path: &str, value: Option<i64>, min: i64, max: i64) {
    match value {
        Some(value) if value < min => {
            issues.add(path, format!("Too small: expected number to be >={min}"));
        }
        Some(value) if value > max => {
            issues.add(path, format!("Too big: expected number to be <={max}"));
        }
        _ => {}
    }
}

/// `resolveAIFileChunkingConfig`: `None` (undefined), `null` or a malformed value give
/// the defaults; the overlap is at most half the chunk size.
pub fn resolve_ai_file_chunking_config(
    config: Option<&serde_json::Value>,
) -> ResolvedAiFileChunkingConfig {
    let value = config
        .and_then(|config| parse_ai_file_chunking_config(config).ok())
        .unwrap_or_default();
    let (chunk_min, chunk_max, chunk_default) = CHUNK_SIZE_LIMITS;
    let (overlap_max, overlap_default) = OVERLAP_SIZE_LIMITS;
    let (row_min, row_max, row_default) = TABULAR_HEADER_ROW_LIMITS;
    let chunk_size = value
        .chunk_size
        .unwrap_or(chunk_default)
        .clamp(chunk_min, chunk_max);
    let overlap_size = overlap_max
        .min(chunk_size.div_euclid(2))
        .min(value.overlap_size.unwrap_or(overlap_default).max(0));
    ResolvedAiFileChunkingConfig {
        strategy: value.strategy.unwrap_or(AiFileChunkingConfigStrategy::Auto),
        chunk_size,
        overlap_size,
        tabular_header_row: value
            .tabular_header_row
            .unwrap_or(row_default)
            .clamp(row_min, row_max),
    }
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
    if input.extracted_chars <= 0 {
        return 0;
    }
    if input.strategy == AiFileChunkStrategy::Prose {
        let step = (input.chunk_size - input.overlap_size).max(1);
        return ceil_div(input.extracted_chars - input.overlap_size, step).max(1);
    }
    ceil_div(input.extracted_chars, input.chunk_size).max(1)
}

/// `Math.ceil(a / b)` for a positive `b`.
fn ceil_div(a: i64, b: i64) -> i64 {
    let quotient = a.div_euclid(b);
    if a.rem_euclid(b) == 0 {
        quotient
    } else {
        quotient + 1
    }
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
    let Some(object) = value.as_object() else {
        return Err(ZodError::one("", "Invalid input: expected object"));
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // The payload key of each variant (`assign_conversation`: `assignedId`, or the legacy
    // `adminId` variant when that key is sent).
    let payload: Option<&str> = match kind {
        "send_flow" => Some("flowId"),
        "assign_conversation" if object.contains_key("adminId") => Some("adminId"),
        "assign_conversation" => Some("assignedId"),
        "add_tag" | "remove_tag" => Some("tagId"),
        "set_custom_field" | "clear_custom_field" => Some("customFieldId"),
        "remove_assignment" | "transfer_to_human" | "mark_follow_up" | "remove_follow_up"
        | "transfer_to_bot" | "archive" | "block_contact" => None,
        _ => return Err(ZodError::one("type", "Invalid input")),
    };
    let mut issues = Issues::default();
    let mut trimmed = object.clone();
    for key in object.keys() {
        if key != "id" && key != "type" && Some(key.as_str()) != payload {
            issues.add("", format!("Unrecognized key: \"{key}\""));
        }
    }
    check_text(&mut issues, &mut trimmed, "id", 128);
    if let Some(key) = payload {
        check_text(&mut issues, &mut trimmed, key, usize::MAX);
        if key == "assignedId" {
            let assigned = trimmed.get(key).and_then(Value::as_str).unwrap_or_default();
            if !is_assignment_target(assigned) {
                issues.add(key, "Invalid string: must match pattern /^[ut]_\\S+$/");
            }
        }
    }
    if !issues.0.is_empty() {
        return Err(ZodError { issues: issues.0 });
    }
    deserialize(&Value::Object(trimmed))
}

/// `/^[ut]_\S+$/`.
fn is_assignment_target(value: &str) -> bool {
    let rest = value
        .strip_prefix("u_")
        .or_else(|| value.strip_prefix("t_"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
}

/// `z.string().trim().min(1).max(max)` on `object[key]` (trimmed in place).
fn check_text(
    issues: &mut Issues,
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    max: usize,
) {
    let Some(text) = object.get(key).and_then(Value::as_str) else {
        issues.add(key, "Invalid input: expected string");
        return;
    };
    let text = text.trim().to_owned();
    let length = js_length(&text);
    if length < 1 {
        issues.add(key, "Too small: expected string to have >=1 characters");
    } else if length > max {
        issues.add(
            key,
            format!("Too big: expected string to have <={max} characters"),
        );
    }
    object.insert(key.to_owned(), Value::String(text));
}

impl AiAgentAction {
    /// The stable action id.
    pub fn id(&self) -> &str {
        match self {
            AiAgentAction::SendFlow { id, .. }
            | AiAgentAction::AssignConversation { id, .. }
            | AiAgentAction::RemoveAssignment { id }
            | AiAgentAction::TransferToHuman { id }
            | AiAgentAction::AddTag { id, .. }
            | AiAgentAction::RemoveTag { id, .. }
            | AiAgentAction::SetCustomField { id, .. }
            | AiAgentAction::ClearCustomField { id, .. }
            | AiAgentAction::MarkFollowUp { id }
            | AiAgentAction::RemoveFollowUp { id }
            | AiAgentAction::TransferToBot { id }
            | AiAgentAction::Archive { id }
            | AiAgentAction::BlockContact { id } => id,
        }
    }
}

/// `aiAgentActionRuleSchema.safeParse`.
pub fn parse_ai_agent_action_rule(
    value: &serde_json::Value,
) -> Result<AiAgentActionRule, ZodError> {
    let Some(object) = value.as_object() else {
        return Err(ZodError::one("", "Invalid input: expected object"));
    };
    let mut issues = Issues::default();
    let mut trimmed = object.clone();
    for key in object.keys() {
        if !matches!(key.as_str(), "id" | "when" | "actions") {
            issues.add("", format!("Unrecognized key: \"{key}\""));
        }
    }
    check_text(&mut issues, &mut trimmed, "id", 128);
    check_text(&mut issues, &mut trimmed, "when", 2000);
    let mut actions = Vec::new();
    match object.get("actions").and_then(Value::as_array) {
        None => issues.add("actions", "Invalid input: expected array"),
        Some(items) => {
            for (index, item) in items.iter().enumerate() {
                match parse_ai_agent_action(item) {
                    Ok(action) => actions.push(action),
                    Err(error) => extend(&mut issues, &format!("actions.{index}"), error),
                }
            }
            if items.is_empty() {
                issues.add("actions", "Too small: expected array to have >=1 items");
            } else if items.len() > 15 {
                issues.add("actions", "Too big: expected array to have <=15 items");
            }
        }
    }
    let rule = AiAgentActionRule {
        id: trimmed
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        when: trimmed
            .get("when")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        actions,
    };
    if !issues.0.is_empty() {
        return issues.finish(rule);
    }
    let mut ids = std::collections::HashSet::new();
    for (index, action) in rule.actions.iter().enumerate() {
        if !ids.insert(action.id()) {
            issues.add(
                format!("actions.{index}.id"),
                "Action IDs must be unique within a rule.",
            );
        }
    }
    issues.finish(rule)
}

fn extend(issues: &mut Issues, prefix: &str, error: ZodError) {
    issues
        .0
        .extend(error.issues.into_iter().map(|issue| ZodIssue {
            path: if issue.path.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}.{}", issue.path)
            },
            message: issue.message,
        }));
}

/// `aiAgentActionRulesSchema.safeParse` (at most 20 rules; rule ids and action ids unique
/// in the agent).
pub fn parse_ai_agent_action_rules(
    value: &serde_json::Value,
) -> Result<Vec<AiAgentActionRule>, ZodError> {
    let items: Vec<Value> = deserialize(value)?;
    let mut issues = Issues::default();
    let mut rules = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match parse_ai_agent_action_rule(item) {
            Ok(rule) => rules.push(rule),
            Err(error) => extend(&mut issues, &index.to_string(), error),
        }
    }
    if items.len() > 20 {
        issues.add("", "Too big: expected array to have <=20 items");
    }
    if !issues.0.is_empty() {
        return issues.finish(rules);
    }
    let mut rule_ids = std::collections::HashSet::new();
    let mut action_ids = std::collections::HashSet::new();
    for (index, rule) in rules.iter().enumerate() {
        if !rule_ids.insert(rule.id.as_str()) {
            issues.add(
                format!("{index}.id"),
                "Rule IDs must be unique within an AI agent.",
            );
        }
        for (action_index, action) in rule.actions.iter().enumerate() {
            if !action_ids.insert(action.id()) {
                issues.add(
                    format!("{index}.actions.{action_index}.id"),
                    "Action IDs must be unique within an AI agent.",
                );
            }
        }
    }
    issues.finish(rules)
}
