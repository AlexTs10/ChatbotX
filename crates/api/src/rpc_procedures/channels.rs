//! The channel settings screens of the builder (Q.D12 / Q.I1b final round):
//! `app/space/[workspaceId]/(integrations)/whatsapps/[id]/*`,
//! `messengers/[id]/*`, `instagrams/[id]/*`, `webchats/[id]/edit`.
//!
//! Page queries (`serverQuery`, P4 spec §5.1; `args: [workspaceId, input]`):
//! `whatsappChannelPage` (`{ id, tab }`), `messengerChannelPage`,
//! `messengerTemplatesPage`, `instagramChannelPage`, `webchatChannelPage`.
//! Each layout is `resolveGuardedWorkspaceId(params, "superAdmin")`: a member
//! without `superAdmin` gets 404, as `notFound()`.
//!
//! Actions: the builder `"use server"` exports of those screens. Where a
//! ported public `/v1` route has the same effect, the action runs it
//! ([`public`]).
//!
//! Procedures: `integrationWhatsappAPIs.{get,sync}WhatsappRoutingRolesAPI`,
//! `getWhatsappRoutingRolesByInboxAPI`.

use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::{IntegrationMessenger, IntegrationWhatsapp};
use chatbotx_db::repo::{find_many, integration_whatsapp, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, schema_errors};
use crate::handlers::channel_integrations::{
    find_instagram, find_messenger, instagram_settings, messenger_settings,
};
use crate::handlers::messenger_templates::{clone_template, find_template, template_json};
use crate::handlers::whatsapp_channels::{self as wa, WhatsappAuth};
use crate::handlers::{internal_error, whatsapp_flows, whatsapp_templates};
use crate::orpc::OrpcError;

/// The workspace actions of this module.
pub(super) const ACTIONS: &[&str] = &[
    "whatsappChannelPage",
    "messengerChannelPage",
    "messengerTemplatesPage",
    "instagramChannelPage",
    "webchatChannelPage",
    "updateWebchatAction",
    "updateMessengerAction",
    "updateInstagramAction",
    "toggleMessengerTagSyncAction",
    "integration-messenger.updateHandoverResumeFlowAction",
    "integration-whatsapp.updateHandoverResumeFlowAction",
    "createMessengerMessageTemplateAction",
    "deleteMessengerMessageTemplateAction",
    "syncMessengerMessageTemplateAction",
    "cloneMessengerMessageTemplateAction",
    "updatePixRedirectAction",
    "updateWhatsappProfileAction",
    "requestWhatsappVerificationCodeAction",
    "verifyWhatsappPhoneCodeAction",
];

/// `WHATSAPP_VERIFICATION_COOLDOWN_SECONDS`.
const VERIFICATION_COOLDOWN_MS: i64 = 60_000;

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let inp = In(input);
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(match path {
        "integrationWhatsappAPIs.getWhatsappRoutingRolesAPI" => {
            match inp.id("integrationWhatsappId") {
                Ok(id) => wa::find_routing_roles(db, ws, id).await.map_err(fail),
                Err(error) => Err(error),
            }
        }
        "integrationWhatsappAPIs.syncWhatsappRoutingRolesAPI" => {
            match inp.id("integrationWhatsappId") {
                Ok(id) => wa::sync_routing_roles(ctx.api, ws, id).await.map_err(fail),
                Err(error) => Err(error),
            }
        }
        "integrationMessengerAPIs.setCoexistMessengerAPI"
        | "integrationInstagramAPIs.setCoexistInstagramAPI" => {
            let channel = if path.starts_with("integrationMessenger") {
                "messenger"
            } else {
                "instagram"
            };
            let run = async {
                let integration_id = inp.id("integrationId")?;
                let enabled = inp.opt_bool("enabled")?.ok_or_else(invalid)?;
                let ai_reads = inp.opt_bool("aiReadsSyncedHistory")?.unwrap_or(false);
                let result = chatbotx_business::coexist::set_pull_coexist(
                    db,
                    channel,
                    ws,
                    integration_id,
                    enabled,
                    ai_reads,
                )
                .await
                .map_err(fail)?;
                // `{ success: false, reason: "not_found" }`.
                Ok(result.map_or_else(
                    || json!({ "success": false, "reason": "not_found" }),
                    crate::handlers::whatsapp_coexist::pull_output,
                ))
            };
            run.await
        }
        "integrationWhatsappAPIs.getWhatsappRoutingRolesByInboxAPI" => match inp.id("inboxId") {
            Ok(inbox) => wa::find_routing_roles_by_inbox(db, ws, inbox)
                .await
                .map_err(fail),
            Err(error) => Err(error),
        },
        _ => return None,
    })
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

