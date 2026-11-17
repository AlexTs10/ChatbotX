//! `features/templates/api/index.ts` (`templatesAPI`, Q.D4): the view-only /
//! undeletable lookup the SPA's lock badges and editor guard read. The
//! selectable-resources and dependency previews need the snapshot engine
//! (not ported yet).

use chatbotx_business::template::{RESOURCE_KINDS, list_locked_resource_ids};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, fail, invalid};

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    match path {
        "templatesAPI.listLockedIdsAPI" => Some(list_locked_ids(ctx, input).await),
        _ => None,
    }
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
    let (locked, undeletable) = list_locked_resource_ids(&ctx.api.db, ctx.workspace_id, &kind, &ids)
        .await
        .map_err(fail)?;
    Ok(json!({ "lockedIds": locked, "undeletableIds": undeletable }))
}
