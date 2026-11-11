//! The `heavy` queue (`packages/worker-config/src/queues/heavy`): job
//! actions, payload and result types, job options per action, and the
//! completion wait budget.
//!
//! The flow steps carried by step jobs come from the flow-config port, which
//! does not exist yet; the step is kept as JSON here, and the
//! `*_default_step` builders stand in for the flow-config `*DefaultFn`s.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::options::{Backoff, JobOptions, Retention};

/// `HeavyJobAction` (serde names = the TypeScript strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeavyJobAction {
    ProcessAiFile,
    AiEditImage,
    AiSegmentImage,
    AiGenerateImage,
    AiSpeechToText,
    AiTextToSpeech,
    ExtractTextFromFile,
    AnalyzeImage,
    OptimizeStoredImage,
}

impl HeavyJobAction {
    /// `"processAIFile"`, `"aiEditImage"`, ..., `"optimizeStoredImage"`.
    pub fn as_str(self) -> &'static str {
        match self {
            HeavyJobAction::ProcessAiFile => "processAIFile",
            HeavyJobAction::AiEditImage => "aiEditImage",
            HeavyJobAction::AiSegmentImage => "aiSegmentImage",
            HeavyJobAction::AiGenerateImage => "aiGenerateImage",
            HeavyJobAction::AiSpeechToText => "aiSpeechToText",
            HeavyJobAction::AiTextToSpeech => "aiTextToSpeech",
            HeavyJobAction::ExtractTextFromFile => "extractTextFromFile",
            HeavyJobAction::AnalyzeImage => "analyzeImage",
            HeavyJobAction::OptimizeStoredImage => "optimizeStoredImage",
        }
    }
}

/// `getHeavyJobCompletionWaitTimeoutMs(action, waitTimeoutMs)`: every paid
/// attempt (`wait_timeout_ms` each) plus the backoff between them plus the
/// 60 000 ms handoff (spec S8).
pub fn heavy_completion_wait_timeout_ms(action: HeavyJobAction, wait_timeout_ms: u64) -> u64 {
    let options = heavy_job_options(action);
    let attempts = u64::from(options.attempts.unwrap_or(1));
    let retries = attempts.saturating_sub(1);
    let backoff_total: u64 = match options.backoff {
        None => 0,
        Some(Backoff::Fixed(delay)) => delay.saturating_mul(retries),
        Some(Backoff::Exponential(delay)) => (0..retries)
            .map(|i| delay.saturating_mul(1u64 << i.min(62)))
            .fold(0u64, u64::saturating_add),
    };
    attempts
        .saturating_mul(wait_timeout_ms)
        .saturating_add(backoff_total)
        .saturating_add(HEAVY_JOB_COMPLETION_WAIT_BUFFER_MS)
}

/// Extra time for the completion event and worker handoff after the final
/// provider attempt (`HEAVY_JOB_COMPLETION_WAIT_BUFFER_MS`).
const HEAVY_JOB_COMPLETION_WAIT_BUFFER_MS: u64 = 60_000;

/// `getHeavyJobOptions(action)`: attempts, backoff and retention per action
/// (spec 1.2 row `heavy`).
pub fn heavy_job_options(action: HeavyJobAction) -> JobOptions {
    let paid = |delay: u64| JobOptions {
        attempts: Some(2),
        backoff: Some(Backoff::Exponential(delay)),
        remove_on_complete: Some(Retention::KeepCount(200)),
        remove_on_fail: Some(Retention::KeepCount(500)),
        ..JobOptions::default()
    };
    match action {
        HeavyJobAction::ProcessAiFile => JobOptions {
            attempts: Some(3),
            backoff: Some(Backoff::Exponential(30_000)),
            remove_on_complete: Some(Retention::KeepCount(500)),
            remove_on_fail: Some(Retention::KeepCount(1000)),
            ..JobOptions::default()
        },
        HeavyJobAction::AiEditImage
        | HeavyJobAction::AiSegmentImage
        | HeavyJobAction::AiGenerateImage
        | HeavyJobAction::AnalyzeImage => paid(30_000),
        HeavyJobAction::AiSpeechToText
        | HeavyJobAction::AiTextToSpeech
        | HeavyJobAction::ExtractTextFromFile => paid(15_000),
        HeavyJobAction::OptimizeStoredImage => JobOptions {
            attempts: Some(2),
            backoff: Some(Backoff::Exponential(60_000)),
            delay_ms: Some(5000),
            priority: Some(1000),
            remove_on_complete: Some(Retention::KeepAge {
                age_secs: 3600,
                count: Some(1000),
            }),
            remove_on_fail: Some(Retention::KeepCount(1000)),
            ..JobOptions::default()
        },
    }
}

