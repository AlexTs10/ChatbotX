//! `pipeline/pipeline-structure.service.ts`: stages and custom fields.

use std::collections::HashSet;

use chatbotx_db::Db;
use chatbotx_db::repo::models::{PipelineField, PipelineStage};
use chatbotx_db::repo::partials::pipeline::PIPELINE_LIMITS;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::pipeline::{self as pipeline_repo, PipelineFieldInsert, StripFieldKeyInput};
use chatbotx_db::repo::pipeline_item::{self as item_repo, MoveStageItemsInput};
use chatbotx_db::repo::pipeline_stage::{self as stage_repo, PipelineStageInsert};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use super::access::{PipelineAccessScope, PipelineAction};
use super::mutation::{ActivityContext, PipelineActor, activity_row_for};
use super::naming::next_field_position;
use super::service::require_access_in;
use crate::errors::{BusinessError, Result};
use crate::tx;

const DUPLICATE_FIELD_REFERENCE: &str = "A field with this reference name already exists";

/// `PIPELINE_LIMITS.colorPattern` (`/^#[0-9a-f]{6}$/i`).
pub fn is_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn opt_text(value: &Option<String>) -> Value {
    value.clone().map_or(Value::Null, Value::Text)
}

/// `structureTransaction`: one write holding the structure, after the
/// manage-settings check.
async fn with_structure<F, R>(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    work: F,
) -> Result<R>
where
    F: FnOnce(&Connection) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::ManageSettings,
        )?;
        work(conn)
    })
    .await
}

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
    let mut columns: Patch = Vec::new();
    if let Some(name) = &values.name {
        columns.push(("name", Value::Text(name.clone())));
    }
    if let Some(description) = &values.description {
        columns.push(("description", opt_text(description)));
    }
    for (key, column, color) in [
        ("textColor", "textColor", &values.text_color),
        (
            "backgroundColor",
            "backgroundColor",
            &values.background_color,
        ),
    ] {
        let Some(color) = color else { continue };
        if color.as_deref().is_some_and(|c| !is_color(c)) {
            return Err(BusinessError::validation(
                key,
                "Color must be a #rrggbb hex value",
            ));
        }
        columns.push((column, opt_text(color)));
    }
    Ok(columns)
}

/// `createStage`: appended at the next position (at most 200 stages).
pub async fn create_stage(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    values: StageValues,
) -> Result<PipelineStage> {
    stage_columns(&values)?;
    with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        let position = stage_repo::next_position(conn, workspace_id, pipeline_id)?;
        let max = PIPELINE_LIMITS.stages_per_pipeline as i64;
        if position >= max {
            return Err(BusinessError::validation(
                "name",
                format!("A pipeline can have at most {max} stages"),
            ));
        }
        Ok(stage_repo::insert(
            conn,
            &PipelineStageInsert {
                workspace_id,
                pipeline_id,
                name: values.name.unwrap_or_default(),
                description: values.description.flatten(),
                text_color: values.text_color.flatten(),
                background_color: values.background_color.flatten(),
                position,
                bulk_job_id: None,
            },
        )?)
    })
    .await
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
    let columns = stage_columns(&values)?;
    with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        stage_repo::update(conn, workspace_id, pipeline_id, stage_id, &columns)?
            .ok_or_else(|| BusinessError::not_found("Stage not found"))
    })
    .await
}

/// `reorderStages`: every stage exactly once.
pub async fn reorder_stages(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    stage_ids: Vec<i64>,
) -> Result<()> {
    with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        let current: HashSet<i64> = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?
            .iter()
            .map(|stage| stage.id)
            .collect();
        let sent: HashSet<i64> = stage_ids.iter().copied().collect();
        if sent.len() != stage_ids.len() || sent != current {
            return Err(BusinessError::validation(
                "stageIds",
                "stageIds must list every stage of the pipeline exactly once",
            ));
        }
        Ok(stage_repo::reorder(
            conn,
            workspace_id,
            pipeline_id,
            &stage_ids,
        )?)
    })
    .await
}

/// `createField`.
pub async fn create_field(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    values: FieldValues,
) -> Result<PipelineField> {
    with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        let existing = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
        if existing
            .iter()
            .any(|field| field.reference_name == values.reference_name)
        {
            return Err(BusinessError::validation(
                "referenceName",
                DUPLICATE_FIELD_REFERENCE,
            ));
        }
        let max = PIPELINE_LIMITS.fields_per_pipeline;
        if existing.len() >= max {
            return Err(BusinessError::validation(
                "name",
                format!("A pipeline can have at most {max} custom fields"),
            ));
        }
        Ok(pipeline_repo::insert_field(
            conn,
            &PipelineFieldInsert {
                workspace_id,
                pipeline_id,
                name: values.name,
                reference_name: values.reference_name,
                r#type: values.field_type,
                required: values.required,
                position: next_field_position(existing.iter().map(|field| field.position)),
            },
        )?)
    })
    .await
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
    with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        if let Some(reference_name) = &patch.reference_name {
            let existing = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
            if existing
                .iter()
                .any(|field| &field.reference_name == reference_name && field.id != field_id)
            {
                return Err(BusinessError::validation(
                    "referenceName",
                    DUPLICATE_FIELD_REFERENCE,
                ));
            }
        }
        let mut columns: Patch = Vec::new();
        if let Some(name) = patch.name {
            columns.push(("name", Value::Text(name)));
        }
        if let Some(reference_name) = patch.reference_name {
            columns.push(("referenceName", Value::Text(reference_name)));
        }
        if let Some(required) = patch.required {
            columns.push(("required", Value::Integer(i64::from(required))));
        }
        pipeline_repo::update_field(conn, workspace_id, pipeline_id, field_id, &columns)?
            .ok_or_else(|| BusinessError::not_found("Field not found"))
    })
    .await
}

