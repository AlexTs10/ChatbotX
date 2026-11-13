//! `pipeline/pipeline-member.service.ts`: per-pipeline member grants.

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineMember;
use chatbotx_db::repo::pipeline::{
    self as pipeline_repo, PipelineMemberInsert, PipelineMemberValues,
};
use chatbotx_db::repo::pipeline_reference;
use chatbotx_db::rusqlite::Connection;

use super::access::{
    PipelineAccessScope, PipelineAction, PipelinePermissions, normalize_pipeline_permissions,
};
use super::service::{permissions_in, plain_member_id, require_access_in};
use crate::errors::{BusinessError, Result};
use crate::tx;

fn cannot_grant() -> BusinessError {
    BusinessError::exception(
        "You cannot grant permissions you do not hold or change your own",
        "forbidden",
        403,
    )
}

/// `assertCanGrant`.
fn assert_can_grant(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    scope: &PipelineAccessScope,
    target: i64,
    granted: &PipelinePermissions,
) -> Result<()> {
    let Some(self_id) = plain_member_id(scope) else {
        return Ok(());
    };
    let held = permissions_in(conn, workspace_id, pipeline_id, scope)?;
    let exceeds = (granted.can_view && !held.can_view)
        || (granted.can_edit && !held.can_edit)
        || (granted.can_delete && !held.can_delete)
        || (granted.can_manage_settings && !held.can_manage_settings);
    if exceeds || target == self_id {
        return Err(cannot_grant());
    }
    Ok(())
}

/// `listMembers` (manage-settings access).
pub async fn list_members(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineMember>> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::ManageSettings,
        )?;
        Ok(pipeline_repo::list_members(
            conn,
            workspace_id,
            pipeline_id,
        )?)
    })
    .await
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
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::ManageSettings,
        )?;
        if pipeline_reference::list_workspace_members(conn, workspace_id, &[workspace_member_id])?
            .is_empty()
        {
            return Err(BusinessError::not_found("Workspace member not found"));
        }
        let granted = normalize_pipeline_permissions(permissions);
        assert_can_grant(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            workspace_member_id,
            &granted,
        )?;
        Ok(pipeline_repo::upsert_member(
            conn,
            &PipelineMemberInsert {
                workspace_id,
                pipeline_id,
                workspace_member_id,
                values: PipelineMemberValues {
                    can_manage_settings: granted.can_manage_settings,
                    can_view: granted.can_view,
                    can_edit: granted.can_edit,
                    can_delete: granted.can_delete,
                },
            },
        )?)
    })
    .await
}

/// `removeMember`.
pub async fn remove_member(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    workspace_member_id: i64,
    scope: PipelineAccessScope,
) -> Result<()> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::ManageSettings,
        )?;
        if plain_member_id(&scope) == Some(workspace_member_id) {
            return Err(cannot_grant());
        }
        if !pipeline_repo::delete_member(conn, workspace_id, pipeline_id, workspace_member_id)? {
            return Err(BusinessError::not_found("Pipeline member not found"));
        }
        Ok(())
    })
    .await
}