fn page_not_found() -> OrpcError {
    fail(BusinessError::not_found("Page not found"))
}

/// The settings layouts: `resolveGuardedWorkspaceId(params, "superAdmin")`.
fn require_page(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.has("superAdmin") {
        Ok(())
    } else {
        Err(page_not_found())
    }
}

/// `assertWorkspaceSuperAdmin`.
fn require_super_admin(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    if ctx.permissions.get("superAdmin") == Some(&Value::Bool(true)) {
        Ok(())
    } else {
        Err(action_fail(BusinessError::exception(
            "You need to be a workspace super admin to do this.",
            "systemError",
            400,
        )))
    }
}

/// The bound id at `index` (`zodBigintAsString`).
fn bound(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

fn input_of(args: &[Value], index: usize) -> Value {
    match arg(args, index) {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Map::new()),
    }
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id;
    let db = &ctx.api.db;
    match name {
        "whatsappChannelPage" => whatsapp_page(ctx, arg(args, 1)).await,
        "messengerChannelPage" => messenger_page(ctx, arg(args, 1)).await,
        "messengerTemplatesPage" => messenger_templates_page(ctx, arg(args, 1)).await,
        "instagramChannelPage" => instagram_page(ctx, arg(args, 1)).await,
        "webchatChannelPage" => webchat_page(ctx, arg(args, 1)).await,
        "updateWebchatAction" => update_webchat(ctx, bound(args, 1)?, input_of(args, 2)).await,
        "updateMessengerAction" | "updateInstagramAction" => {
            let id = bound(args, 1)?;
            let route = if name == "updateMessengerAction" {
                "messengerChannels.updateSettings"
            } else {
                "instagramChannels.updateSettings"
            };
            let body = input_of(args, 2);
            public(ctx, route, &[("id", id.to_string())], None, Some(&body))
                .await
                .map_err(schema_errors)?;
            Ok(Value::Null)
        }
        "toggleMessengerTagSyncAction" => {
            let id = bound(args, 1)?;
            let enabled = In(arg(args, 2)).opt_bool("enabled")?.ok_or_else(invalid)?;
            public(
                ctx,
                "messengerChannels.updateTagSync",
                &[("id", id.to_string())],
                None,
                Some(&json!({ "enabled": enabled })),
            )
            .await
        }
        "integration-messenger.updateHandoverResumeFlowAction"
        | "integration-whatsapp.updateHandoverResumeFlowAction" => {
            let id = bound(args, 1)?;
            let flow = match arg(args, 2).get("handoverResumeFlowId") {
                Some(Value::Null) => None,
                Some(_) => Some(In(arg(args, 2)).id("handoverResumeFlowId")?),
                None => return Err(invalid()),
            };
            require_super_admin(ctx)?;
            let messenger = name.starts_with("integration-messenger");
            let done = if messenger {
                messenger_handover_flow(ctx, id, flow).await
            } else {
                wa::update_handover_resume_flow(db, ws, id, flow).await
            };
            // The action's own copy (`conversationRouting.settings.*`, en).
            done.map_err(|error| match error {
                BusinessError::Exception { code, .. } if code == "notFound" => {
                    action_fail(BusinessError::exception(
                        match (flow, messenger) {
                            (Some(_), _) => "The selected flow was not found or is not active",
                            (None, true) => "Messenger page not found",
                            (None, false) => "WhatsApp number not found",
                        },
                        "notFound",
                        404,
                    ))
                }
                other => action_fail(other),
            })?;
            Ok(json!({ "handoverResumeFlowId": flow.map(|id| id.to_string()) }))
        }
        "createMessengerMessageTemplateAction" => {
            let id = bound(args, 1)?;
            messenger_page_or_error(ctx, id).await?;
            let body = input_of(args, 2);
            let created = public(
                ctx,
                "messengerTemplates.create",
                &[("id", id.to_string())],
                None,
                Some(&body),
            )
            .await
            .map_err(schema_errors)?;
            if created["status"] == "REJECTED" {
                let reasons: Vec<&str> = ["rejectionReason", "specificRejectionReason"]
                    .iter()
                    .filter_map(|k| created[*k].as_str().filter(|s| !s.is_empty()))
                    .collect();
                let message = if reasons.is_empty() {
                    "Meta rejected this template".to_owned()
                } else {
                    format!("Meta rejected this template: {}", reasons.join(" / "))
                };
                return Err(OrpcError::new("BAD_REQUEST", Some(400), Some(&message)));
            }
            Ok(json!({ "status": created["status"] }))
        }
        "deleteMessengerMessageTemplateAction" => {
            let (id, template) = (bound(args, 1)?, bound(args, 2)?);
            messenger_page_or_error(ctx, id).await?;
            tx::write(db, move |conn| {
                conn.execute(
                    r#"DELETE FROM "MessengerMessageTemplate" WHERE "id" = ?1 AND "integrationMessengerId" = ?2"#,
                    params![template, id],
                )?;
                Ok(())
            })
            .await
            .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "syncMessengerMessageTemplateAction" => {
            let id = bound(args, 1)?;
            messenger_page_or_error(ctx, id).await?;
            public(
                ctx,
                "messengerTemplates.sync",
                &[("id", id.to_string())],
                None,
                Some(&json!({})),
            )
            .await?;
            Ok(Value::Null)
        }
        "cloneMessengerMessageTemplateAction" => {
            clone_messenger_template(ctx, bound(args, 1)?, bound(args, 2)?, arg(args, 3)).await
        }
        "updatePixRedirectAction" => {
            let id = bound(args, 1)?;
            let enabled = In(arg(args, 2))
                .opt_bool("pixRedirectEnabled")?
                .ok_or_else(invalid)?;
            require_super_admin(ctx)?;
            wa::update_pix_redirect(db, ws, id, enabled)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "pixRedirectEnabled": enabled }))
        }
        // The original saves nothing (the profile form is a placeholder).
        "updateWhatsappProfileAction" => {
            check_profile_request(arg(args, 1))?;
            Ok(json!({ "success": true }))
        }
        "requestWhatsappVerificationCodeAction" => {
            request_verification_code(ctx, arg(args, 1)).await
        }
        "verifyWhatsappPhoneCodeAction" => verify_phone_code(ctx, arg(args, 1)).await,
        _ => Err(invalid()),
    }
}

