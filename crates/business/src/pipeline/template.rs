//! `pipeline/pipeline-template.service.ts` (`pipelineTemplateService`):
//! template installs match-or-create a pipeline, its stages and fields.

use std::collections::{HashMap, VecDeque};

use chatbotx_db::repo::models::PipelineStage;
use chatbotx_db::repo::partials::pipeline::{
    PIPELINE_LIMITS, PipelineType, parse_pipeline_reference_name, parse_pipeline_settings,
    pipeline_type_defaults,
};
use chatbotx_db::repo::pipeline::{self as pipeline_repo, PipelineFieldInsert, PipelineInsert};
use chatbotx_db::repo::pipeline_stage::{self as stage_repo, PipelineStageInsert};
use chatbotx_db::rusqlite::Connection;
use serde_json::Value;

use super::naming::next_field_position;
use super::service::{allocate_names, settings_json};
use super::settings::default_settings_for;
use super::structure::is_color;
use crate::errors::{BusinessError, Result};

/// `stageKey`: names match case- and space-insensitively.
fn stage_key(name: &str) -> String {
    chatbotx_core::js::trim(name).to_lowercase()
}

/// `PIPELINE_LIMITS.currencyPattern` (`/^[A-Z]{3}$/`).
fn is_currency(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())
}

/// `validColor` (`colorPattern`: `#rrggbb`, any case).
fn valid_color(color: &Option<String>) -> Option<String> {
    color.clone().filter(|c| is_color(c))
}

fn pipeline_type(name: &str) -> Result<PipelineType> {
    match name {
        "opportunity" => Ok(PipelineType::Opportunity),
        "ticket" => Ok(PipelineType::Ticket),
        "custom" => Ok(PipelineType::Custom),
        _ => Err(BusinessError::validation("type", "Invalid pipeline type")),
    }
}

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
    conn: &Connection,
    workspace_id: i64,
    entry: &TemplatePipelineEntry,
) -> Result<TemplateEntryResolution> {
    let kind = pipeline_type(&entry.r#type)?;
    // The advisory locks of the original are implicit: one SQLite writer.
    let found = pipeline_repo::find_by_reference_name(conn, workspace_id, &entry.reference_name)?;
    let created = found.is_none();
    let pipeline = match found {
        Some(pipeline) => pipeline,
        None => {
            let (name, derived_reference) = allocate_names(conn, workspace_id, &entry.name)?;
            let settings = parse_pipeline_settings(&entry.settings)
                .unwrap_or_else(|_| default_settings_for(kind));
            let currency = entry.currency.clone().filter(|c| is_currency(c));
            let reference_name = if parse_pipeline_reference_name(&entry.reference_name).is_ok() {
                entry.reference_name.clone()
            } else {
                derived_reference
            };
            pipeline_repo::insert(
                conn,
                &PipelineInsert {
                    workspace_id,
                    name,
                    r#type: entry.r#type.clone(),
                    reference_name,
                    currency,
                    settings: settings_json(&settings),
                    created_by_id: None,
                },
            )?
        }
    };
    let pipeline_id = pipeline.id;
    let stages: Vec<PipelineStage> = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
    let mut stage_count = stages.len();
    if created && entry.stages.is_empty() && stage_count == 0 {
        stage_repo::insert(
            conn,
            &PipelineStageInsert {
                workspace_id,
                pipeline_id,
                name: pipeline_type_defaults(kind).first_stage_name.to_string(),
                position: 0,
                ..PipelineStageInsert::default()
            },
        )?;
    }
    // Each existing stage is matched at most once, in board order.
    let mut unclaimed: HashMap<String, VecDeque<i64>> = HashMap::new();
    for stage in &stages {
        unclaimed
            .entry(stage_key(&stage.name))
            .or_default()
            .push_back(stage.id);
    }
    let mut out = TemplateEntryResolution {
        pipeline_id,
        created,
        ..TemplateEntryResolution::default()
    };
    for source in &entry.stages {
        if let Some(existing) = unclaimed
            .get_mut(&stage_key(&source.name))
            .and_then(VecDeque::pop_front)
        {
            out.stage_ids.insert(source.source_id.clone(), existing);
            continue;
        }
        if stage_count >= PIPELINE_LIMITS.stages_per_pipeline {
            out.unmatched_stages.push(source.source_id.clone());
            continue;
        }
        let row = stage_repo::insert(
            conn,
            &PipelineStageInsert {
                workspace_id,
                pipeline_id,
                name: source.name.clone(),
                description: source.description.clone(),
                text_color: valid_color(&source.text_color),
                background_color: valid_color(&source.background_color),
                position: i64::try_from(stage_count).unwrap_or(i64::MAX),
                bulk_job_id: None,
            },
        )?;
        stage_count += 1;
        out.stage_ids.insert(source.source_id.clone(), row.id);
    }
    let fields = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
    let mut next_position = next_field_position(fields.iter().map(|f| f.position));
    let mut by_reference: HashMap<String, (i64, String)> = fields
        .into_iter()
        .map(|f| (f.reference_name, (f.id, f.r#type)))
        .collect();
    for source in &entry.fields {
        if let Some((existing_id, existing_type)) = by_reference.get(&source.reference_name) {
            if *existing_type == source.r#type {
                out.field_ids.insert(source.source_id.clone(), *existing_id);
            } else {
                out.unmatched_fields.push(source.source_id.clone());
            }
            continue;
        }
        if by_reference.len() >= PIPELINE_LIMITS.fields_per_pipeline {
            out.unmatched_fields.push(source.source_id.clone());
            continue;
        }
        let row = pipeline_repo::insert_field(
            conn,
            &PipelineFieldInsert {
                workspace_id,
                pipeline_id,
                name: source.name.clone(),
                reference_name: source.reference_name.clone(),
                r#type: source.r#type.clone(),
                required: source.required,
                position: next_position,
            },
        )?;
        next_position += 1;
        by_reference.insert(source.reference_name.clone(), (row.id, row.r#type));
        out.field_ids.insert(source.source_id.clone(), row.id);
    }
    Ok(out)
}
