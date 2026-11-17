//! Pipelines of the flow editor, the inbox panel and the pipelines list:
//! `features/pipelines/api/private.ts` (`privateListPipelineOptionsAPI`,
//! `privateGetPipelineAPI`, `privateListPipelinesAPI`) and
//! `features/pipelines/actions/pipeline.action.ts`. The member's pipeline
//! grants apply (`pipelineScopeFromMember`), not the workspace-token scope
//! of `/v1`.

use chatbotx_business::pipeline::access::{PipelineGrantPolicy, pipeline_grant_policy};
use chatbotx_business::pipeline::service::{self, stored_settings};
use chatbotx_business::pipeline::{PipelineAccessScope, PipelinePermissions, PipelineView};
use chatbotx_db::repo::find_many;
use chatbotx_db::repo::models::WorkspaceMember;
use chatbotx_db::repo::pipeline_stage as stage_repo;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail};
use crate::handlers::input::{Input, key};
use crate::handlers::pipelines::{
    CURRENCY, NAME, PIPELINE_TYPES, REFERENCE_NAME, pipeline_type, settings_patch,
};
use crate::orpc::OrpcError;

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "pipelinesAPI.privateListPipelineOptionsAPI" => options(ctx).await,
        "pipelinesAPI.privateGetPipelineAPI" => get(ctx, input).await,
        "pipelinesAPI.privateListPipelinesAPI" => list(ctx).await,
        _ => return None,
    })
}

pub(super) const ACTIONS: &[&str] = &[
    "createPipelineAction",
    "updatePipelineAction",
    "duplicatePipelineAction",
    "deletePipelineAction",
];

