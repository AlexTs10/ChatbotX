//! The ads screens (Q.D7): the private procedures of `adsAPI`,
//! `adsCampaignAPI`, `integrationFacebookAdsAPI`, `facebookLeadAdsAPI` and
//! `facebookMarketingMessagesAPI`, the page queries of `features/{ads,
//! ads-campaign,facebook-lead-ad-automation,facebook-marketing-messages,
//! integration-google-ads}` and their server actions. Where the builder
//! procedure or action runs the same service with the same schema as a ported
//! public `/v1` route, the handler runs that route ([`public`]).

use chatbotx_business::ads::{connect, graph, rules};
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::marketing_messages::{self as mm, campaigns};
use chatbotx_business::{BusinessError, google_ads_setup, tx};
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::params;
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, public, query_of, schema_errors};
use crate::orpc::OrpcError;

const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";

/// The page queries and server actions of this module.
pub(super) const ACTIONS: &[&str] = &[
    // queries
    "getAdsSwitcherData",
    "getConversionEventsData",
    "getAdsAnalytics",
    "getGoogleAdsStatsPage",
    "listMessagingAdsToolIntegrations",
    "listActiveMessagingAdsIntegrationIds",
    "checkMessagingAdsConnectionState",
    "listFacebookLeadAdsAutomations",
    "getFacebookLeadAdAutomation",
    "getMarketingMessagesPage",
    "getMarketingMessagePage",
    "resolveMarketingMessagesGrant",
    "resolveGoogleAdsDashboardEntry",
    // actions
    "createAdsConversionRuleAction",
    "updateAdsConversionRuleAction",
    "toggleAdsConversionRuleAction",
    "deleteAdsConversionRuleAction",
    "retargetAdAction",
    "disconnectMessagingAdsAction",
    "connectLeadAdsAction",
    "connectMarketingMessagesAction",
    "connectMessagingAdsAction",
    "createFacebookLeadAdAutomationAction",
    "updateFacebookLeadAdAutomationAction",
    "deleteFacebookLeadAdAutomationsAction",
    "createMarketingMessageAction",
    "updateMarketingMessageAction",
    "deleteMarketingMessagesAction",
    "stopMarketingMessageAction",
    "resumeMarketingMessageAction",
    "sendMarketingMessageAction",
    "createAudienceAction",
    "uploadMoreAudienceAction",
    "deleteAudienceAction",
];

/// `assertWorkspaceSuperAdmin`.
fn super_admin(ctx: &Ctx<'_>) -> Result<(), BusinessError> {
    if ctx.has("superAdmin") {
        Ok(())
    } else {
        Err(BusinessError::exception(
            SUPER_ADMIN_REQUIRED,
            "systemError",
            400,
        ))
    }
}

