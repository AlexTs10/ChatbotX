//! `pipeline/service.ts` + `pipeline-base.service.ts`: pipeline CRUD, access
//! checks and naming. No list cache, no template guards, no audit log
//! (see specs/P3.6-D1p.md).

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Pipeline, PipelineField, PipelineMember, PipelineStage};
use chatbotx_db::repo::partials::pipeline::{
    PIPELINE_LIMITS, PipelineSettings, PipelineType, parse_pipeline_settings,
    pipeline_type_defaults, reference_name_from_name,
};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::pipeline::{
    self as pipeline_repo, PipelineInsert, PipelineMemberInsert, PipelineMemberValues,
    PipelineNameColumn,
};
use chatbotx_db::repo::pipeline_stage::{self as stage_repo, PipelineStageInsert};
use chatbotx_db::repo::{Json, pipeline_activity, pipeline_item};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use super::access::{
    FULL_PIPELINE_PERMISSIONS, PipelineAccessScope, PipelineAction, PipelinePermissions,
    permits_pipeline_action, resolve_pipeline_permissions,
};
use super::currency::{PipelineView, view_of, views_of};
use super::naming::{SuffixStyle, allocate_unique_name};
use super::settings::{PipelineSettingsPatch, default_settings_for, merge_pipeline_settings};
use crate::errors::{BusinessError, Result};
use crate::tx;

const NOT_FOUND: &str = "Pipeline not found";
const NAME_TAKEN: &str = "This name is already used by another pipeline";

fn member_permissions(row: &PipelineMember) -> PipelinePermissions {
    PipelinePermissions {
        can_view: row.can_view,
        can_edit: row.can_edit,
        can_delete: row.can_delete,
        can_manage_settings: row.can_manage_settings,
    }
}

/// `permissionsFor` (no pipeline lookup).
pub fn permissions_in(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    scope: &PipelineAccessScope,
) -> Result<PipelinePermissions> {
    match scope {
        PipelineAccessScope::Member {
            workspace_member_id,
            is_workspace_admin: false,
            ..
        } => {
            let row =
                pipeline_repo::find_member(conn, workspace_id, pipeline_id, *workspace_member_id)?;
            Ok(resolve_pipeline_permissions(
                scope,
                row.as_ref().map(member_permissions),
            ))
        }
        _ => Ok(FULL_PIPELINE_PERMISSIONS),
    }
}

/// `requireAccessWithPermissions` on an open connection.
pub fn require_access_in(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    scope: &PipelineAccessScope,
    action: PipelineAction,
) -> Result<(Pipeline, PipelinePermissions)> {
    let pipeline = pipeline_repo::find_by_id(conn, workspace_id, pipeline_id)?
        .ok_or_else(|| BusinessError::not_found(NOT_FOUND))?;
    let permissions = permissions_in(conn, workspace_id, pipeline_id, scope)?;
    if !permissions.can_view {
        return Err(BusinessError::not_found(NOT_FOUND));
    }
    if !permits_pipeline_action(&permissions, action) {
        return Err(forbidden());
    }
    Ok((pipeline, permissions))
}

/// `403 forbidden`.
pub fn forbidden() -> BusinessError {
    BusinessError::exception(
        "You do not have permission to do this on this pipeline",
        "forbidden",
        403,
    )
}

/// The stored settings, every key backfilled (an unreadable value = defaults).
pub fn stored_settings(pipeline: &Pipeline) -> PipelineSettings {
    parse_pipeline_settings(&pipeline.settings.0).unwrap_or_default()
}

/// `PipelineType` as stored (`opportunity` / `ticket` / `custom`).
pub fn type_name(pipeline_type: PipelineType) -> &'static str {
    match pipeline_type {
        PipelineType::Opportunity => "opportunity",
        PipelineType::Ticket => "ticket",
        PipelineType::Custom => "custom",
    }
}

fn settings_json(settings: &PipelineSettings) -> Json {
    Json(serde_json::to_value(settings).unwrap_or_default())
}

fn utf16_prefix(text: &str, max: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(max).collect();
    String::from_utf16_lossy(&units)
}