/// A payload that does not match its schema (the zod `safeParse` failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub message: String,
}

impl SchemaError {
    pub fn new(message: impl Into<String>) -> SchemaError {
        SchemaError {
            message: message.into(),
        }
    }

    pub(crate) fn from_serde(error: serde_json::Error) -> SchemaError {
        SchemaError::new(error.to_string())
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SchemaError {}

/// `processAIFile` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessAiFileData {
    pub ai_file_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// The durable continuation of a flow-owned heavy step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyContinuation {
    pub flow_execution_key: String,
    pub flow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// `"inbox"` when the step was started by an inbox user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_by_user_id: Option<String>,
}

/// Data of a step job (`aiEditImage`, `aiSegmentImage`, `aiGenerateImage`,
/// `aiSpeechToText`, `aiTextToSpeech`). Identifiers are plain strings at
/// the queue boundary (no model objects).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyStepJobData {
    pub conversation_id: String,
    pub contact_inbox_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<HeavyContinuation>,
    /// The flow step (flow-config shape); its `type` must match the job.
    pub step: Value,
}

/// `extractTextFromFile` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractTextFromFileData {
    pub workspace_id: String,
    pub conversation_id: String,
    pub attachment_id: String,
    pub origin_path: String,
    pub mime_type: String,
    pub query: String,
}

/// The provider of an image analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageProviderInfo {
    /// `{ kind: "openaiCompatible", integrationId, model }`.
    #[serde(rename_all = "camelCase")]
    OpenaiCompatible {
        kind: String,
        integration_id: String,
        model: String,
    },
    /// `{ provider, model }`.
    Builtin { provider: String, model: String },
}

/// `analyzeImage` data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeImageData {
    pub workspace_id: String,
    pub origin_path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub prompt: String,
    pub provider_info: ImageProviderInfo,
}

/// `heavyJobDataSchema`: `{ type, data }` of a heavy job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum HeavyJobData {
    #[serde(rename = "processAIFile")]
    ProcessAiFile(ProcessAiFileData),
    #[serde(rename = "aiEditImage")]
    AiEditImage(HeavyStepJobData),
    #[serde(rename = "aiSegmentImage")]
    AiSegmentImage(HeavyStepJobData),
    #[serde(rename = "aiGenerateImage")]
    AiGenerateImage(HeavyStepJobData),
    #[serde(rename = "aiSpeechToText")]
    AiSpeechToText(HeavyStepJobData),
    #[serde(rename = "aiTextToSpeech")]
    AiTextToSpeech(HeavyStepJobData),
    #[serde(rename = "extractTextFromFile")]
    ExtractTextFromFile(ExtractTextFromFileData),
    #[serde(rename = "analyzeImage")]
    AnalyzeImage(AnalyzeImageData),
}

impl HeavyJobData {
    /// `heavyJobDataSchema.parse`: deserialize, then the refinements (step
    /// type matches the job, segment details field differs from the output
    /// field, string identifiers).
    pub fn parse(value: &Value) -> Result<HeavyJobData, SchemaError> {
        let parsed: HeavyJobData =
            serde_json::from_value(value.clone()).map_err(SchemaError::from_serde)?;
        parsed.validate()?;
        Ok(parsed)
    }