// ---- pages ------------------------------------------------------------------

async fn whatsapp_row(ctx: &Ctx<'_>, id: i64) -> Result<IntegrationWhatsapp, OrpcError> {
    wa::find(&ctx.api.db, ctx.workspace_id, id)
        .await
        .map_err(fail)?
        .ok_or_else(|| fail(BusinessError::not_found("Whatsapp integration not found")))
}

/// `toIntegrationWhatsappLinkable`: never the `auth` / `capiAccessToken`.
fn linkable(row: &IntegrationWhatsapp) -> Value {
    json!({
        "id": row.id.to_string(),
        "workspaceId": row.workspace_id.to_string(),
        "businessId": row.business_id,
        "wabaId": row.waba_id,
    })
}

/// The `whatsapps/[id]/*` pages; `tab` = `settings`, `accountHealths`,
/// `flows`, `messageTemplates` or `links` (useful links + e-commerce).
async fn whatsapp_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_page(ctx)?;
    let inp = In(input);
    let id = inp.id("id").map_err(|_| page_not_found())?;
    let tab = inp.opt_str("tab")?.unwrap_or_default();
    let row = whatsapp_row(ctx, id).await?;
    let auth = WhatsappAuth::of(&row);
    let graph = ctx.api.graph_url.as_str();
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let mut out = json!({ "integrationWhatsapp": linkable(&row) });
    match tab.as_str() {
        "settings" => {
            out["automation"] = match wa::conversational_automation(graph, &auth).await {
                Ok(automation) => automation,
                Err(error) => json!({
                    "enable_welcome_message": false,
                    "prompts": [],
                    "commands": [],
                    "error": error.message,
                }),
            };
            out["isSuperAdmin"] = json!(true);
            out["pixRedirectEnabled"] = json!(row.pix_redirect_enabled);
            out["handoverResumeFlowId"] = json!(row.handover_resume_flow_id.map(|i| i.to_string()));
        }
        "accountHealths" => {
            let (phone, waba) = tokio::join!(
                wa::phone_number_detail(graph, &auth),
                wa::waba(graph, &auth, "marketing_messages_onboarding_status"),
            );
            if let Err(error) = &phone {
                tracing::warn!(integration = row.id, error = %error.message, "Unable to load WhatsApp phone number health");
            }
            if let Err(error) = &waba {
                tracing::warn!(integration = row.id, error = %error.message, "Unable to load WhatsApp WABA health");
            }
            let phone = phone.ok();
            let text = |key: &str| {
                phone
                    .as_ref()
                    .and_then(|p| p.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            out["displayPhoneNumber"] = json!(
                text("display_phone_number").unwrap_or_else(|| row.display_phone_number.clone())
            );
            out["verifiedName"] = json!(text("verified_name").unwrap_or_else(|| row.name.clone()));
            out["phoneNumber"] = phone.clone().unwrap_or(Value::Null);
            out["waba"] = waba.unwrap_or_else(|_| json!({}));
            out["businessManagerUrl"] = json!(format!(
                "{}/wa/manage/home/?business_id={}&waba_id={}",
                wa::BUSINESS_URL,
                auth.business_id,
                auth.waba_id
            ));
            out["webhookUrl"] = json!(auth.webhook_url);
            out["registrationStatus"] = json!(row.registration_status);
            out["registrationError"] = row
                .registration_error
                .as_ref()
                .map_or(Value::Null, |j| j.0.clone());
            out["verificationCodeRequestedAt"] = json!(
                row.verification_code_requested_at
                    .map(chatbotx_db::repo::iso)
            );
        }
        "flows" => {
            out["flows"] = Value::Array(
                whatsapp_flows::list(db, ws, None, Some(id), false)
                    .await
                    .map_err(fail)?,
            );
        }
        "messageTemplates" => {
            out["messageTemplates"] = Value::Array(
                whatsapp_templates::list(db, ws, None, Some(id), None)
                    .await
                    .map_err(fail)?,
            );
        }
        _ => out["urls"] = auth.urls(),
    }
    Ok(out)
}

async fn mark_read_on_outbound(ctx: &Ctx<'_>, inbox: i64) -> Result<bool, OrpcError> {
    let ws = ctx.workspace_id;
    tx::read(&ctx.api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "markReadOnOutbound" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![inbox, ws],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?
            .unwrap_or(false))
    })
    .await
    .map_err(fail)
}

