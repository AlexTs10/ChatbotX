//! `features/templates/api/index.ts` (`templatesAPI`, Q.D4): the view-only /
//! undeletable lookup the SPA's lock badges and editor guard read, and the
//! template form's resource picker and dependency preview (they run the
//! matching `/v1` routes with the member actor).

use chatbotx_business::template::permissions::Actor;
use chatbotx_business::template::{RESOURCE_KINDS, list_locked_resource_ids};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, invalid};

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    match path {
        "templatesAPI.listLockedIdsAPI" => Some(list_locked_ids(ctx, input).await),
        "templatesAPI.listSelectableResourcesAPI" => {
            let params: Vec<(&'static str, String)> = ["category", "keyword", "cursor", "limit"]
                .into_iter()
                .filter_map(|key| match &input[key] {
                    Value::String(text) => Some((key, text.clone())),
                    Value::Number(n) => Some((key, n.to_string())),
                    _ => None,
                })
                .collect();
            Some(route(ctx, "templates.listSelectableResources", &params, b"").await)
        }
        "templatesAPI.previewDependenciesAPI" => {
            let body = json!({ "selection": input["selection"] }).to_string();
            Some(route(ctx, "templates.previewDependencies", &[], body.as_bytes()).await)
        }
        _ => None,
    }
}

/// `template-install-intent.action.ts` (`authActionClient`): remember the
/// share link a new workspace is created from, or end that choice. The
/// marker cookie is the SPA's (`install-intent-marker.ts`).
pub(super) async fn auth_action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    use chatbotx_business::template::install_intent;
    let kv = ctx.api.kv.as_deref().map(|kv| &kv.durable);
    match name {
        "rememberTemplateInstallIntentAction" => Some(
            async {
                let token = In(arg(args, 0)).str("shareToken", true, 1, usize::MAX)?;
                // Only a live link is remembered (one answer for every other).
                let live = chatbotx_business::template::request::find_public_by_share_token(
                    &ctx.api.db,
                    token.clone(),
                )
                .await
                .map_err(fail)?
                .is_some();
                if let (true, Some(kv)) = (live, kv) {
                    install_intent::remember(kv, ctx.user_id, &token).await;
                }
                Ok(Value::Null)
            }
            .await,
        ),
        "clearTemplateInstallIntentAction" => {
            if let Some(kv) = kv {
                install_intent::clear(kv, ctx.user_id).await;
            }
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

/// Runs a template `/v1` route as the member (`memberActor`).
async fn route(
    ctx: &Ctx<'_>,
    route_id: &str,
    params: &[(&'static str, String)],
    body: &[u8],
) -> Result<Value, crate::orpc::OrpcError> {
    crate::handlers::templates::handle_json(
        ctx.api,
        route_id,
        crate::handlers::Call {
            workspace_id: ctx.workspace_id,
            params,
            query: None,
            body,
        },
        Actor::Member(ctx.permissions.clone()),
        Some(ctx.user_id),
    )
    .await
}

/// `{ resourceKind, resourceIds (<= 200) }` → `{ lockedIds, undeletableIds }`.
async fn list_locked_ids(ctx: &Ctx<'_>, input: &Value) -> Result<Value, crate::orpc::OrpcError> {
    let input = In(input);
    let kind = input
        .opt_str("resourceKind")?
        .filter(|kind| RESOURCE_KINDS.contains(&kind.as_str()))
        .ok_or_else(invalid)?;
    let ids = input.ids("resourceIds")?;
    if ids.len() > 200 {
        return Err(invalid());
    }
    let (locked, undeletable) =
        list_locked_resource_ids(&ctx.api.db, ctx.workspace_id, &kind, &ids)
            .await
            .map_err(fail)?;
    Ok(json!({ "lockedIds": locked, "undeletableIds": undeletable }))
}