/// `allocateNames`: name and reference name free in the workspace.
fn allocate_names(conn: &Connection, workspace_id: i64, name: &str) -> Result<(String, String)> {
    let base_name = utf16_prefix(name.trim(), PIPELINE_LIMITS.name);
    let base_reference = reference_name_from_name(&base_name, "pipeline");
    let names = pipeline_repo::list_names_starting_with(
        conn,
        workspace_id,
        PipelineNameColumn::Name,
        &utf16_prefix(&base_name, PIPELINE_LIMITS.name - 4),
    )?;
    let references = pipeline_repo::list_names_starting_with(
        conn,
        workspace_id,
        PipelineNameColumn::ReferenceName,
        &utf16_prefix(&base_reference, 60),
    )?;
    Ok((
        allocate_unique_name(
            &base_name,
            names.iter().map(String::as_str),
            SuffixStyle::Name,
        ),
        allocate_unique_name(
            &base_reference,
            references.iter().map(String::as_str),
            SuffixStyle::ReferenceName,
        ),
    ))
}

/// `assertNameFree`.
fn assert_name_free(
    conn: &Connection,
    workspace_id: i64,
    column: PipelineNameColumn,
    value: &str,
    except_id: i64,
) -> Result<()> {
    let (clash, field) = match column {
        PipelineNameColumn::Name => (
            pipeline_repo::find_by_name(conn, workspace_id, value)?,
            "name",
        ),
        PipelineNameColumn::ReferenceName => (
            pipeline_repo::find_by_reference_name(conn, workspace_id, value)?,
            "referenceName",
        ),
    };
    match clash {
        Some(row) if row.id != except_id => Err(BusinessError::validation(field, NAME_TAKEN)),
        _ => Ok(()),
    }
}

/// `assertNoBulkJob`.
fn assert_no_bulk_job(conn: &Connection, workspace_id: i64, pipeline_id: i64) -> Result<()> {
    let stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
    if stages.iter().any(|stage| stage.bulk_job_id.is_some()) {
        return Err(BusinessError::conflict(
            "A stage of this pipeline is being updated. Try again shortly.",
            "stageBusy",
        ));
    }
    Ok(())
}

const FULL_MEMBER: PipelineMemberValues = PipelineMemberValues {
    can_manage_settings: true,
    can_view: true,
    can_edit: true,
    can_delete: true,
};

/// The member id a plain (non-admin) member scope acts as.
pub fn plain_member_id(scope: &PipelineAccessScope) -> Option<i64> {
    match scope {
        PipelineAccessScope::Member {
            workspace_member_id,
            is_workspace_admin: false,
            ..
        } => Some(*workspace_member_id),
        _ => None,
    }
}

/// `membersForCopy`: the duplicating plain member gets full rights.
pub fn members_for_copy(
    workspace_id: i64,
    pipeline_id: i64,
    members: &[PipelineMember],
    scope: &PipelineAccessScope,
) -> Vec<PipelineMemberInsert> {
    let duplicator = plain_member_id(scope);
    let mut out: Vec<PipelineMemberInsert> = members
        .iter()
        .filter(|member| Some(member.workspace_member_id) != duplicator)
        .map(|member| PipelineMemberInsert {
            workspace_id,
            pipeline_id,
            workspace_member_id: member.workspace_member_id,
            values: PipelineMemberValues {
                can_manage_settings: member.can_manage_settings,
                can_view: member.can_view,
                can_edit: member.can_edit,
                can_delete: member.can_delete,
            },
        })
        .collect();
    if let Some(workspace_member_id) = duplicator {
        out.push(PipelineMemberInsert {
            workspace_id,
            pipeline_id,
            workspace_member_id,
            values: FULL_MEMBER,
        });
    }
    out
}

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
    tx::read(db, move |conn| {
        require_access_in(conn, workspace_id, pipeline_id, &scope, action)
    })
    .await
}