    /// The JSON written to the queue.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn validate(&self) -> Result<(), SchemaError> {
        match self {
            HeavyJobData::ProcessAiFile(d) => {
                non_empty("aiFileId", &d.ai_file_id)?;
                if let Some(run) = &d.run_id {
                    non_empty("runId", run)?;
                }
                Ok(())
            }
            HeavyJobData::AiEditImage(d) => d.validate("aiEditImage"),
            HeavyJobData::AiSegmentImage(d) => {
                d.validate("aiSegmentImage")?;
                let field = |name: &str| d.step.get(name).and_then(Value::as_str);
                if let Some(details) = field("detailsOutputFieldId").filter(|s| !s.is_empty()) {
                    if Some(details) == field("outputFieldId") {
                        return Err(SchemaError::new(
                            "Details field must differ from the output image field",
                        ));
                    }
                    if Some(details) == field("inputFieldId") {
                        return Err(SchemaError::new(
                            "Details field must differ from the source image field",
                        ));
                    }
                }
                Ok(())
            }
            HeavyJobData::AiGenerateImage(d) => d.validate("aiGenerateImage"),
            HeavyJobData::AiSpeechToText(d) => d.validate("aiSpeechToText"),
            HeavyJobData::AiTextToSpeech(d) => d.validate("aiTextToSpeech"),
            HeavyJobData::ExtractTextFromFile(d) => {
                for (name, value) in [
                    ("workspaceId", &d.workspace_id),
                    ("conversationId", &d.conversation_id),
                    ("attachmentId", &d.attachment_id),
                    ("originPath", &d.origin_path),
                    ("mimeType", &d.mime_type),
                    ("query", &d.query),
                ] {
                    non_empty(name, value)?;
                }
                Ok(())
            }
            HeavyJobData::AnalyzeImage(d) => {
                for (name, value) in [
                    ("workspaceId", &d.workspace_id),
                    ("originPath", &d.origin_path),
                    ("mimeType", &d.mime_type),
                    ("prompt", &d.prompt),
                ] {
                    non_empty(name, value)?;
                }
                match &d.provider_info {
                    ImageProviderInfo::OpenaiCompatible {
                        kind,
                        integration_id,
                        model,
                    } => {
                        if kind != "openaiCompatible" {
                            return Err(SchemaError::new("providerInfo.kind is not valid"));
                        }
                        non_empty("providerInfo.integrationId", integration_id)?;
                        non_empty("providerInfo.model", model)
                    }
                    ImageProviderInfo::Builtin { provider, model } => {
                        non_empty("providerInfo.provider", provider)?;
                        non_empty("providerInfo.model", model)
                    }
                }
            }
        }
    }
}