/// The input without `workspaceId` and without empty strings (the builder
/// services read `""` as "not set").
fn strip(input: &Value) -> Value {
    let mut out = Map::new();
    if let Some(object) = input.as_object() {
        for (key, value) in object {
            if key != "workspaceId" && value != "" && !value.is_null() {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(out)
}

/// A GET public route with the input's `keys` as the query.
async fn get(ctx: &Ctx<'_>, route: &str, input: &Value, keys: &[&str]) -> Result<Value, OrpcError> {
    let query = query_of(&strip(input), keys);
    public(ctx, route, &[], Some(&query), None).await
}

const INTEGRATION_KEYS: &[&str] = &["channel", "integrationId", "refresh"];

/// Runs the procedure at `path`; `None` = not this module's.
pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let admin_route = |route: &'static str| async move {
        super_admin(ctx).map_err(fail)?;
        public(ctx, route, &[], None, Some(&strip(input))).await
    };
    let operation = |route: &'static str| async move {
        super_admin(ctx).map_err(fail)?;
        let id = In(input).id("operationId")?.to_string();
        public(ctx, route, &[("id", id)], Some(""), Some(&json!({}))).await
    };
    Some(match path {
        "adsAPI.listRules" => async {
            super_admin(ctx)?;
            let channel = In(input).opt_str("channel").ok().flatten();
            rules::list(db, ws, channel)
                .await
                .map(|data| json!({ "data": data }))
        }
        .await
        .map_err(fail),
        "adsAPI.listChannelAdAccounts" => async {
            super_admin(ctx)?;
            let channel = In(input)
                .opt_str("channel")
                .ok()
                .flatten()
                .unwrap_or_default();
            let integration_id = In(input).opt_id("integrationId").ok().flatten();
            let accounts = graph::channel_ad_accounts(db, ws, &channel, integration_id).await?;
            let data: Vec<Value> = accounts.into_iter().map(|a| a.account).collect();
            Ok::<_, BusinessError>(json!({ "data": data }))
        }
        .await
        .map_err(fail),
        "adsAPI.getReferralStats" => match super_admin(ctx) {
            Ok(()) => {
                get(
                    ctx,
                    "ads.referralStats",
                    input,
                    &["from", "to", "tz", "channel", "limit"],
                )
                .await
            }
            Err(error) => Err(fail(error)),
        },
        "integrationFacebookAdsAPI.listAdAccounts" => graph::cached_ad_accounts(db, ws)
            .await
            .map(|data| json!({ "data": data }))
            .map_err(fail),
        "integrationFacebookAdsAPI.listCustomAudiences" => {
            match In(input).str("adAccountId", true, 1, usize::MAX) {
                Ok(account) => graph::cached_custom_audiences(db, ws, account)
                    .await
                    .map(|data| json!({ "data": data }))
                    .map_err(fail),
                Err(error) => Err(error),
            }
        }
        "adsCampaignAPI.createMessagingAd" => admin_route("ads.createCampaign").await,
        "adsCampaignAPI.retryMessagingAd" => operation("ads.retryCampaign").await,
        "adsCampaignAPI.publishMessagingAd" => operation("ads.publishCampaign").await,
        "adsCampaignAPI.pauseMessagingAd" => operation("ads.pauseCampaign").await,
        "adsCampaignAPI.deleteMessagingAd" => operation("ads.deleteCampaign").await,
        "adsCampaignAPI.listMessagingAds" => {
            get(ctx, "ads.listCampaigns", input, INTEGRATION_KEYS).await
        }
        "adsCampaignAPI.getMessagingAdsInsights" => {
            public(
                ctx,
                "ads.getCampaignsInsights",
                &[],
                None,
                Some(&strip(input)),
            )
            .await
        }
        "adsCampaignAPI.listAdAccounts" => {
            get(ctx, "ads.listCampaignAdAccounts", input, INTEGRATION_KEYS).await
        }
        "adsCampaignAPI.getAdAccountDetails" => {
            let keys = ["adAccountId", "channel", "integrationId", "refresh"];
            get(ctx, "ads.getCampaignAdAccountDetails", input, &keys).await
        }
        "adsCampaignAPI.uploadAdVideo" => admin_route("ads.uploadCampaignVideo").await,
        "adsCampaignAPI.getAdVideoStatus" => {
            let keys = ["videoId", "channel", "integrationId"];
            get(ctx, "ads.getCampaignVideoStatus", input, &keys).await
        }
        "adsCampaignAPI.listMessengerPages" => {
            get(
                ctx,
                "ads.listCampaignMessengerPages",
                input,
                &["channel", "integrationId"],
            )
            .await
        }
        "adsCampaignAPI.checkPrerequisites" => get(
            ctx,
            "ads.checkCampaignPrerequisites",
            input,
            &["channel", "integrationId"],
        )
        .await
        .map(|body| json!({ "connected": body["connected"] })),
        "facebookLeadAdsAPI.listLeadAdsPagesAPI" => {
            public(ctx, "facebookLeadAds.listPages", &[], Some(""), None).await
        }
        "facebookLeadAdsAPI.listLeadAdsFormsAPI" => {
            get(ctx, "facebookLeadAds.listForms", input, &["pageId"]).await
        }
        "facebookMarketingMessagesAPI.getMarketingMessagesStatisticsAPI" => {
            let keys = [
                "adAccountId",
                "since",
                "until",
                "search",
                "page",
                "perPage",
                "sort",
                "order",
            ];
            get(
                ctx,
                "facebookMarketingMessages.statistics.get",
                input,
                &keys,
            )
            .await
        }
        "facebookMarketingMessagesAPI.listMarketingMessagesPagesAPI" => {
            get(ctx, "facebookMarketingMessages.pages.list", input, &[]).await
        }
        "facebookMarketingMessagesAPI.listMarketingMessagesAdAccountsAPI" => {
            get(ctx, "facebookMarketingMessages.adAccounts.list", input, &[]).await
        }
        "facebookMarketingMessagesAPI.listMarketingMessagesAudiencesAPI" => {
            let keys = ["adAccountId", "pageId", "readyOnly"];
            get(
                ctx,
                "facebookMarketingMessages.audiences.list",
                input,
                &keys,
            )
            .await
        }
        "facebookMarketingMessagesAPI.listMarketingMessagesAudiencesLiveAPI" => {
            get(
                ctx,
                "facebookMarketingMessages.audiences.liveStatus",
                input,
                &[],
            )
            .await
        }
        _ => return None,
    })
}

