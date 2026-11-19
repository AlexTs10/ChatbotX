//! The settings integration pages of the SPA that had no Rust handler yet
//! (Q.F2-settings):
//!
//! - the email-marketing connect / disconnect actions
//!   (`features/integration-<name>/actions`, `createCredentialConnectAction` /
//!   `createDisconnectAction` of `lib/integration-actions.ts`) and their page
//!   query `emailIntegrationPage` (`integration<Name>Service.findByWorkspaceId`),
//! - `makeIntegrationPage` (the `make` platform credential's invite URL),
//! - `chatbotxMcpPage` (`mcpOAuthService.listGrantsForUser` +
//!   `toMcpOAuthGrantDto`, the MCP and builder URLs),
//! - `connectPaypalAction`, `listIntegrationSmtps`,
//! - the TikTok and Zalo channel tables (`listIntegrationTiktoks`,
//!   `listIntegrationZalo`) and their actions (disconnect, token refresh,
//!   Comment-to-Message, Zalo reconnect), the Threads table
//!   (`listIntegrationThreads`, disconnect, reconnect),
//!   `updateMarkReadOnOutboundAction`.
//!
//! The `/rpc` membership check ran; the settings layout is super admin only
//! (the SPA guard, as the builder's layout).

use chatbotx_business::BusinessError;
use chatbotx_business::audit::AuditRecordInput;
use chatbotx_business::channel_integration::{self, TIKTOK_BUSINESS_API_URL};
use chatbotx_business::connection::api_key::{self, ApiKeyProvider};
use chatbotx_business::connection::credentials::{self, HttpUpstream, SafeUpstream};
use chatbotx_business::inbox::service as inbox_service;
use chatbotx_business::{mcp_oauth, payment_provider, platform_credential, tenant, tx};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::handlers::date;
use crate::orpc::OrpcError;

/// The email-marketing provider keys and their export stems.
const EMAIL_PROVIDERS: &[(&str, &str)] = &[
    ("ActiveCampaign", "activeCampaign"),
    ("Drip", "drip"),
    ("GetResponse", "getResponse"),
    ("Klaviyo", "klaviyo"),
    ("Mailchimp", "mailchimp"),
    ("MailerLite", "mailerLite"),
    ("Moosend", "moosend"),
    ("SendGrid", "sendGrid"),
];

pub(super) const ACTIONS: &[&str] = &[
    "connectActiveCampaignAction",
    "connectDripAction",
    "connectGetResponseAction",
    "connectKlaviyoAction",
    "connectMailchimpAction",
    "connectMailerLiteAction",
    "connectMoosendAction",
    "connectSendGridAction",
    "disconnectActiveCampaignAction",
    "disconnectDripAction",
    "disconnectGetResponseAction",
    "disconnectKlaviyoAction",
    "disconnectMailchimpAction",
    "disconnectMailerLiteAction",
    "disconnectMoosendAction",
    "disconnectSendGridAction",
    "emailIntegrationPage",
    "makeIntegrationPage",
    "chatbotxMcpPage",
    "connectPaypalAction",
    "listIntegrationSmtps",
    "listIntegrationTiktoks",
    "listIntegrationZalo",
    "disconnectTiktokAction",
    "disconnectZaloAction",
    "refreshTiktokTokenAction",
    "toggleTiktokCommentToMessageAction",
    "refreshTiktokCommentToMessageAction",
    "reconnectZaloAction",
    "updateMarkReadOnOutboundAction",
    "channelCredentialPage",
    "toggleZaloTagSyncAction",
    "listIntegrationThreads",
    "disconnectThreadsAction",
    "reconnectThreadsAction",
];

