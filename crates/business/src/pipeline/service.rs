//! `pipeline/service.ts` + `pipeline-base.service.ts`: pipeline CRUD, access
//! checks and naming. No list cache, no template guards, no audit log
//! (see specs/P3.6-D1p.md).

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Pipeline, PipelineField, PipelineStage};
use chatbotx_db::repo::partials::pipeline::PipelineType;

use super::access::{PipelineAccessScope, PipelineAction, PipelinePermissions};
use super::currency::PipelineView;
use super::settings::PipelineSettingsPatch;
use crate::errors::Result;

/// `PipelineListEntryWithPermissions`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineListEntry {
    pub pipeline: PipelineView,
    pub items_count: i64,
    pub permissions: PipelinePermissions,
}

/// The result of `pipelineService.get`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineDetail {
    pub pipeline: PipelineView,
    pub stages: Vec<PipelineStage>,
    pub fields: Vec<PipelineField>,
    pub permissions: PipelinePermissions,
}

/// `CreatePipelineInput` (`actor.userId` = `actor_user_id`).
#[derive(Debug, Clone)]
pub struct CreatePipeline {
    pub workspace_id: i64,
    pub pipeline_type: PipelineType,
    pub name: String,
    pub scope: PipelineAccessScope,
    pub actor_user_id: Option<i64>,
}

/// `UpdatePipelineInput`; `currency: Some(None)` follows the workspace again.
#[derive(Debug, Clone)]
pub struct UpdatePipeline {
    pub workspace_id: i64,
    pub id: i64,
    pub scope: PipelineAccessScope,
    pub name: Option<String>,
    pub reference_name: Option<String>,
    pub currency: Option<Option<String>>,
    pub settings: Option<PipelineSettingsPatch>,
}

/// `requireAccessWithPermissions`: "Pipeline not found" when missing or not
/// viewable, `403 forbidden` when the action is not granted.
pub async fn require_access(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    action: PipelineAction,
) -> Result<(Pipeline, PipelinePermissions)> {
    let _ = (db, workspace_id, pipeline_id, scope, action);
    todo!("P3.6-D1p")
}

/// `listWithPermissions`: viewable pipelines oldest first with item counts.
pub async fn list_with_permissions(
    db: &Db,
    workspace_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineListEntry>> {
    let _ = (db, workspace_id, scope);
    todo!("P3.6-D1p")
}

/// `get`.
pub async fn get(
    db: &Db,
    workspace_id: i64,
    id: i64,
    scope: PipelineAccessScope,
) -> Result<PipelineDetail> {
    let _ = (db, workspace_id, id, scope);
    todo!("P3.6-D1p")
}

/// `listFields`.
pub async fn list_fields(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineField>> {
    let _ = (db, workspace_id, pipeline_id, scope);
    todo!("P3.6-D1p")
}

/// `create`: unique name / reference name, the type's first stage, a member
/// row for a plain member.
pub async fn create(db: &Db, input: CreatePipeline) -> Result<PipelineView> {
    let _ = (db, input);
    todo!("P3.6-D1p")
}

/// `update`.
pub async fn update(db: &Db, input: UpdatePipeline) -> Result<PipelineView> {
    let _ = (db, input);
    todo!("P3.6-D1p")
}

/// `delete` (items, activity, then the pipeline).
pub async fn delete(db: &Db, workspace_id: i64, id: i64, scope: PipelineAccessScope) -> Result<()> {
    let _ = (db, workspace_id, id, scope);
    todo!("P3.6-D1p")
}

/// `duplicate`: stages, fields and members as `<name> copy`, never items.
pub async fn duplicate(
    db: &Db,
    workspace_id: i64,
    id: i64,
    scope: PipelineAccessScope,
    actor_user_id: Option<i64>,
) -> Result<PipelineView> {
    let _ = (db, workspace_id, id, scope, actor_user_id);
    todo!("P3.6-D1p")
}

/// `canViewPipeline` (builder query): `None` = the viewer is not a member of
/// the workspace; a missing or forbidden pipeline is `false`.
pub async fn can_view_pipeline(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: Option<PipelineAccessScope>,
) -> Result<bool> {
    let _ = (db, workspace_id, pipeline_id, scope);
    todo!("P3.6-D1p")
}