/// Runs the query or action `name`; `None` = not this module's.
pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let input = arg(args, 1);
    let bound = || {
        In(&json!({ "id": arg(args, 1) }))
            .id("id")
            .map(|id| id.to_string())
    };
    Some(match name {
        "getAdsSwitcherData" => switcher(ctx).await.map_err(fail),
        "resolveGoogleAdsDashboardEntry" => google_entry(ctx).await.map(Value::Bool).map_err(fail),
        "getConversionEventsData" => conversion_events(ctx).await.map_err(fail),
        "getAdsAnalytics" => analytics(ctx, input).await,
        "getGoogleAdsStatsPage" => google_stats(ctx, input).await,
        "listMessagingAdsToolIntegrations" => tool_integrations(ctx, &channel_of(arg(args, 0)))
            .await
            .map_err(fail),
        "listActiveMessagingAdsIntegrationIds" => {
            active_integration_ids(ctx, &channel_of(arg(args, 0)))
                .await
                .map_err(fail)
        }
        "checkMessagingAdsConnectionState" => connection_state(ctx, arg(args, 0)).await,
        "listFacebookLeadAdsAutomations" => {
            let query = super::tools::list_query(&strip(arg(args, 0)), &["keyword"]);
            public(ctx, "facebookLeadAds.list", &[], Some(&query), None).await
        }
        "getFacebookLeadAdAutomation" => match bound() {
            Ok(id) => public(ctx, "facebookLeadAds.get", &[("id", id)], Some(""), None)
                .await
                .map(|mut row| {
                    row["workspaceId"] = json!(ctx.workspace_id.to_string());
                    row
                }),
            Err(error) => Err(error),
        },
        "resolveMarketingMessagesGrant" => mm::grant_state(&ctx.api.db, ctx.workspace_id)
            .await
            .map(|state| json!({ "state": state }))
            .map_err(fail),
        "getMarketingMessagesPage" => marketing_messages_page(ctx).await.map_err(fail),
        "getMarketingMessagePage" => match bound() {
            Ok(id) => marketing_message_page(ctx, &id).await.map_err(fail),
            Err(error) => Err(error),
        },
        "createAdsConversionRuleAction" => rule_action(ctx, "ads.createRule", input).await,
        "updateAdsConversionRuleAction" => rule_action(ctx, "ads.updateRuleLegacy", input).await,
        "toggleAdsConversionRuleAction" => rule_action(ctx, "ads.toggleRuleStatus", input).await,
        "deleteAdsConversionRuleAction" => rule_action(ctx, "ads.deleteRule", input)
            .await
            .map(|_| Value::Null),
        "retargetAdAction" => retarget(ctx, input).await,
        "disconnectMessagingAdsAction" => disconnect_messaging_ads(ctx, input).await,
        "createFacebookLeadAdAutomationAction" => {
            public(ctx, "facebookLeadAds.create", &[], None, Some(input))
                .await
                .map(|row| json!({ "id": row["id"] }))
                .map_err(schema_errors)
        }
        "updateFacebookLeadAdAutomationAction" => match bound() {
            Ok(id) => public(
                ctx,
                "facebookLeadAds.update",
                &[("id", id)],
                None,
                Some(arg(args, 2)),
            )
            .await
            .map(|_| Value::Null)
            .map_err(schema_errors),
            Err(error) => Err(error),
        },
        "deleteFacebookLeadAdAutomationsAction" => {
            delete_each(ctx, "facebookLeadAds.delete", input).await
        }
        "createMarketingMessageAction" => public(
            ctx,
            "facebookMarketingMessages.campaigns.create",
            &[],
            None,
            Some(input),
        )
        .await
        .map_err(schema_errors),
        "updateMarketingMessageAction" => match bound() {
            Ok(id) => {
                let mut body = arg(args, 2).clone();
                if let Some(object) = body.as_object_mut() {
                    object.insert("id".into(), json!(id));
                }
                public(
                    ctx,
                    "facebookMarketingMessages.campaigns.update",
                    &[],
                    None,
                    Some(&body),
                )
                .await
                .map_err(schema_errors)
            }
            Err(error) => Err(error),
        },
        "deleteMarketingMessagesAction" => {
            delete_each(ctx, "facebookMarketingMessages.campaigns.delete", input).await
        }
        "stopMarketingMessageAction"
        | "resumeMarketingMessageAction"
        | "sendMarketingMessageAction" => {
            let route = match name {
                "stopMarketingMessageAction" => "facebookMarketingMessages.campaigns.stop",
                "resumeMarketingMessageAction" => "facebookMarketingMessages.campaigns.resume",
                _ => "facebookMarketingMessages.campaigns.send",
            };
            match bound() {
                Ok(id) => public(ctx, route, &[("id", id)], Some(""), None).await,
                Err(error) => Err(error),
            }
        }
        "createAudienceAction" => public(
            ctx,
            "facebookMarketingMessages.audiences.create",
            &[],
            None,
            Some(input),
        )
        .await
        .map_err(schema_errors),
        "uploadMoreAudienceAction" => {
            let id = input["id"].as_str().unwrap_or_default().to_owned();
            public(
                ctx,
                "facebookMarketingMessages.audiences.uploadMore",
                &[("id", id)],
                None,
                Some(input),
            )
            .await
            .map_err(schema_errors)
        }
        "connectLeadAdsAction" => {
            let path = format!("/space/{}/fb-lead-ads", ctx.workspace_id);
            connect(ctx, connect::Flow::LeadAds, path).await
        }
        "connectMarketingMessagesAction" => {
            let path = format!("/space/{}/fb-marketing-messages", ctx.workspace_id);
            connect(ctx, connect::Flow::MarketingMessages, path).await
        }
        "connectMessagingAdsAction" => match (bound(), super_admin(ctx)) {
            (Ok(id), Ok(())) => match channel_of(arg(args, 2)).as_str() {
                channel @ ("whatsapp" | "messenger" | "instagram") => {
                    // `buildMessagingAdsToolPath({ workspaceId, channel, integrationId })`.
                    let path = format!(
                        "/space/{}/messaging-ads/{channel}?integration={id}",
                        ctx.workspace_id
                    );
                    let flow = connect::Flow::MessagingAds {
                        channel: channel.to_owned(),
                        integration_id: id.parse().unwrap_or_default(),
                    };
                    connect(ctx, flow, path).await
                }
                _ => Err(action_fail(BusinessError::validation(
                    "channel",
                    "Invalid option",
                ))),
            },
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(action_fail(error)),
        },
        _ => {
            // deleteAudienceAction
            let id = input["id"].as_str().unwrap_or_default().to_owned();
            public(
                ctx,
                "facebookMarketingMessages.audiences.delete",
                &[("id", id)],
                Some(""),
                None,
            )
            .await
            .map(|_| Value::Null)
        }
    })
}

