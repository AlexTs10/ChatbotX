//! `pipeline/pipeline-member.service.ts`: per-pipeline member grants.

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineMember;

use super::access::{PipelineAccessScope, PipelinePermissions};
use crate::errors::Result;

/// `listMembers` (manage-settings access).
pub async fn list_members(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineMember>> {
    let _ = (db, workspace_id, pipeline_id, scope);
    todo!("P3.6-D1p")
}

/// `setMember`: normalized grants (edit, delete, manage imply view); a plain
/// member cannot grant more than they hold or change their own row.
pub async fn set_member(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
    scope: PipelineAccessScope,
    permissions: PipelinePermissions,
) -> Result<PipelineMember> {
    let _ = (
        db,
        workspace_id,
        pipeline_id,
        workspace_member_id,
        scope,
        permissions,
    );
    todo!("P3.6-D1p")
}

/// `removeMember`.
pub async fn remove_member(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
    scope: PipelineAccessScope,
) -> Result<()> {
    let _ = (db, workspace_id, pipeline_id, workspace_member_id, scope);
    todo!("P3.6-D1p")
}
