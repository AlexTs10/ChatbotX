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

use crate::options::JobOptions;

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
        todo!()
    }
}

/// `getHeavyJobCompletionWaitTimeoutMs(action, waitTimeoutMs)`: every paid
/// attempt (`wait_timeout_ms` each) plus the backoff between them plus the
/// 60 000 ms handoff (spec S8).
pub fn heavy_completion_wait_timeout_ms(action: HeavyJobAction, wait_timeout_ms: u64) -> u64 {
    todo!()
}

/// `getHeavyJobOptions(action)`: attempts, backoff and retention per action
/// (spec 1.2 row `heavy`).
pub fn heavy_job_options(action: HeavyJobAction) -> JobOptions {
    todo!()
}

/// A payload that does not match its schema (the zod `safeParse` failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
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
        todo!()
    }

    /// The JSON written to the queue.
    pub fn to_value(&self) -> Value {
        todo!()
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
        todo!()
    }

    pub fn to_value(&self) -> Value {
        todo!()
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
        todo!()
    }

    pub fn to_value(&self) -> Value {
        todo!()
    }
}

/// `heavyAnalyzeImageResultSchema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavyAnalyzeImageResult {
    pub analysis: String,
}

impl HeavyAnalyzeImageResult {
    pub fn parse(value: &Value) -> Result<HeavyAnalyzeImageResult, SchemaError> {
        todo!()
    }

    pub fn to_value(&self) -> Value {
        todo!()
    }
}

/// Stand-in for flow-config `aiEditImageDefaultFn(overrides)`: the default
/// `aiEditImage` step with `overrides` (a JSON object) applied.
pub fn ai_edit_image_default_step(overrides: Value) -> Value {
    todo!()
}

/// Stand-in for flow-config `aiSegmentImageDefaultFn(overrides)`.
pub fn ai_segment_image_default_step(overrides: Value) -> Value {
    todo!()
}

/// Stand-in for flow-config `aiGenerateImageDefaultFn(overrides)`.
pub fn ai_generate_image_default_step(overrides: Value) -> Value {
    todo!()
}

/// Stand-in for flow-config `AISpeechToTextDefaultFn(overrides)`.
pub fn ai_speech_to_text_default_step(overrides: Value) -> Value {
    todo!()
}

/// Stand-in for flow-config `AITextToSpeechDefaultFn(overrides)`.
pub fn ai_text_to_speech_default_step(overrides: Value) -> Value {
    todo!()
}
