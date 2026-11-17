//! `templatesAPI.listLockedIdsAPI` (`features/templates/api/index.ts`,
//! `listLockedTemplateResourceIds`): which ids of one list page are
//! view-only (`allowEdit` off) or undeletable (`allowDelete` off) template
//! copies. Access: the `/rpc` membership check (`workspaceAuthorizedMidddleware`).

use chatbotx_db::DbError;
use chatbotx_db::repo::template_installed_resource::{
    TEMPLATE_RESOURCE_KINDS, TemplateResourcePermission, find_blocking,
};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, invalid};

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    if path != "templatesAPI.listLockedIdsAPI" {
        return None;
    }
    Some(
        async {
            let kind = In(input).str("resourceKind", false, 1, 64)?;
            if !TEMPLATE_RESOURCE_KINDS.contains(&kind.as_str()) {
                return Err(invalid());
            }
            let ids = In(input).ids("resourceIds")?;
            if ids.len() > 200 {
                return Err(invalid());
            }
            let ws = ctx.workspace_id;
            let (locked, undeletable) = ctx
                .api
                .db
                .read(move |conn| {
                    let find = |permission| {
                        find_blocking(conn, ws, &kind, &ids, permission, &[])
                            .map_err(|e| DbError::other(e.to_string()))
                    };
                    Ok((
                        find(TemplateResourcePermission::AllowEdit)?,
                        find(TemplateResourcePermission::AllowDelete)?,
                    ))
                })
                .await
                .map_err(|e| crate::handlers::internal_error(&e.to_string()))?;
            let strings = |ids: Vec<i64>| ids.iter().map(i64::to_string).collect::<Vec<_>>();
            Ok(json!({
                "lockedIds": strings(locked),
                "undeletableIds": strings(undeletable),
            }))
        }
        .await,
    )
}