fn non_empty(name: &str, value: &str) -> Result<(), SchemaError> {
    if value.is_empty() {
        Err(SchemaError::new(format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}

impl HeavyStepJobData {
    fn validate(&self, step_type: &str) -> Result<(), SchemaError> {
        non_empty("conversationId", &self.conversation_id)?;
        non_empty("contactInboxId", &self.contact_inbox_id)?;
        if let Some(key) = &self.outcome_key {
            non_empty("outcomeKey", key)?;
        }
        if let Some(c) = &self.continuation {
            non_empty("continuation.flowExecutionKey", &c.flow_execution_key)?;
            non_empty("continuation.flowId", &c.flow_id)?;
            if let Some(send_from) = &c.send_from
                && send_from != "inbox"
            {
                return Err(SchemaError::new("continuation.sendFrom must be inbox"));
            }
        }
        let Some(step) = self.step.as_object() else {
            return Err(SchemaError::new("step must be an object"));
        };
        if step.get("stepType").and_then(Value::as_str) != Some(step_type) {
            return Err(SchemaError::new(format!(
                "step.stepType must be \"{step_type}\""
            )));
        }
        match step.get("id") {
            Some(Value::String(id)) if !id.is_empty() => Ok(()),
            _ => Err(SchemaError::new("step.id must be a string")),
        }
    }
}

/// One `{ customFieldId, value }` output (both non-empty).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyOutputValue {
    pub custom_field_id: String,
    pub value: String,
}

/// `heavyStepResultSchema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum HeavyStepResult {
    /// At least one output value.
    #[serde(rename_all = "camelCase")]
    Success {
        output_values: Vec<HeavyOutputValue>,
    },
    /// Stray fields are dropped.
    Skip,
    #[serde(rename_all = "camelCase")]
    Error { error_message: String },
}

impl HeavyStepResult {
    /// `heavyStepResultSchema.parse`.
    pub fn parse(value: &Value) -> Result<HeavyStepResult, SchemaError> {
        let parsed: HeavyStepResult =
            serde_json::from_value(value.clone()).map_err(SchemaError::from_serde)?;
        match &parsed {
            HeavyStepResult::Success { output_values } => {
                if output_values.is_empty() {
                    return Err(SchemaError::new("outputValues must not be empty"));
                }
                for v in output_values {
                    non_empty("outputValues.customFieldId", &v.custom_field_id)?;
                    non_empty("outputValues.value", &v.value)?;
                }
            }
            HeavyStepResult::Skip => {}
            HeavyStepResult::Error { error_message } => {
                non_empty("errorMessage", error_message)?;
            }
        }
        Ok(parsed)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// `heavyExtractTextFromFileResultSchema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavyExtractTextFromFileResult {
    pub snippets: Vec<String>,
    pub truncated: bool,
}

impl HeavyExtractTextFromFileResult {
    pub fn parse(value: &Value) -> Result<HeavyExtractTextFromFileResult, SchemaError> {
        serde_json::from_value(value.clone()).map_err(SchemaError::from_serde)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// `heavyAnalyzeImageResultSchema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavyAnalyzeImageResult {
    pub analysis: String,
}

impl HeavyAnalyzeImageResult {
    pub fn parse(value: &Value) -> Result<HeavyAnalyzeImageResult, SchemaError> {
        serde_json::from_value(value.clone()).map_err(SchemaError::from_serde)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Stand-in for flow-config `aiEditImageDefaultFn(overrides)`: the default
/// `aiEditImage` step with `overrides` (a JSON object) applied.
pub fn ai_edit_image_default_step(overrides: Value) -> Value {
    default_step(
        "aiEditImage",
        serde_json::json!({ "provider": "openai", "model": "gpt-image-1", "inputFieldId": "", "prompt": "", "size": "auto", "quality": "auto", "outputFieldId": "" }),
        overrides,
        2,
    )
}

/// Stand-in for flow-config `aiSegmentImageDefaultFn(overrides)`.
pub fn ai_segment_image_default_step(overrides: Value) -> Value {
    default_step(
        "aiSegmentImage",
        serde_json::json!({ "inputFieldId": "", "target": "", "matchSelection": "largest", "outputFieldId": "" }),
        overrides,
        3,
    )
}

/// Stand-in for flow-config `aiGenerateImageDefaultFn(overrides)`.
pub fn ai_generate_image_default_step(overrides: Value) -> Value {
    default_step(
        "aiGenerateImage",
        serde_json::json!({ "provider": "openai", "model": "gpt-image-1", "prompt": "", "size": "auto", "quality": "auto", "outputFieldId": "" }),
        overrides,
        2,
    )
}

/// Stand-in for flow-config `AISpeechToTextDefaultFn(overrides)`.
pub fn ai_speech_to_text_default_step(overrides: Value) -> Value {
    default_step(
        "aiSpeechToText",
        serde_json::json!({ "inputFieldId": "", "outputFieldId": "" }),
        overrides,
        2,
    )
}

/// Stand-in for flow-config `AITextToSpeechDefaultFn(overrides)`.
pub fn ai_text_to_speech_default_step(overrides: Value) -> Value {
    default_step(
        "aiTextToSpeech",
        serde_json::json!({ "message": "", "outputFieldId": "" }),
        overrides,
        2,
    )
}

/// `{ id: createId(), ...defaults, ...overrides, stepType, states }` (the
/// shape of the flow-config `*DefaultFn`s).
fn default_step(step_type: &str, defaults: Value, overrides: Value, states: usize) -> Value {
    let mut step = serde_json::Map::new();
    step.insert("id".to_owned(), Value::String(crate::rand::numeric_id()));
    if let Value::Object(defaults) = defaults {
        step.extend(defaults);
    }
    if let Value::Object(overrides) = overrides {
        step.extend(overrides);
    }
    step.insert("stepType".to_owned(), Value::String(step_type.to_owned()));
    let kinds: &[&str] = if states == 3 {
        &["success", "skip", "error"]
    } else {
        &["success", "error"]
    };
    let states: Vec<Value> = kinds
        .iter()
        .map(|kind| serde_json::json!({ "id": crate::rand::numeric_id(), "type": kind }))
        .collect();
    step.insert("states".to_owned(), Value::Array(states));
    Value::Object(step)
}
