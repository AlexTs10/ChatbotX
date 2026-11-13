//! `pipeline/pipeline-structure.service.ts`: stages and custom fields.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{PipelineField, PipelineStage};
use chatbotx_db::repo::patch::Patch;

use super::access::PipelineAccessScope;
use crate::errors::Result;

/// `StageValues`; `Some(None)` clears a nullable column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageValues {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub text_color: Option<Option<String>>,
    pub background_color: Option<Option<String>>,
}

/// `PipelineFieldValues`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValues {
    pub name: String,
    pub reference_name: String,
    pub field_type: String,
    pub required: bool,
}

/// The `updateField` patch (the type is immutable).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldPatch {
    pub name: Option<String>,
    pub reference_name: Option<String>,
    pub required: Option<bool>,
}

/// `stageColumns`: a color must be `#rrggbb` (422 on the color key).
pub fn stage_columns(values: &StageValues) -> Result<Patch> {
    let _ = values;
    todo!("P3.6-D1p")
}

/// `createStage`: appended at the next position (at most 200 stages).
pub async fn create_stage(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    values: StageValues,
) -> Result<PipelineStage> {
    let _ = (db, workspace_id, pipeline_id, scope, values);
    todo!("P3.6-D1p")
}

/// `updateStage`.
pub async fn update_stage(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    scope: PipelineAccessScope,
    values: StageValues,
) -> Result<PipelineStage> {
    let _ = (db, workspace_id, pipeline_id, stage_id, scope, values);
    todo!("P3.6-D1p")
}

/// `reorderStages`: every stage exactly once.
pub async fn reorder_stages(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    stage_ids: Vec<i64>,
) -> Result<()> {
    let _ = (db, workspace_id, pipeline_id, scope, stage_ids);
    todo!("P3.6-D1p")
}

/// `createField`.
pub async fn create_field(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    values: FieldValues,
) -> Result<PipelineField> {
    let _ = (db, workspace_id, pipeline_id, scope, values);
    todo!("P3.6-D1p")
}

/// `updateField`.
pub async fn update_field(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    field_id: i64,
    scope: PipelineAccessScope,
    patch: FieldPatch,
) -> Result<PipelineField> {
    let _ = (db, workspace_id, pipeline_id, field_id, scope, patch);
    todo!("P3.6-D1p")
}

/// `deleteField`: deletes the row, then strips the key from item values.
pub async fn delete_field(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    field_id: i64,
    scope: PipelineAccessScope,
) -> Result<()> {
    let _ = (db, workspace_id, pipeline_id, field_id, scope);
    todo!("P3.6-D1p")
}
