//! `pipeline/pipeline-template.service.ts` (`pipelineTemplateService`):
//! template installs match-or-create a pipeline, its stages and fields.

use std::collections::HashMap;

use chatbotx_db::rusqlite::Connection;
use serde_json::Value;

use crate::errors::Result;

/// One stage of [`TemplatePipelineEntry`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateStage {
    pub source_id: String,
    pub name: String,
    pub description: Option<String>,
    pub text_color: Option<String>,
    pub background_color: Option<String>,
}

/// One field of [`TemplatePipelineEntry`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateField {
    pub source_id: String,
    pub name: String,
    pub reference_name: String,
    pub r#type: String,
    pub required: bool,
}

/// `TemplatePipelineManifestEntry` (parsed by
/// `template_pipeline_manifest_entry_schema`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemplatePipelineEntry {
    pub name: String,
    pub reference_name: String,
    /// `opportunity`, `ticket` or `custom`.
    pub r#type: String,
    pub currency: Option<String>,
    pub settings: Value,
    pub stages: Vec<TemplateStage>,
    pub fields: Vec<TemplateField>,
}

/// `resolveTemplateEntry` result: template source id -> local id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateEntryResolution {
    pub pipeline_id: i64,
    pub created: bool,
    pub stage_ids: HashMap<String, i64>,
    pub field_ids: HashMap<String, i64>,
    /// Stages over the per-pipeline limit.
    pub unmatched_stages: Vec<String>,
    /// Fields whose reference name exists with another type, or over the limit.
    pub unmatched_fields: Vec<String>,
}

/// `resolveTemplateEntry`: runs inside the template install transaction.
/// Finds the pipeline by reference name or creates it; matches stages by name
/// (trimmed, case-insensitive, each at most once, in board order) and fields
/// by reference name; adds the missing ones.
pub fn resolve_template_entry(
    _conn: &Connection,
    _workspace_id: i64,
    _entry: &TemplatePipelineEntry,
) -> Result<TemplateEntryResolution> {
    todo!("P3.6-D1p")
}