/// A model row as the client props: never the tokens.
fn client_row(mut row: Value) -> Value {
    if let Value::Object(map) = &mut row {
        map.remove("auth");
        map.remove("capiAccessToken");
    }
    row
}

/// `messengers/[id]/edit`: the form, the routing card and the AI hand-over
/// card. No support sessions in Rust, so `canEditAiHandover` = super admin.
async fn messenger_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_page(ctx)?;
    let id = In(input).id("id").map_err(|_| page_not_found())?;
    let ws = ctx.workspace_id;
    let row = find_messenger(&ctx.api.db, ws, id).await.map_err(fail)?;
    let mut integration = client_row(row.to_json());
    if let (Value::Object(target), Value::Object(settings)) =
        (&mut integration, messenger_settings(&row))
    {
        target.extend(settings);
    }
    let inbox = row.inbox_id;
    let settings = tx::read(&ctx.api.db, move |conn| {
        chatbotx_business::ai_handover::find_in(conn, ws, inbox)
    })
    .await
    .map_err(fail)?;
    let initial = json!({
        "enabled": settings.as_ref().is_some_and(|s| s.enabled),
        "scheduleEnabled": settings.as_ref().is_some_and(|s| s.schedule_enabled),
        "timeRanges": settings.as_ref().map_or(json!([]), |s| s.time_ranges.0.clone()),
        "gotoFlowId": settings.as_ref().and_then(|s| s.goto_flow_id).map(|i| i.to_string()),
        "returnMessage": settings.as_ref().and_then(|s| s.return_message.clone()).unwrap_or_default(),
        "pauseBotWaitingForStaff": settings.as_ref().is_some_and(|s| s.pause_bot_waiting_for_staff),
    });
    Ok(json!({
        "integrationMessenger": integration,
        "markReadOnOutbound": mark_read_on_outbound(ctx, inbox).await?,
        "isSuperAdmin": true,
        "canEditAiHandover": true,
        "aiHandover": initial,
    }))
}