/// `pipeline.action.ts`: the input is `args[1]`; the member scope and the
/// user as the actor (`pipelineActionCaller`).
pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(
        async {
            let body = arg(args, 1).to_string();
            let mut input = Input::from_body(&[], body.as_bytes());
            let scope = member_scope(ctx).await?;
            let (db, ws) = (&ctx.api.db, ctx.workspace_id);
            match name {
                "createPipelineAction" => {
                    let kind = input.enum_at(
                        &key("type"),
                        input.get("type").cloned().as_ref(),
                        PIPELINE_TYPES,
                        true,
                    );
                    let name = input.string("name", NAME);
                    input.finish()?;
                    let view = service::create(
                        db,
                        service::CreatePipeline {
                            workspace_id: ws,
                            pipeline_type: pipeline_type(kind.as_deref().unwrap_or_default()),
                            name: name.unwrap_or_default(),
                            scope,
                            actor_user_id: Some(ctx.user_id),
                        },
                    )
                    .await
                    .map_err(action_fail)?;
                    Ok(json!({ "id": view.pipeline.id.to_string() }))
                }
                "updatePipelineAction" => {
                    let id = input.id("id");
                    let name = input.opt_string("name", NAME);
                    let reference_name = input.opt_string("referenceName", REFERENCE_NAME);
                    let currency = input.nullish_string("currency", CURRENCY);
                    let settings = settings_patch(&mut input);
                    input.finish()?;
                    service::update(
                        db,
                        service::UpdatePipeline {
                            workspace_id: ws,
                            id: id.unwrap_or_default(),
                            scope,
                            name,
                            reference_name,
                            currency,
                            settings,
                        },
                    )
                    .await
                    .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                "duplicatePipelineAction" => {
                    let id = input.id("id");
                    input.finish()?;
                    let copy = service::duplicate(
                        db,
                        ws,
                        id.unwrap_or_default(),
                        scope,
                        Some(ctx.user_id),
                    )
                    .await
                    .map_err(action_fail)?;
                    Ok(json!({ "id": copy.pipeline.id.to_string() }))
                }
                _ => {
                    let id = input.id("id");
                    input.finish()?;
                    service::delete(db, ws, id.unwrap_or_default(), scope)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}

/// `listWithPermissions`: `{ pipeline, itemsCount, permissions }` entries.
async fn list(ctx: &Ctx<'_>) -> Result<Value, OrpcError> {
    let scope = member_scope(ctx).await?;
    let entries = service::list_with_permissions(&ctx.api.db, ctx.workspace_id, scope)
        .await
        .map_err(fail)?;
    Ok(Value::Array(
        entries
            .iter()
            .map(|entry| {
                json!({
                    "pipeline": pipeline_with_currency(&entry.pipeline),
                    "itemsCount": entry.items_count,
                    "permissions": permissions_json(&entry.permissions),
                })
            })
            .collect(),
    ))
}

/// `pipelineScopeFromMember` (support sessions do not reach the SPA `/rpc`).
async fn member_scope(ctx: &Ctx<'_>) -> Result<PipelineAccessScope, OrpcError> {
    let (ws, user) = (ctx.workspace_id, ctx.user_id);
    let member = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let rows: Vec<WorkspaceMember> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 AND "userId" = ?2 LIMIT 1"#,
            &[&ws, &user],
        )?;
        Ok(rows.into_iter().next())
    })
    .await
    .map_err(fail)?
    .ok_or_else(|| {
        fail(chatbotx_business::BusinessError::not_found(
            "Workspace not found",
        ))
    })?;
    Ok(PipelineAccessScope::Member {
        workspace_member_id: member.id,
        is_workspace_admin: ctx.is_admin(),
        can_view_contact_email_and_phone: ctx.has("emailAndPhone"),
    })
}

/// `listWithStages`: the viewable pipelines with their stages.
async fn options(ctx: &Ctx<'_>) -> Result<Value, OrpcError> {
    let scope = member_scope(ctx).await?;
    let ws = ctx.workspace_id;
    let entries = service::list_with_permissions(&ctx.api.db, ws, scope)
        .await
        .map_err(fail)?;
    let ids: Vec<i64> = entries.iter().map(|e| e.pipeline.pipeline.id).collect();
    // ponytail: one stage query per pipeline; a workspace has few pipelines.
    let stages = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        ids.iter()
            .map(|id| Ok(stage_repo::list_by_pipeline(conn, ws, *id)?))
            .collect::<Result<Vec<_>, chatbotx_business::BusinessError>>()
    })
    .await
    .map_err(fail)?;
    let data: Vec<Value> = entries
        .iter()
        .zip(stages)
        .map(|(entry, stages)| {
            let pipeline = &entry.pipeline.pipeline;
            let settings = serde_json::to_value(stored_settings(pipeline)).unwrap_or_default();
            json!({
                "id": pipeline.id.to_string(),
                "name": pipeline.name,
                "referenceName": pipeline.reference_name,
                "statusLabels": settings["statusLabels"],
                "stages": stages
                    .iter()
                    .map(|s| json!({ "id": s.id.to_string(), "name": s.name }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "data": data }))
}

/// `pipelineService.get` + `grantPolicy`: the rows as models
/// (`PipelineWithCurrency`, stages, fields).
async fn get(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let id = In(input).id("pipelineId")?;
    let scope = member_scope(ctx).await?;
    let detail = service::get(&ctx.api.db, ctx.workspace_id, id, scope)
        .await
        .map_err(fail)?;
    Ok(json!({
        "pipeline": pipeline_with_currency(&detail.pipeline),
        "stages": detail.stages.iter().map(|s| s.to_json()).collect::<Vec<_>>(),
        "fields": detail.fields.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
        "permissions": permissions_json(&detail.permissions),
        "grantPolicy": match pipeline_grant_policy(&scope) {
            PipelineGrantPolicy::Unlimited => json!({ "limited": false }),
            PipelineGrantPolicy::Limited { self_member_id } => {
                json!({ "limited": true, "selfMemberId": self_member_id.to_string() })
            }
        },
    }))
}

/// `WithEffectiveCurrency<PipelineModel>`.
fn pipeline_with_currency(view: &PipelineView) -> Value {
    let mut out = view.pipeline.to_json();
    out["currency"] = json!(view.currency);
    out["currencyOverride"] = json!(view.currency_override);
    out
}

fn permissions_json(p: &PipelinePermissions) -> Value {
    json!({
        "canView": p.can_view,
        "canEdit": p.can_edit,
        "canDelete": p.can_delete,
        "canManageSettings": p.can_manage_settings,
    })
}