/// A Meta ads connect action: the Facebook dialog URL as `{ redirectTo }`
/// (the builder's `redirect(authUrl)`).
async fn connect(ctx: &Ctx<'_>, flow: connect::Flow, path: String) -> Result<Value, OrpcError> {
    let encryption = ctx.api.encryption.as_deref().ok_or_else(|| {
        action_fail(BusinessError::exception(
            "Facebook App settings are not valid",
            "systemError",
            400,
        ))
    })?;
    connect::start(
        &ctx.api.db,
        encryption,
        &ctx.api.app_url,
        ctx.workspace_id,
        ctx.user_id,
        flow,
        path,
    )
    .await
    .map(|url| json!({ "redirectTo": url }))
    .map_err(action_fail)
}

fn channel_of(input: &Value) -> String {
    input["channel"].as_str().unwrap_or_default().to_owned()
}

/// A conversion-rule action: super admin, then the public route.
async fn rule_action(ctx: &Ctx<'_>, route: &str, input: &Value) -> Result<Value, OrpcError> {
    super_admin(ctx).map_err(action_fail)?;
    let id = In(input)
        .opt_id("id")
        .ok()
        .flatten()
        .map(|id| id.to_string());
    let params: Vec<(&'static str, String)> = id.into_iter().map(|id| ("id", id)).collect();
    match route {
        "ads.deleteRule" => public(ctx, route, &params, Some(""), None).await,
        _ => public(ctx, route, &params, None, Some(input)).await,
    }
    .map_err(schema_errors)
}

/// `retargetAdAction` (`audienceName` or `customAudienceId` required).
async fn retarget(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    super_admin(ctx).map_err(action_fail)?;
    let named = |key: &str| input[key].as_str().is_some_and(|s| !s.is_empty());
    if !(named("audienceName") || named("customAudienceId")) {
        let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
        error.data = Some(json!({
            "validationErrors": {
                "audienceName": { "_errors": ["audienceName or customAudienceId is required"] },
            }
        }));
        return Err(error);
    }
    public(ctx, "ads.startRetargetAudienceSync", &[], None, Some(input))
        .await
        .map_err(schema_errors)
}

/// `disconnectMessagingAdsAction` (super admin).
async fn disconnect_messaging_ads(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    super_admin(ctx).map_err(action_fail)?;
    let query = query_of(input, &["channel", "integrationId"]);
    public(ctx, "ads.disconnectConnection", &[], Some(&query), None)
        .await
        .map(|_| Value::Null)
}

/// A bulk delete (`{ ids }`) as one public delete per id.
async fn delete_each(ctx: &Ctx<'_>, route: &str, input: &Value) -> Result<Value, OrpcError> {
    for id in In(input).strings("ids")? {
        public(ctx, route, &[("id", id)], Some(""), None).await?;
    }
    Ok(Value::Null)
}

/// `(id, name)` rows of a workspace's integrations of one channel table.
async fn integrations(ctx: &Ctx<'_>, sql: &'static str) -> Result<Vec<Value>, BusinessError> {
    let ws = ctx.workspace_id;
    tx::read(&ctx.api.db, move |conn| {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![ws], |r| {
                let columns = r.as_ref().column_count();
                let mut row = Map::new();
                for index in 0..columns {
                    let name = r.as_ref().column_name(index)?.to_owned();
                    let value: chatbotx_db::rusqlite::types::Value = r.get(index)?;
                    use chatbotx_db::rusqlite::types::Value as V;
                    let json = match value {
                        V::Integer(n) if name == "id" || name.ends_with("Id") => {
                            json!(n.to_string())
                        }
                        V::Integer(n) if name.starts_with("is") || name.starts_with("has") => {
                            json!(n != 0)
                        }
                        V::Integer(n) => json!(n),
                        V::Text(text) => json!(text),
                        _ => Value::Null,
                    };
                    row.insert(name, json);
                }
                Ok(Value::Object(row))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

const WHATSAPP_SQL: &str = r#"SELECT "id", "name", "displayPhoneNumber", "inboxId", "hasCapiScope", "isCoexist"
    FROM "IntegrationWhatsapp" WHERE "workspaceId" = ?1 ORDER BY "id""#;
const MESSENGER_SQL: &str =
    r#"SELECT "id", "name" FROM "IntegrationMessenger" WHERE "workspaceId" = ?1 ORDER BY "id""#;
const INSTAGRAM_SQL: &str = r#"SELECT "id", "name", "type" FROM "IntegrationInstagram" WHERE "workspaceId" = ?1 ORDER BY "id""#;

fn id_name(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|r| json!({ "id": r["id"], "name": r["name"] }))
        .collect()
}

/// `getAdsSwitcherData`.
async fn switcher(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let created_at: i64 = tx::read(db, move |conn| {
        Ok(conn.query_row(
            r#"SELECT "createdAt" FROM "Workspace" WHERE "id" = ?1"#,
            [ws],
            |r| r.get(0),
        )?)
    })
    .await?;
    let whatsapp_public = match ctx.api.encryption.clone() {
        Some(encryption) => {
            let owner =
                chatbotx_business::tenant::resolve_credential_owner_id_for_workspace(db, ws, None)
                    .await?;
            tx::read(db, move |conn| {
                Ok(chatbotx_business::platform_credential::resolve_for_owner(
                    conn,
                    &encryption,
                    owner,
                    "whatsapp",
                    false,
                )?
                .map(|c| c.public_config))
            })
            .await?
        }
        None => None,
    };
    let whatsapp = integrations(ctx, WHATSAPP_SQL).await?;
    Ok(json!({
        "integrations": whatsapp,
        "whatsappCredentialPublic": whatsapp_public,
        "oauthCallbackUrl": format!("{}/integrations/whatsapp/callback", ctx.api.app_url.trim_end_matches('/')),
        "messengerIntegrations": id_name(&integrations(ctx, MESSENGER_SQL).await?),
        "instagramIntegrations": id_name(&integrations(ctx, INSTAGRAM_SQL).await?),
        "workspaceCreatedAt": iso(created_at),
    }))
}

/// `getConversionEventsData` (super admin page).
async fn conversion_events(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    super_admin(ctx)?;
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let whatsapp_templates = integrations(
        ctx,
        r#"SELECT t."id", t."name", t."language", t."status", t."integrationWhatsappId"
           FROM "WhatsappMessageTemplate" t
           JOIN "IntegrationWhatsapp" i ON i."id" = t."integrationWhatsappId"
           WHERE i."workspaceId" = ?1 AND t."status" = 'APPROVED' ORDER BY t."id""#,
    )
    .await?;
    let messenger_templates = integrations(
        ctx,
        r#"SELECT t."id", t."name", t."language", t."status", t."integrationMessengerId"
           FROM "MessengerMessageTemplate" t
           JOIN "IntegrationMessenger" i ON i."id" = t."integrationMessengerId"
           WHERE i."workspaceId" = ?1 AND t."status" = 'APPROVED' ORDER BY t."id""#,
    )
    .await?;
    // ponytail: the pickers' option lists are capped at 50 rows each, as the
    // original (`RULE_PICKER_OPTIONS_LIMIT`).
    let tags = integrations(
        ctx,
        r#"SELECT "id", "name" FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL
           ORDER BY "createdAt" DESC, "id" DESC LIMIT 50"#,
    )
    .await?;
    let responses: Vec<Value> = tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "id", "keywords", "status", "type" FROM "AutomatedResponse"
               WHERE "workspaceId" = ?1 AND "type" = 'inbound' AND "folderId" IS NULL
               ORDER BY "createdAt" DESC, "id" DESC LIMIT 50"#,
        )?;
        let rows = stmt
            .query_map([ws], |r| {
                let keywords: String = r.get(1)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "keywords": serde_json::from_str::<Value>(&keywords).unwrap_or(json!([])),
                    "status": r.get::<_, i64>(2)? != 0,
                    "type": r.get::<_, String>(3)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await?;
    Ok(json!({
        "whatsappIntegrations": id_name(&integrations(ctx, WHATSAPP_SQL).await?),
        "whatsappTemplates": whatsapp_templates,
        "messengerIntegrations": id_name(&integrations(ctx, MESSENGER_SQL).await?),
        "messengerTemplates": messenger_templates,
        "instagramIntegrations": id_name(&integrations(ctx, INSTAGRAM_SQL).await?),
        "rules": rules::list(db, ws, None).await?,
        "tags": tags,
        "automatedResponses": responses,
    }))
}

/// The `[overview, capiDelivery, timeseries]` of the ads analytics page
/// (super admin). The delivery card is hidden in the original, so its summary
/// is the zero one, as there.
async fn analytics(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    super_admin(ctx).map_err(fail)?;
    let keys = [
        "from",
        "to",
        "tz",
        "channel",
        "adAccountId",
        "integrationWhatsappId",
        "integrationMessengerId",
        "integrationInstagramId",
    ];
    let overview = get(ctx, "ads.getAnalyticsOverview", input, &keys).await?;
    let timeseries = get(ctx, "ads.getAnalyticsTimeseries", input, &keys).await?;
    Ok(json!([
        overview,
        { "sent": 0, "pending": 0, "failed": 0, "skippedNoScope": 0, "skippedRegion": 0 },
        timeseries["data"],
    ]))
}

/// `resolveGoogleAdsDashboardEntry`: super admins see the Google Ads nav
/// entry once a setup exists or any conversion was recorded.
async fn google_entry(ctx: &Ctx<'_>) -> Result<bool, BusinessError> {
    if !ctx.has("superAdmin") {
        return Ok(false);
    }
    let ws = ctx.workspace_id;
    if google_ads_setup::get_setup(&ctx.api.db, ws)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    tx::read(&ctx.api.db, move |conn| {
        Ok(conn.query_row(
            r#"SELECT EXISTS(SELECT 1 FROM "GoogleAdsConversionEvent" WHERE "workspaceId" = ?1)"#,
            [ws],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

/// The Google Ads stats page (super admin): stats, connection, synced
/// actions, workspace birth and Google's own report.
async fn google_stats(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    super_admin(ctx).map_err(fail)?;
    let keys = ["from", "to", "tz", "channel", "conversionActionId"];
    let mut search = strip(input);
    if let Some(action) = search.get("action").cloned() {
        search["conversionActionId"] = action;
    }
    let stats = get(ctx, "googleAds.getStats", &search, &keys).await?;
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let page = async {
        let setup = google_ads_setup::get_setup(db, ws).await?;
        let created_at: i64 = tx::read(db, move |conn| {
            Ok(conn.query_row(
                r#"SELECT "createdAt" FROM "Workspace" WHERE "id" = ?1"#,
                [ws],
                |r| r.get(0),
            )?)
        })
        .await?;
        let report = match (&setup, ctx.api.encryption.as_deref()) {
            (Some(_), Some(encryption)) => {
                let credential =
                    google_ads_setup::credential(db, encryption, &ctx.api.app_url, ws).await?;
                let from = stats["range"]["from"].as_str().unwrap_or_default();
                let to = stats["range"]["to"].as_str().unwrap_or_default();
                Some(
                    google_ads_setup::conversion_report(
                        db,
                        &HttpUpstream,
                        credential.as_ref(),
                        ws,
                        from,
                        to,
                    )
                    .await?,
                )
            }
            (Some(_), None) => Some(json!({ "status": "unavailable" })),
            _ => None,
        };
        let synced: Vec<Value> = setup
            .as_ref()
            .and_then(|s| s.integration.conversion_actions.as_ref())
            .and_then(|actions| actions.0.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|a| json!({ "id": a["id"], "name": a["name"] }))
            .collect();
        Ok::<_, BusinessError>(json!({
            "connected": setup.is_some(),
            "syncedActions": synced,
            "workspaceCreatedAt": iso(created_at),
            "googleReport": report,
        }))
    }
    .await
    .map_err(fail)?;
    let mut out = page;
    out["stats"] = stats;
    Ok(out)
}

/// `listMessagingAdsToolIntegrations`.
async fn tool_integrations(ctx: &Ctx<'_>, channel: &str) -> Result<Value, BusinessError> {
    let (integrations_out, unsupported) = match channel {
        "whatsapp" => {
            let rows = integrations(ctx, WHATSAPP_SQL).await?;
            let list = rows
                .iter()
                .map(|r| {
                    let name = r["name"].as_str().unwrap_or_default();
                    let label = match r["displayPhoneNumber"].as_str() {
                        Some(phone) if !phone.is_empty() => format!("{name} — {phone}"),
                        _ => name.to_owned(),
                    };
                    json!({ "id": r["id"], "name": label })
                })
                .collect();
            (list, false)
        }
        "messenger" => (id_name(&integrations(ctx, MESSENGER_SQL).await?), false),
        "instagram" => {
            let rows = integrations(ctx, INSTAGRAM_SQL).await?;
            let eligible: Vec<Value> = rows
                .iter()
                .filter(|r| r["type"] == "facebook")
                .cloned()
                .collect();
            (id_name(&eligible), eligible.len() < rows.len())
        }
        _ => return Err(BusinessError::not_found("Page not found")),
    };
    Ok(json!({ "integrations": integrations_out, "hasUnsupportedIntegrations": unsupported }))
}

fn integration_column(channel: &str) -> Option<&'static str> {
    match channel {
        "whatsapp" => Some("integrationWhatsappId"),
        "messenger" => Some("integrationMessengerId"),
        "instagram" => Some("integrationInstagramId"),
        _ => None,
    }
}

/// `listActiveMessagingAdsIntegrationIds` (fail-soft: none).
async fn active_integration_ids(ctx: &Ctx<'_>, channel: &str) -> Result<Value, BusinessError> {
    let Some(column) = integration_column(channel) else {
        return Ok(json!([]));
    };
    let ws = ctx.workspace_id;
    let channel = channel.to_owned();
    let ids: Vec<String> = tx::read(&ctx.api.db, move |conn| {
        let mut stmt = conn.prepare(&format!(
            r#"SELECT "{column}" FROM "MessagingAdsConnection"
               WHERE "workspaceId" = ?1 AND "channel" = ?2 AND "status" = 'active' AND "{column}" IS NOT NULL
               ORDER BY "id""#
        ))?;
        let rows = stmt
            .query_map(params![ws, channel], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().map(|id| id.to_string()).collect())
    })
    .await
    .unwrap_or_else(|error| {
        tracing::error!(%error, "Failed to resolve active messaging-ads integration ids; degrading to none");
        Vec::new()
    });
    Ok(json!(ids))
}

/// `checkMessagingAdsConnectionState` (fail-soft: not connected).
async fn connection_state(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let body = get(
        ctx,
        "ads.checkCampaignPrerequisites",
        input,
        &["channel", "integrationId"],
    )
    .await;
    Ok(match body {
        Ok(body) => {
            json!({ "connected": body["connected"], "reconnectNeeded": body["reconnectNeeded"] })
        }
        Err(error) => {
            tracing::error!(message = %error.message, "Failed to resolve messaging-ads connection state; degrading box to not-connected");
            json!({ "connected": false, "reconnectNeeded": false })
        }
    })
}

/// The marketing messages list page: grant state, rows and per-row counts.
async fn marketing_messages_page(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let state = mm::grant_state(db, ws).await?;
    let rows = campaigns::list(db, ws).await?;
    let mut counts = Map::new();
    for row in &rows {
        // A row whose stats cannot be read renders without counts.
        if let Ok(stats) = campaigns::stats(db, row.id).await {
            let c = &stats["counts"];
            counts.insert(
                row.id.to_string(),
                json!({ "sent": c["sent"], "delivered": c["delivered"], "failed": c["failed"] }),
            );
        }
    }
    Ok(json!({
        "grantState": state,
        "rows": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
        "counts": counts,
    }))
}

/// The marketing message edit page: grant state, row and stats (404 when
/// missing).
async fn marketing_message_page(ctx: &Ctx<'_>, id: &str) -> Result<Value, BusinessError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let row = campaigns::find(db, ws, id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Page not found"))?;
    let state = mm::grant_state(db, ws).await?;
    let stats = campaigns::stats(db, row.id).await?;
    Ok(json!({ "grantState": state, "row": row.to_json(), "stats": stats }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_drops_workspace_and_empty_values() {
        let out = strip(&json!({ "workspaceId": "1", "a": "", "b": null, "c": "x", "d": false }));
        assert_eq!(out, json!({ "c": "x", "d": false }));
    }
}