/// `listCloneTargetsForUser`: every Messenger page of a workspace where the
/// user is the owner or a `superAdmin`, without `exclude_page_id`, by name.
async fn clone_targets(
    ctx: &Ctx<'_>,
    exclude_page_id: Option<String>,
) -> Result<Vec<IntegrationMessenger>, OrpcError> {
    let user = ctx.user_id;
    tx::read(&ctx.api.db, move |conn| {
        Ok(find_many::<IntegrationMessenger>(
            conn,
            r#"WHERE "workspaceId" IN (
                   SELECT "workspaceId" FROM "WorkspaceMember" WHERE "userId" = ?1
                   AND ("role" = 'owner' OR json_extract("permissions", '$.superAdmin') = 1))
               AND (?2 IS NULL OR "pageId" <> ?2)
               ORDER BY "name""#,
            &[&user, &exclude_page_id],
        )?)
    })
    .await
    .map_err(fail)
}

/// `messengers/[id]/message-templates` (`{ id, name, page, perPage }`).
async fn messenger_templates_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_page(ctx)?;
    let inp = In(input);
    let id = inp.id("id").map_err(|_| page_not_found())?;
    let ws = ctx.workspace_id;
    let row = find_messenger(&ctx.api.db, ws, id)
        .await
        .map_err(|_| page_not_found())?;
    let channels: Vec<Value> = clone_targets(ctx, Some(row.page_id.clone()))
        .await?
        .iter()
        .map(|c| json!({ "id": c.id.to_string(), "name": c.name }))
        .collect();
    let name = inp.opt_str("name")?.filter(|n| !n.is_empty());
    let per_page = inp.opt_positive("perPage")?.unwrap_or(20).min(100);
    let page = inp.opt_positive("page")?.unwrap_or(1);
    let pattern = name.map(|n| {
        format!(
            "%{}%",
            n.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let (rows, total) = tx::read(&ctx.api.db, move |conn| {
        let filter = r#"WHERE "integrationMessengerId" = ?1
            AND "integrationMessengerId" IN (SELECT "id" FROM "IntegrationMessenger" WHERE "workspaceId" = ?2)
            AND (?3 IS NULL OR "name" LIKE ?3 ESCAPE '\')"#;
        let rows: Vec<chatbotx_db::repo::models::MessengerMessageTemplate> = find_many(
            conn,
            &format!(r#"{filter} ORDER BY "id" DESC LIMIT ?4 OFFSET ?5"#),
            &[&id, &ws, &pattern, &per_page, &((page - 1) * per_page)],
        )?;
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "MessengerMessageTemplate" {filter}"#),
                params![id, ws, pattern],
                |r| r.get(0),
            )
            .map_err(chatbotx_db::repo::Error::from)?;
        Ok((rows, total))
    })
    .await
    .map_err(fail)?;
    let integration = client_row(row.to_json());
    let data: Vec<Value> = rows
        .iter()
        .map(|t| {
            let mut out = template_json(t);
            out["integrationMessenger"] = integration.clone();
            out
        })
        .collect();
    Ok(json!({
        "integrationMessenger": integration,
        "channels": channels,
        "templates": { "data": data, "pageCount": ((total + per_page - 1) / per_page).max(1) },
    }))
}

/// `instagrams/[id]/edit`.
async fn instagram_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_page(ctx)?;
    let id = In(input).id("id").map_err(|_| page_not_found())?;
    let row = find_instagram(&ctx.api.db, ctx.workspace_id, id)
        .await
        .map_err(fail)?;
    let mut integration = client_row(row.to_json());
    if let (Value::Object(target), Value::Object(settings)) =
        (&mut integration, instagram_settings(&row))
    {
        target.extend(settings);
    }
    Ok(json!({
        "integrationInstagram": integration,
        "markReadOnOutbound": mark_read_on_outbound(ctx, row.inbox_id).await?,
    }))
}

/// `webchats/[id]/edit` (`requireWorkspacePermission(superAdmin)`).
async fn webchat_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    require_page(ctx)?;
    let id = In(input).id("id").map_err(|_| page_not_found())?;
    let row = chatbotx_business::integration_webchat::find_by_id_for_workspace(
        &ctx.api.db,
        id,
        ctx.workspace_id,
    )
    .await
    .map_err(fail)?;
    let mut webchat = crate::handlers::webchats::webchat_json(&row);
    webchat["workspaceId"] = json!(ctx.workspace_id.to_string());
    Ok(json!({
        "integrationWebchat": webchat,
        "markReadOnOutbound": mark_read_on_outbound(ctx, row.inbox_id).await?,
    }))
}

// ---- actions ----------------------------------------------------------------

/// `updateWebchatAction`: super admin only (a plain `Error` = the default
/// server error), then `integrationWebchatService.update` (the public route)
/// and the inbox's `markReadOnOutbound`.
async fn update_webchat(ctx: &Ctx<'_>, id: i64, mut body: Value) -> Result<Value, OrpcError> {
    if !ctx.has("superAdmin") {
        return Err(internal_error(
            "You need to be a super admin to update this webchat",
        ));
    }
    let ws = ctx.workspace_id;
    let row = chatbotx_business::integration_webchat::find_by_id_for_workspace(&ctx.api.db, id, ws)
        .await
        .map_err(action_fail)?;
    let mark_read = body
        .as_object_mut()
        .and_then(|map| map.remove("markReadOnOutbound"));
    // `authorizedDomains: [{ value }]` → the hostnames.
    if let Some(domains) = body.get_mut("authorizedDomains")
        && let Value::Array(items) = domains
    {
        for item in items.iter_mut() {
            if let Some(value) = item.get("value").cloned() {
                *item = value;
            }
        }
    }
    public(
        ctx,
        "webchats.update",
        &[("id", id.to_string())],
        None,
        Some(&body),
    )
    .await
    .map_err(schema_errors)?;
    match mark_read {
        Some(Value::Bool(enabled)) => {
            chatbotx_business::inbox::service::update_mark_read_on_outbound(
                &ctx.api.db,
                ws,
                row.inbox_id,
                enabled,
            )
            .await
            .map_err(action_fail)?;
        }
        None | Some(Value::Null) => {}
        Some(_) => return Err(invalid()),
    }
    Ok(Value::Null)
}

/// `messengerIntegrationService.updateHandoverResumeFlow`.
async fn messenger_handover_flow(
    ctx: &Ctx<'_>,
    id: i64,
    flow: Option<i64>,
) -> Result<(), BusinessError> {
    let ws = ctx.workspace_id;
    tx::write(&ctx.api.db, move |conn| {
        if let Some(flow) = flow {
            let active: Option<i64> = conn
                .query_row(
                    r#"SELECT "id" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                    params![flow, ws],
                    |r| r.get(0),
                )
                .ok();
            if active.is_none() {
                return Err(BusinessError::not_found("Handover flow not found"));
            }
        }
        chatbotx_db::repo::integration_messenger::update_handover_resume_flow(conn, id, ws, flow)?
            .map(|_| ())
            .ok_or_else(|| BusinessError::not_found("Messenger integration not found"))
    })
    .await
}

/// `messengerIntegrationService.findByIdForWorkspace` of an action; a page
/// of another workspace is the original's plain `Error`.
async fn messenger_page_or_error(
    ctx: &Ctx<'_>,
    id: i64,
) -> Result<IntegrationMessenger, OrpcError> {
    find_messenger(&ctx.api.db, ctx.workspace_id, id)
        .await
        .map_err(|_| internal_error("Messenger integration not found"))
}

/// `cloneMessengerMessageTemplateAction`: targets are the user's admin
/// workspaces' pages (read now), never the source's own Facebook page.
async fn clone_messenger_template(
    ctx: &Ctx<'_>,
    source_id: i64,
    template_id: i64,
    input: &Value,
) -> Result<Value, OrpcError> {
    let requested = match input.get("targetIntegrationMessengerIds") {
        Some(Value::Array(items)) if !items.is_empty() => {
            In(input).ids("targetIntegrationMessengerIds")?
        }
        _ => return Err(invalid()),
    };
    let ws = ctx.workspace_id;
    let (template, page) = find_template(&ctx.api.db, ws, template_id)
        .await
        .map_err(action_fail)?
        .filter(|(t, _)| t.integration_messenger_id == source_id)
        .ok_or_else(|| internal_error("Source template not found"))?;
    let targets: Vec<IntegrationMessenger> = clone_targets(ctx, Some(page.page_id.clone()))
        .await?
        .into_iter()
        .filter(|t| requested.contains(&t.id))
        .collect();
    if targets.is_empty() {
        return Err(internal_error("No authorized target channels found"));
    }
    Ok(clone_template(&ctx.api.db, &template, targets).await)
}

/// `updateWhatsappProfileRequest`.
fn check_profile_request(input: &Value) -> Result<(), OrpcError> {
    let inp = In(input);
    for key in ["about", "description", "address"] {
        inp.str(key, true, 0, 255)?;
    }
    let email = inp.str("email", false, 1, usize::MAX)?;
    let website = inp.str("websiteUrl", false, 1, usize::MAX)?;
    let at = email.find('@').unwrap_or(0);
    if at == 0 || !email[at + 1..].contains('.') || url::Url::parse(&website).is_err() {
        return Err(invalid());
    }
    Ok(())
}

/// `whatsapp.phoneVerification.errors.*` (en).
const INTEGRATION_NOT_FOUND: &str = "WhatsApp integration not found.";

fn exception(message: &str) -> OrpcError {
    action_fail(BusinessError::exception(message, "systemError", 400))
}

/// `requestWhatsappVerificationCodeAction`: one code per 60 s per number
/// (`claimVerificationCodeSlot`); a Meta refusal hands the slot back.
async fn request_verification_code(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let inp = In(input);
    let id = inp.id("integrationId")?;
    let method = inp.opt_str("codeMethod")?.ok_or_else(invalid)?;
    if !matches!(method.as_str(), "SMS" | "VOICE") {
        return Err(invalid());
    }
    let ws = ctx.workspace_id;
    let db = &ctx.api.db;
    let row = wa::find(db, ws, id)
        .await
        .map_err(action_fail)?
        .ok_or_else(|| exception(INTEGRATION_NOT_FOUND))?;
    let now = now_ms();
    let claimed = tx::write(db, move |conn| {
        let claimed = integration_whatsapp::claim_verification_code_slot(
            conn,
            id,
            ws,
            now,
            now - VERIFICATION_COOLDOWN_MS,
        )?;
        let existing = match claimed {
            Some(_) => None,
            None => integration_whatsapp::find_verification_code_requested_at(conn, id, ws)?,
        };
        Ok((claimed, existing))
    })
    .await
    .map_err(action_fail)?;
    let requested_at = match claimed {
        (Some(at), _) => at,
        (None, None) => return Err(exception(INTEGRATION_NOT_FOUND)),
        (None, Some(previous)) => {
            let next = previous.map_or(now, |at| at + VERIFICATION_COOLDOWN_MS);
            return Ok(json!({
                "status": "cooldown",
                "requestedAt": previous.map(chatbotx_db::repo::iso),
                "remainingSeconds": ((next - now).max(0) + 999) / 1000,
            }));
        }
    };
    let auth = WhatsappAuth::of(&row);
    if let Err(error) =
        wa::request_verification_code(&ctx.api.graph_url, &auth, &row.phone_number_id, &method)
            .await
    {
        let release = tx::write(db, move |conn| {
            Ok(integration_whatsapp::release_verification_code_slot(
                conn,
                id,
                ws,
                requested_at,
            )?)
        })
        .await;
        if let Err(release) = release {
            tracing::error!(integration = id, error = %release, "Failed to release WhatsApp verification code slot");
        }
        return Err(exception(
            &error.action_message("WhatsApp verification code could not be sent."),
        ));
    }
    Ok(json!({ "status": "sent", "requestedAt": chatbotx_db::repo::iso(requested_at) }))
}

/// `verifyWhatsappPhoneCodeAction`: Meta's `verify_code`, then the
/// registration retry (a coexist number is registered as is).
async fn verify_phone_code(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let inp = In(input);
    let id = inp.id("integrationId")?;
    let code = inp.str("code", true, 6, 6)?;
    if !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let ws = ctx.workspace_id;
    let db = &ctx.api.db;
    let graph = ctx.api.graph_url.as_str();
    let row = wa::find(db, ws, id)
        .await
        .map_err(action_fail)?
        .ok_or_else(|| exception(INTEGRATION_NOT_FOUND))?;
    let auth = WhatsappAuth::of(&row);
    wa::verify_code(graph, &auth, &row.phone_number_id, &code)
        .await
        .map_err(|e| {
            exception(&e.action_message("WhatsApp verification code could not be verified."))
        })?;
    let outcome = if row.is_coexist {
        ("registered", None)
    } else {
        wa::register_phone_number(graph, &auth, &row.phone_number_id)
            .await
            .map_err(|e| {
                exception(&e.action_message("WhatsApp phone number verification failed."))
            })?
    };
    let status = outcome.0;
    let stored = wa::record_registration(db, ws, id, outcome)
        .await
        .map_err(action_fail)?;
    if status != "registered" {
        let text = |key: &str| {
            stored
                .as_ref()
                .and_then(|e| e.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let message = text("userMessage")
            .or_else(|| text("userTitle"))
            .or_else(|| text("message"))
            .unwrap_or_else(|| "WhatsApp phone number verification failed.".to_owned());
        return Err(exception(&message));
    }
    Ok(json!({ "status": "registered" }))
}