/// `listWithPermissions`: viewable pipelines oldest first with item counts.
pub async fn list_with_permissions(
    db: &Db,
    workspace_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineListEntry>> {
    let (rows, counts, access) = tx::read(db, move |conn| {
        let rows = pipeline_repo::list_by_workspace(conn, workspace_id)?;
        let counts = pipeline_repo::count_items_by_pipeline(conn, workspace_id)?;
        let access = match plain_member_id(&scope) {
            Some(member_id) => Some(pipeline_repo::list_member_access(
                conn,
                workspace_id,
                member_id,
            )?),
            None => None,
        };
        Ok((rows, counts, access))
    })
    .await?;
    let counts: HashMap<i64, i64> = counts
        .iter()
        .map(|c| (c.pipeline_id, c.items_count))
        .collect();
    let permissions_of = |pipeline_id: i64| match &access {
        None => FULL_PIPELINE_PERMISSIONS,
        Some(rows) => resolve_pipeline_permissions(
            &scope,
            rows.iter()
                .find(|row| row.pipeline_id == pipeline_id)
                .map(|row| PipelinePermissions {
                    can_view: row.can_view,
                    can_edit: row.can_edit,
                    can_delete: row.can_delete,
                    can_manage_settings: row.can_manage_settings,
                }),
        ),
    };
    let viewable: Vec<Pipeline> = rows
        .into_iter()
        .filter(|p| permissions_of(p.id).can_view)
        .collect();
    let views = views_of(db, workspace_id, viewable).await?;
    Ok(views
        .into_iter()
        .map(|view| PipelineListEntry {
            items_count: counts.get(&view.pipeline.id).copied().unwrap_or(0),
            permissions: permissions_of(view.pipeline.id),
            pipeline: view,
        })
        .collect())
}

/// `get`.
pub async fn get(
    db: &Db,
    workspace_id: i64,
    id: i64,
    scope: PipelineAccessScope,
) -> Result<PipelineDetail> {
    let (pipeline, permissions, stages, fields) = tx::read(db, move |conn| {
        let (pipeline, permissions) =
            require_access_in(conn, workspace_id, id, &scope, PipelineAction::View)?;
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, id)?;
        let fields = pipeline_repo::list_fields(conn, workspace_id, id)?;
        Ok((pipeline, permissions, stages, fields))
    })
    .await?;
    Ok(PipelineDetail {
        pipeline: view_of(db, workspace_id, pipeline).await?,
        stages,
        fields,
        permissions,
    })
}

/// `listFields`.
pub async fn list_fields(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<PipelineField>> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::View,
        )?;
        Ok(pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?)
    })
    .await
}

/// `create`: unique name / reference name, the type's first stage, a member
/// row for a plain member.
pub async fn create(db: &Db, input: CreatePipeline) -> Result<PipelineView> {
    let workspace_id = input.workspace_id;
    let created = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let (name, reference_name) = allocate_names(conn, workspace_id, &input.name)?;
        let pipeline = pipeline_repo::insert(
            conn,
            &PipelineInsert {
                workspace_id,
                name,
                r#type: type_name(input.pipeline_type).to_string(),
                reference_name,
                currency: None,
                settings: settings_json(&default_settings_for(input.pipeline_type)),
                created_by_id: input.actor_user_id,
            },
        )?;
        stage_repo::insert(
            conn,
            &PipelineStageInsert {
                workspace_id,
                pipeline_id: pipeline.id,
                name: pipeline_type_defaults(input.pipeline_type)
                    .first_stage_name
                    .to_string(),
                position: 0,
                ..PipelineStageInsert::default()
            },
        )?;
        if let Some(workspace_member_id) = plain_member_id(&input.scope) {
            pipeline_repo::upsert_member(
                conn,
                &PipelineMemberInsert {
                    workspace_id,
                    pipeline_id: pipeline.id,
                    workspace_member_id,
                    values: FULL_MEMBER,
                },
            )?;
        }
        Ok(pipeline)
    })
    .await?;
    view_of(db, workspace_id, created).await
}

/// `update`.
pub async fn update(db: &Db, input: UpdatePipeline) -> Result<PipelineView> {
    let UpdatePipeline {
        workspace_id,
        id,
        scope,
        name,
        reference_name,
        currency,
        settings,
    } = input;
    require_access(db, workspace_id, id, scope, PipelineAction::ManageSettings).await?;
    let updated = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let existing = pipeline_repo::find_by_id(conn, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found(NOT_FOUND))?;
        let mut patch: Patch = Vec::new();
        if let Some(name) = name {
            assert_name_free(conn, workspace_id, PipelineNameColumn::Name, &name, id)?;
            patch.push(("name", Value::Text(name)));
        }
        if let Some(reference_name) = reference_name {
            assert_name_free(
                conn,
                workspace_id,
                PipelineNameColumn::ReferenceName,
                &reference_name,
                id,
            )?;
            patch.push(("referenceName", Value::Text(reference_name)));
        }
        if let Some(currency) = currency {
            patch.push(("currency", currency.map_or(Value::Null, Value::Text)));
        }
        if let Some(settings) = settings {
            let merged = merge_pipeline_settings(&stored_settings(&existing), &settings);
            patch.push((
                "settings",
                Value::Text(settings_json(&merged).0.to_string()),
            ));
        }
        pipeline_repo::update(conn, workspace_id, id, &patch)?
            .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
    })
    .await?;
    view_of(db, workspace_id, updated).await
}

