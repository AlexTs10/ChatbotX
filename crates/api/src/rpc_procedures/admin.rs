//! The `/admin` console page reads (Q.F2 web run 7), as `POST
//! /rpc/actions/<name>` without a workspace. Each one re-checks the caller
//! like the original (`assertPlatformSuperAdmin`: a `PlatformAdmin` row,
//! else `notFound()`; the `(enterprise)` pages also `hasEnterpriseFeatures`):
//!
//! - `listPlatformAdmins(page)`: `platformAdminService.list({ page, perPage: 50 })`
//! - `listRootHelpItems()`: `tenantHelpItemService.listByTenant(ROOT_TENANT_ID)`
//! - `getRootDefaultTemplateSettings()`: `app/admin/default-template/page.tsx`
//!   (`tenantDefaultTemplateService.getSettings({ scope: root })`; not an
//!   `(enterprise)` page)

use chatbotx_business::BusinessError;
use chatbotx_db::repo::PaginationInput;
use chatbotx_db::repo::platform_admin::{self, ROOT_TENANT_ID};
use serde_json::{Value, json};

use super::{Ctx, Outcome, arg, fail, page};
use crate::handlers::{date, id};
use crate::orpc::OrpcError;

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !matches!(
        name,
        "listPlatformAdmins" | "listRootHelpItems" | "getRootDefaultTemplateSettings"
    ) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let user_id = ctx.user_id;
    let is_admin = chatbotx_business::tx::read(db, move |conn| {
        Ok(platform_admin::find_for_auth(conn, user_id)?.is_some())
    })
    .await
    .map_err(fail)?;
    // `notFound()`; the `(enterprise)` layout 404s without the features too.
    if !is_admin {
        return Err(fail(BusinessError::not_found("Not found")));
    }
    if name == "getRootDefaultTemplateSettings" {
        let Some(encryption) = ctx.api.encryption.as_deref() else {
            return Err(fail(BusinessError::exception(
                "encryption keys are not configured",
                "systemError",
                500,
            )));
        };
        return Ok(chatbotx_business::template::tenant_default::get_settings(
            db,
            encryption,
            chatbotx_business::template::tenant_default::Scope::Root,
        )
        .await
        .map_err(fail)?
        .unwrap_or(Value::Null));
    }
    if !chatbotx_business::license::has_enterprise_features(db, ctx.api.edition).await {
        return Err(fail(BusinessError::not_found("Not found")));
    }
    match name {
        "listPlatformAdmins" => {
            let page_number = arg(args, 0).as_i64().filter(|p| *p >= 1);
            let listed = chatbotx_business::tx::read(db, move |conn| {
                Ok(platform_admin::list_with_user(
                    conn,
                    PaginationInput {
                        page: page_number,
                        per_page: Some(50),
                    },
                )?)
            })
            .await
            .map_err(fail)?;
            Ok(page(
                listed.data.iter().map(|a| {
                    json!({
                        "id": id(a.id),
                        "createdAt": date(a.created_at),
                        "updatedAt": date(a.updated_at),
                        "userId": id(a.user_id),
                        "role": a.role,
                        "userEmail": a.user_email,
                        "userName": a.user_name,
                    })
                }),
                listed.page_count,
            ))
        }
        _ => Ok(
            chatbotx_business::tenant_help_item::list_by_tenant(db, ROOT_TENANT_ID)
                .await
                .map_err(fail)?
                .iter()
                .map(|item| item.to_json())
                .collect(),
        ),
    }
}