/// `deleteField`: deletes the row, then strips the key from item values.
pub async fn delete_field(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    field_id: i64,
    scope: PipelineAccessScope,
) -> Result<()> {
    let deleted = with_structure(db, workspace_id, pipeline_id, scope, move |conn| {
        Ok(pipeline_repo::delete_field(
            conn,
            workspace_id,
            pipeline_id,
            field_id,
        )?)
    })
    .await?;
    // Strip the key even when the row was already gone (a retried delete).
    let limit = PIPELINE_LIMITS.bulk_batch_size;
    let mut after_id = None;
    loop {
        let batch = tx::write(db, move |tx| {
            Ok(pipeline_repo::strip_field_key_batch(
                tx,
                &StripFieldKeyInput {
                    workspace_id,
                    pipeline_id,
                    field_id: field_id.to_string(),
                    limit: limit as i64,
                    after_id,
                },
            )?)
        })
        .await?;
        if batch.scanned < limit || batch.last_id.is_none() {
            break;
        }
        after_id = batch.last_id;
    }
    if !deleted {
        return Err(BusinessError::not_found("Field not found"));
    }
    Ok(())
}

/// `DeleteStageResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteStageResult {
    pub queued: bool,
    pub job_id: Option<String>,
}

/// `pipelineStageBulkService.deleteStage`: the checks, the item drain (moved
/// or deleted, always in the request: no background job), the row delete and
/// the position compaction.
pub async fn delete_stage(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    move_items_to_stage_id: Option<i64>,
    scope: PipelineAccessScope,
    actor: PipelineActor,
) -> Result<DeleteStageResult> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let (_, permissions) = require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::ManageSettings,
        )?;
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
        let stage = stages
            .iter()
            .find(|stage| stage.id == stage_id)
            .ok_or_else(|| BusinessError::not_found("Stage not found"))?;
        if stages
            .iter()
            .any(|s| s.bulk_job_id.is_some() && s.id != stage_id)
        {
            return Err(BusinessError::conflict(
                "Another stage is being processed",
                "stageBusy",
            ));
        }
        if stages.len() <= 1 {
            return Err(BusinessError::validation(
                "stageId",
                "A pipeline needs at least one stage",
            ));
        }
        if let Some(target_id) = move_items_to_stage_id {
            let target = stages.iter().find(|s| s.id == target_id);
            match target {
                Some(target) if target.id != stage.id => {
                    if target.bulk_job_id.is_some() {
                        return Err(BusinessError::conflict(
                            "The destination stage is busy",
                            "stageBusy",
                        ));
                    }
                }
                _ => {
                    return Err(BusinessError::validation(
                        "moveItemsToStageId",
                        "Choose a different stage of this pipeline",
                    ));
                }
            }
        }
        if stage.bulk_job_id.is_some() {
            return Err(BusinessError::conflict(
                "This stage is already being processed",
                "stageBusy",
            ));
        }
        let count = chatbotx_db::repo::pipeline_item::count_by_stage(
            conn,
            workspace_id,
            pipeline_id,
            stage_id,
        )?;
        let needed = if move_items_to_stage_id.is_some() {
            PipelineAction::Edit
        } else {
            PipelineAction::Delete
        };
        if count > 0 && !super::access::permits_pipeline_action(&permissions, needed) {
            return Err(super::service::forbidden());
        }
        drain_stage(
            conn,
            workspace_id,
            pipeline_id,
            stage_id,
            move_items_to_stage_id,
            &actor,
        )?;
        stage_repo::delete(conn, workspace_id, pipeline_id, stage_id)?;
        let remaining: Vec<i64> = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?
            .iter()
            .map(|s| s.id)
            .collect();
        stage_repo::reorder(conn, workspace_id, pipeline_id, &remaining)?;
        Ok(DeleteStageResult {
            queued: false,
            job_id: None,
        })
    })
    .await
}

/// `drain`: moves (with a `stageChanged` row each) or deletes (with a
/// `deleted` row each) the stage's items in batches of `bulkSyncItems`.
fn drain_stage(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    move_to: Option<i64>,
    actor: &PipelineActor,
) -> Result<()> {
    let limit = PIPELINE_LIMITS.bulk_sync_items as i64;
    loop {
        let now = chatbotx_db::repo::now_ms();
        let rows = match move_to {
            Some(to_stage_id) => item_repo::move_stage_items_batch(
                conn,
                &MoveStageItemsInput {
                    workspace_id,
                    pipeline_id,
                    from_stage_id: stage_id,
                    to_stage_id,
                    limit,
                    now,
                },
            )?,
            None => item_repo::delete_stage_items_batch(
                conn,
                workspace_id,
                pipeline_id,
                stage_id,
                limit,
            )?,
        };
        if rows.is_empty() {
            return Ok(());
        }
        let activity: Vec<_> = rows
            .iter()
            .map(|item| {
                let context = ActivityContext {
                    item,
                    actor,
                    now,
                    from_pipeline_id: None,
                };
                let mut row = activity_row_for(
                    if move_to.is_some() {
                        "stageChanged"
                    } else {
                        "deleted"
                    },
                    &context,
                );
                row.from_stage_id = Some(stage_id);
                if move_to.is_some() {
                    row.to_stage_id = Some(item.stage_id);
                }
                row
            })
            .collect();
        chatbotx_db::repo::pipeline_activity::insert_many(conn, &activity)?;
    }
}
