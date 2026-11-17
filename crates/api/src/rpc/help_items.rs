//! The help item actions of the builder (`features/help-items/actions/*`,
//! Q.D11 slice 10) under the `/rpc` key `helpItemActions`: `createHelpItem`,
//! `updateHelpItem`, `deleteHelpItem` (all `authActionClient`; input
//! `{ scope, ...input }`, `scope` = the bound `helpItemScopeSchema`).

use chatbotx_business::tenant_help_item::{self as service, HelpItemData};
use chatbotx_db::repo::models::TenantHelpItem;
use serde_json::{Value, json};

use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::platform_credentials::{is_super_admin, unauthorized};
use crate::handlers::input::{Input, Num, Str, key};
use crate::handlers::{ApiState, date, id};

/// `ROOT_TENANT_ID`.
const ROOT_TENANT_ID: i64 = 1;

/// The actions of the `helpItemActions` key.
pub fn action_gate(name: &str) -> Option<Gate> {
    matches!(name, "createHelpItem" | "updateHelpItem" | "deleteHelpItem").then_some(Gate::Auth)
}

/// `resolveHelpItemTenantId(user, scope)`: `platform` = the root tenant for a
/// super admin; `tenant` = `resolveAdminTenantId(user, { requireActiveCloudTenant })`.
async fn resolve_tenant(api: &ApiState, user_id: i64, scope: &str) -> Result<i64, ActionError> {
    if scope == "platform" || !api.edition.is_cloud() {
        // Self-hosted `tenant` scope: only a super admin targets the root tenant.
        return if is_super_admin(api, user_id).await? {
            Ok(ROOT_TENANT_ID)
        } else {
            Err(unauthorized())
        };
    }
    let tenant = api
        .db
        .read(move |c| {
            chatbotx_db::repo::tenant::find_by_owner(c, user_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(|e| ActionError::Internal(e.to_string()))?;
    match tenant {
        Some(t) if t.status == "active" => Ok(t.id),
        _ => Err(unauthorized()),
    }
}

fn item_json(item: &TenantHelpItem) -> Value {
    json!({
        "id": id(item.id),
        "createdAt": date(item.created_at),
        "updatedAt": date(item.updated_at),
        "tenantId": id(item.tenant_id),
        "name": item.name,
        "url": item.url,
        "icon": item.icon,
        "position": item.position,
    })
}

/// `z.url({ protocol: /^(https?|mailto)$/ })`.
fn url_at(input: &mut Input) -> Option<String> {
    let text = input.string("url", Str::default())?;
    let ok =
        url::Url::parse(&text).is_ok_and(|u| matches!(u.scheme(), "http" | "https" | "mailto"));
    if !ok {
        input.issue(&key("url"), "invalid_format", "Invalid URL");
        return None;
    }
    Some(text)
}

/// `helpItemSchema` (defaults applied).
fn help_item(input: &mut Input) -> Option<HelpItemData> {
    let name = input.string("name", Str::trimmed(1, usize::MAX));
    let url = url_at(input);
    // `z.preprocess("" → null, z.string().trim().nullable().default(null))`.
    let icon = match input.get("icon").cloned() {
        Some(Value::String(s)) if s.is_empty() => Some(None),
        _ => Some(
            input
                .nullish_string(
                    "icon",
                    Str {
                        trim: true,
                        ..Str::default()
                    },
                )
                .flatten(),
        ),
    };
    let position = input
        .opt_coerce_int("position", Num::int(0.0, f64::MAX))
        .unwrap_or(0);
    Some(HelpItemData {
        name: name?,
        url: url?,
        icon: icon?,
        position,
    })
}

/// `helpItemIdSchema`: `z.string().min(1)`; a non-numeric id matches no row.
fn item_id(input: &mut Input) -> Option<i64> {
    let text = input.string(
        "id",
        Str {
            min: Some(1),
            ..Str::default()
        },
    )?;
    Some(text.parse().unwrap_or(-1))
}

/// Runs one help item action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    let scope = {
        let value = input.get("scope").cloned();
        input.enum_at(&key("scope"), value.as_ref(), &["tenant", "platform"], true)
    };
    let db = &api.db;
    match name {
        "createHelpItem" => {
            let data = help_item(&mut input);
            input.finish()?;
            let tenant = resolve_tenant(api, context.user_id, &scope.unwrap_or_default()).await?;
            let item = service::create(db, tenant, data.expect("validated")).await?;
            Ok(Some(item_json(&item)))
        }
        "updateHelpItem" => {
            let data = help_item(&mut input);
            let item = item_id(&mut input);
            input.finish()?;
            let tenant = resolve_tenant(api, context.user_id, &scope.unwrap_or_default()).await?;
            let item =
                service::update(db, item.unwrap_or(-1), tenant, data.expect("validated")).await?;
            Ok(Some(item_json(&item)))
        }
        "deleteHelpItem" => {
            let item = item_id(&mut input);
            input.finish()?;
            let tenant = resolve_tenant(api, context.user_id, &scope.unwrap_or_default()).await?;
            service::remove(db, item.unwrap_or(-1), tenant).await?;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!(
            "unknown help item action {name}"
        ))),
    }
}