/// `delete` (items, activity, then the pipeline).
pub async fn delete(db: &Db, workspace_id: i64, id: i64, scope: PipelineAccessScope) -> Result<()> {
    require_access(db, workspace_id, id, scope, PipelineAction::Delete).await?;
    let limit = PIPELINE_LIMITS.bulk_batch_size as i64;
    for items in [true, false] {
        loop {
            let removed = tx::write(db, move |tx| {
                let conn: &Connection = tx;
                require_access_in(
                    conn,
                    workspace_id,
                    id,
                    &PipelineAccessScope::Support,
                    PipelineAction::View,
                )?;
                assert_no_bulk_job(conn, workspace_id, id)?;
                Ok(if items {
                    pipeline_item::delete_pipeline_items_batch(conn, workspace_id, id, limit)?
                } else {
                    pipeline_activity::delete_by_pipeline_batch(conn, workspace_id, id, limit)?
                })
            })
            .await?;
            if removed == 0 {
                break;
            }
        }
    }
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        assert_no_bulk_job(conn, workspace_id, id)?;
        if !pipeline_repo::delete(conn, workspace_id, id)? {
            return Err(BusinessError::not_found(NOT_FOUND));
        }
        Ok(())
    })
    .await
}

/// `duplicate`: stages, fields and members as `<name> copy`, never items.
pub async fn duplicate(
    db: &Db,
    workspace_id: i64,
    id: i64,
    scope: PipelineAccessScope,
    actor_user_id: Option<i64>,
) -> Result<PipelineView> {
    let (source, _) = require_access(db, workspace_id, id, scope, PipelineAction::View).await?;
    let copy = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, source.id)?;
        let fields = pipeline_repo::list_fields(conn, workspace_id, source.id)?;
        let members = pipeline_repo::list_members(conn, workspace_id, source.id)?;
        let (name, reference_name) =
            allocate_names(conn, workspace_id, &format!("{} copy", source.name))?;
        let pipeline = pipeline_repo::insert(
            conn,
            &PipelineInsert {
                workspace_id,
                name,
                r#type: source.r#type.clone(),
                reference_name,
                currency: source.currency.clone(),
                settings: source.settings.clone(),
                created_by_id: actor_user_id,
            },
        )?;
        let stage_rows: Vec<PipelineStageInsert> = stages
            .iter()
            .map(|stage| PipelineStageInsert {
                workspace_id,
                pipeline_id: pipeline.id,
                name: stage.name.clone(),
                description: stage.description.clone(),
                text_color: stage.text_color.clone(),
                background_color: stage.background_color.clone(),
                position: stage.position,
                bulk_job_id: None,
            })
            .collect();
        stage_repo::insert_many(conn, &stage_rows)?;
        let field_rows: Vec<pipeline_repo::PipelineFieldInsert> = fields
            .iter()
            .map(|field| pipeline_repo::PipelineFieldInsert {
                workspace_id,
                pipeline_id: pipeline.id,
                name: field.name.clone(),
                reference_name: field.reference_name.clone(),
                r#type: field.r#type.clone(),
                required: field.required,
                position: field.position,
            })
            .collect();
        pipeline_repo::insert_fields(conn, &field_rows)?;
        pipeline_repo::insert_members(
            conn,
            &members_for_copy(workspace_id, pipeline.id, &members, &scope),
        )?;
        Ok(pipeline)
    })
    .await?;
    view_of(db, workspace_id, copy).await
}

/// `canViewPipeline` (builder query): `None` = the viewer is not a member of
/// the workspace; a missing or forbidden pipeline is `false`.
pub async fn can_view_pipeline(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: Option<PipelineAccessScope>,
) -> Result<bool> {
    let Some(scope) = scope else {
        return Ok(false);
    };
    match get(db, workspace_id, pipeline_id, scope).await {
        Ok(_) => Ok(true),
        Err(error) if error.code() == Some("notFound") => Ok(false),
        Err(error) => Err(error),
    }
}