/// The provider of a `connect<Name>Action` / `disconnect<Name>Action`.
fn email_provider(name: &str) -> Option<ApiKeyProvider> {
    let stem = name
        .strip_prefix("connect")
        .or_else(|| name.strip_prefix("disconnect"))?
        .strip_suffix("Action")?;
    let key = EMAIL_PROVIDERS.iter().find(|(s, _)| *s == stem)?.1;
    api_key::api_key_provider(key)
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    match name {
        "emailIntegrationPage" => {
            let key = arg(args, 0).as_str().ok_or_else(invalid)?;
            let provider = api_key::api_key_provider(key).ok_or_else(invalid)?;
            let table = provider.table;
            let connected = tx::read(db, move |conn| {
                Ok(conn.query_row(
                    &format!(r#"SELECT EXISTS (SELECT 1 FROM "{table}" WHERE "workspaceId" = ?1)"#),
                    params![ws],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .await
            .map_err(fail)?;
            Ok(json!({ "connected": connected }))
        }
        "makeIntegrationPage" => {
            let owner = tenant::resolve_credential_owner_id_for_workspace(db, ws, None)
                .await
                .map_err(fail)?;
            let invite_url = match ctx.api.encryption.clone() {
                Some(encryption) => tx::read(db, move |conn| {
                    platform_credential::resolve_for_owner(conn, &encryption, owner, "make", false)
                })
                .await
                .map_err(fail)?
                .and_then(|c| c.text("inviteUrl").map(str::to_string)),
                None => None,
            };
            Ok(json!({ "inviteUrl": invite_url }))
        }
        "chatbotxMcpPage" => {
            let grants = mcp_oauth::list_grants_for_user(db, ctx.user_id, ctx.now_ms)
                .await
                .map_err(fail)?;
            let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
            let mcp_url = env("MCP_PUBLIC_URL")
                .unwrap_or_else(|| format!("{}/mcp", ctx.api.app_url.trim_end_matches('/')));
            Ok(json!({
                "grants": grants.iter().map(|g| json!({
                    "activeUntil": g.active_until.map(date),
                    "clientIcon": https_icon(g.client_icon.as_deref()),
                    "clientId": g.client_id,
                    "clientName": g.client_name,
                    "firstAuthorizedAt": date(g.first_authorized_at),
                    "lastRefreshedAt": g.last_refreshed_at.map(date),
                    "scopes": g.scopes,
                })).collect::<Vec<_>>(),
                "mcpUrl": mcp_url.trim_end_matches('/'),
                "builderUrl": env("BETTER_AUTH_URL").unwrap_or_else(|| ctx.api.app_url.clone()),
            }))
        }
        "listIntegrationSmtps" => {
            let rows = chatbotx_business::email_marketing::smtp_service::list_by_workspace(db, ws)
                .await
                .map_err(fail)?;
            Ok(json!({ "data": rows }))
        }
        "connectPaypalAction" => connect_paypal(ctx, arg(args, 1)).await,
        // The channel pages' `platformCredentialService.resolveForOwner`:
        // only the public app id reaches the page (`publicConfig.clientId`).
        "channelCredentialPage" => {
            let kind = arg(args, 0).as_str().ok_or_else(invalid)?;
            if !matches!(kind, "tiktok" | "zalo" | "threads") {
                return Err(invalid());
            }
            let kind = kind.to_string();
            let owner = tenant::resolve_credential_owner_id_for_workspace(db, ws, None)
                .await
                .map_err(fail)?;
            let client_id = match ctx.api.encryption.clone() {
                Some(encryption) => tx::read(db, move |conn| {
                    platform_credential::resolve_for_owner(conn, &encryption, owner, &kind, false)
                })
                .await
                .map_err(fail)?
                .and_then(|c| {
                    c.text("clientId")
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                }),
                None => None,
            };
            Ok(json!({ "clientId": client_id }))
        }
        "listIntegrationTiktoks" | "listIntegrationZalo" | "listIntegrationThreads" => {
            let channel = match name {
                "listIntegrationZalo" => "zalo",
                "listIntegrationThreads" => "threads",
                _ => "tiktok",
            };
            let data = channel_integration::settings_list(db, channel, ws)
                .await
                .map_err(fail)?;
            Ok(json!({ "data": data }))
        }
        "disconnectTiktokAction" | "disconnectZaloAction" => {
            let id = bound_id(arg(args, 1))?;
            let (channel, label) = if name == "disconnectZaloAction" {
                ("zalo", "Zalo")
            } else {
                ("tiktok", "TikTok")
            };
            channel_integration::disconnect(db, channel, ws, id)
                .await
                .map_err(action_fail)?;
            audit(
                ctx,
                "disconnect",
                format!("disconnected the {label} channel (#{id})"),
            )
            .await;
            Ok(Value::Null)
        }
        "refreshTiktokTokenAction" => {
            let id = bound_id(arg(args, 1))?;
            channel_integration::refresh_tiktok_token(
                db,
                &HttpUpstream,
                &ctx.api.audit,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id,
            )
            .await
            .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "toggleTiktokCommentToMessageAction" => {
            let id = bound_id(arg(args, 1))?;
            let enabled = arg(args, 2)["enabled"].as_bool().ok_or_else(invalid)?;
            let status = channel_integration::set_comment_to_message(
                db,
                &HttpUpstream,
                &ctx.api.audit,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id,
                enabled,
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({ "status": status }))
        }
        "refreshTiktokCommentToMessageAction" => {
            let id = bound_id(arg(args, 1))?;
            let status = channel_integration::refresh_comment_to_message(
                db,
                &HttpUpstream,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id,
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({ "status": status }))
        }
        "toggleZaloTagSyncAction" => {
            let id = bound_id(arg(args, 1))?;
            let enabled = arg(args, 2)["enabled"].as_bool().ok_or_else(invalid)?;
            channel_integration::update_zalo_tag_sync(db, ws, id, enabled)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "reconnectZaloAction" => reconnect_channel(ctx, "zalo", bound_id(arg(args, 1))?).await,
        "reconnectThreadsAction" => {
            reconnect_channel(ctx, "threads", bound_id(arg(args, 1))?).await
        }
        // `disconnectThreads`: no audit in the original.
        "disconnectThreadsAction" => {
            channel_integration::disconnect(db, "threads", ws, bound_id(arg(args, 1))?)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "updateMarkReadOnOutboundAction" => {
            let id = bound_id(arg(args, 1))?;
            let enabled = arg(args, 2)["enabled"].as_bool().ok_or_else(invalid)?;
            let inbox = inbox_service::update_mark_read_on_outbound(db, ws, id, enabled)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "markReadOnOutbound": inbox.mark_read_on_outbound }))
        }
        _ => {
            let provider = email_provider(name).ok_or_else(invalid)?;
            if name.starts_with("connect") {
                connect_email(ctx, provider, arg(args, 1)).await
            } else {
                disconnect_email(ctx, provider).await
            }
        }
    }
}

/// `zodBigintAsString` of a bound argument.
fn bound_id(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse::<i64>().map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}

/// `reconnectZaloAction` / `reconnectThreadsAction`: the row must be in this
/// workspace; then the signed
/// connect-session reconnect of its `Connection` (`connections.reconnect`,
/// bound to this user and workspace), back to the channel settings. Answers
/// `{ url }` for the SPA to open (an action cannot `redirect()` here).
async fn reconnect_channel(ctx: &Ctx<'_>, channel: &str, id: i64) -> Result<Value, OrpcError> {
    let ws = ctx.workspace_id;
    let (table, missing) = match channel {
        "zalo" => ("IntegrationZalo", "Zalo integration not found"),
        "threads" => ("IntegrationThreads", "Threads integration not found"),
        _ => return Err(invalid()),
    };
    let connection_id: Option<i64> = tx::read(&ctx.api.db, move |conn| {
        Ok(conn
            .query_row(
                &format!(
                    r#"SELECT c."id" FROM "{table}" t
                       JOIN "Connection" c ON c."inboxId" = t."inboxId" AND c."workspaceId" = t."workspaceId"
                       WHERE t."id" = ?1 AND t."workspaceId" = ?2"#
                ),
                params![id, ws],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
    .map_err(fail)?;
    let connection_id =
        connection_id.ok_or_else(|| action_fail(BusinessError::not_found(missing)))?;
    let redirect = format!(
        "{}/space/{ws}/settings/channels?channel={channel}",
        ctx.api.app_url.trim_end_matches('/')
    );
    let out = super::public(
        ctx,
        "connections.reconnect",
        &[("id", connection_id.to_string())],
        None,
        Some(&json!({ "redirectUrl": redirect })),
    )
    .await?;
    let url = out["session"]["nextAction"]["url"]
        .as_str()
        .ok_or_else(invalid)?;
    Ok(json!({ "url": url }))
}

/// `toHttpsIcon`: only an `https:` icon URL reaches the page.
fn https_icon(icon: Option<&str>) -> Option<String> {
    let icon = icon.filter(|i| !i.is_empty())?;
    (url::Url::parse(icon).ok()?.scheme() == "https").then(|| icon.to_string())
}

/// `createCredentialConnectAction`: the provider schema (trimmed strings,
/// `apiUrl` a URL), then `connectFromCredentials` with `allowUpdate`.
async fn connect_email(
    ctx: &Ctx<'_>,
    provider: ApiKeyProvider,
    input: &Value,
) -> Result<Value, OrpcError> {
    let input = In(input);
    let mut config = Map::new();
    for field in credentials::config_fields(provider.provider) {
        let text = input.str(field.name, true, 1, usize::MAX)?;
        if field.kind == "url" && url::Url::parse(&text).is_err() {
            return Err(invalid());
        }
        config.insert(field.name.to_string(), Value::String(text));
    }
    // The ActiveCampaign probe goes to the user's `apiUrl`: SSRF-checked.
    api_key::connect(
        &ctx.api.db,
        &SafeUpstream,
        ctx.workspace_id,
        provider,
        &config,
        Some(ctx.user_id),
        true,
    )
    .await
    .map_err(|error| {
        tracing::error!(error = %error.message(), workspace_id = ctx.workspace_id, "Failed to connect {}", provider.display_name);
        action_fail(error)
    })?;
    Ok(Value::Null)
}

/// `createDisconnectAction`: through the workspace-singleton `Connection`
/// (audited here), else the legacy service disconnect (the `Integration`
/// row and its satellite).
async fn disconnect_email(ctx: &Ctx<'_>, provider: ApiKeyProvider) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let found = credentials::find_workspace_connection(db, ws, provider.provider)
        .await
        .map_err(action_fail)?;
    match found {
        Some(connection) => {
            let id = connection.id;
            credentials::disconnect_integration(db, connection)
                .await
                .map_err(action_fail)?;
            audit(
                ctx,
                "disconnect",
                format!(
                    "disconnected the {} integration (#{id})",
                    provider.display_name
                ),
            )
            .await;
        }
        None => {
            let key = provider.provider;
            tx::write(db, move |tx| {
                tx.execute(
                    r#"DELETE FROM "Integration" WHERE "workspaceId" = ?1 AND "integrationType" = ?2"#,
                    params![ws, key],
                )?;
                Ok(())
            })
            .await
            .map_err(action_fail)?;
        }
    }
    Ok(Value::Null)
}

/// `connectPaypalAction`: `assertWorkspaceSuperAdmin`, then the same
/// `paymentProviderService.connectPaypal` the public route makes.
async fn connect_paypal(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let client_id = input.str("clientId", true, 1, 512)?;
    let client_secret = input.str("clientSecret", true, 1, 512)?;
    if ctx.permissions.get("superAdmin") != Some(&Value::Bool(true)) {
        return Err(action_fail(BusinessError::exception(
            "You need to be a workspace super admin to do this.",
            "systemError",
            400,
        )));
    }
    let encryption = ctx.api.encryption.as_deref().ok_or_else(|| {
        action_fail(BusinessError::exception(
            "ENCRYPTION_KEY is not set",
            "systemError",
            500,
        ))
    })?;
    let pctx = payment_provider::Ctx {
        db: &ctx.api.db,
        encryption,
        app_url: &ctx.api.app_url,
    };
    payment_provider::connect_paypal(pctx, ctx.workspace_id, &client_id, &client_secret)
        .await
        .map_err(action_fail)?;
    Ok(Value::Null)
}

async fn audit(ctx: &Ctx<'_>, action: &str, detail: String) {
    ctx.api
        .audit
        .record(AuditRecordInput {
            user_id: Some(ctx.user_id.to_string()),
            workspace_id: Some(ctx.workspace_id.to_string()),
            ..AuditRecordInput::new(action, detail)
        })
        .await;
}
