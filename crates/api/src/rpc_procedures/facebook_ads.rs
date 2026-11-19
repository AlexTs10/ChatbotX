//! The Facebook Ads settings page (`features/integration-facebook-ads`):
//! `connectFacebookAds`, `disconnectFacebookAdsAction`, the page row and
//! the `integrationFacebookAdsAPI` reads. The connect is a signed connect
//! session (`connection::oauth_facebook_ads`).

use chatbotx_business::BusinessError;
use chatbotx_business::ads::graph;
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth_facebook_ads as facebook_ads;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, fail};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "connectFacebookAds",
    "disconnectFacebookAdsAction",
    "facebookAdsSettingsPage",
];

/// `assertWorkspaceSuperAdmin`.
fn assert_super_admin(ctx: &Ctx<'_>) -> Result<(), BusinessError> {
    if ctx.has("superAdmin") {
        return Ok(());
    }
    Err(BusinessError::exception(
        "You need to be a workspace super admin to do this.",
        "systemError",
        400,
    ))
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, _args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        match name {
            // The SPA cannot follow a server `redirect()`: it opens `url`.
            "connectFacebookAds" => {
                async {
                    assert_super_admin(ctx)?;
                    let encryption = ctx.api.encryption.as_deref().ok_or_else(|| {
                        BusinessError::exception(
                            "Facebook App settings are not valid",
                            "systemError",
                            400,
                        )
                    })?;
                    let url =
                        facebook_ads::start(db, encryption, &ctx.api.app_url, ws, ctx.user_id)
                            .await?;
                    Ok(json!({ "url": url }))
                }
                .await
            }
            "disconnectFacebookAdsAction" => {
                async {
                    assert_super_admin(ctx)?;
                    if !facebook_ads::disconnect(db, &HttpUpstream, ws).await? {
                        return Err(BusinessError::not_found(
                            "Facebook Ads integration not found",
                        ));
                    }
                    ctx.api
                        .audit
                        .audit("disconnect", "disconnected the Facebook Ads integration")
                        .await;
                    Ok(Value::Null)
                }
                .await
            }
            // The page: any member; the row carries no `auth`.
            _ => facebook_ads::settings_row(db, ws)
                .await
                .map(|row| json!({ "integration": row })),
        }
        .map_err(action_fail),
    )
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let result: Result<Value, OrpcError> = match path {
        "integrationFacebookAdsAPI.listAdAccounts" => graph::cached_ad_accounts(db, ws)
            .await
            .map(|data| json!({ "data": data }))
            .map_err(fail),
        "integrationFacebookAdsAPI.listCustomAudiences" => {
            let id = match In(input).str("adAccountId", true, 1, usize::MAX) {
                Ok(id) => id,
                Err(error) => return Some(Err(error)),
            };
            graph::cached_custom_audiences(db, ws, id)
                .await
                .map(|data| json!({ "data": data }))
                .map_err(fail)
        }
        _ => return None,
    };
    Some(result)
}
