//! Pipeline option lists of the flow editor and the inbox panel:
//! `features/pipelines/api/private.ts` (`privateListPipelineOptionsAPI`,
//! `privateGetPipelineAPI`). The member's pipeline grants apply
//! (`pipelineScopeFromMember`), not the workspace-token scope of `/v1`.

use chatbotx_business::pipeline::access::{PipelineGrantPolicy, pipeline_grant_policy};
use chatbotx_business::pipeline::service::{self, stored_settings};
use chatbotx_business::pipeline::{PipelineAccessScope, PipelinePermissions, PipelineView};
use chatbotx_db::repo::find_many;
use chatbotx_db::repo::models::WorkspaceMember;
use chatbotx_db::repo::pipeline_stage as stage_repo;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, fail};
use crate::orpc::OrpcError;

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "pipelinesAPI.privateListPipelineOptionsAPI" => options(ctx).await,
        "pipelinesAPI.privateGetPipelineAPI" => get(ctx, input).await,
        _ => return None,
    })
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
