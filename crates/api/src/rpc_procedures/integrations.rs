//! The workspace settings screens of channels and AI providers (P4):
//!
//! - the AI key connect / disconnect actions
//!   (`features/integration-{openai,claude,deepseek,gemini,muse,openrouter}/actions`,
//!   `lib/integration-actions.ts`) and the OpenAI-compatible ones,
//! - Telegram connect / disconnect (`features/integration-telegram/actions`),
//!   webchat delete (`features/integration-webchat/actions`),
//! - the server-component queries of those pages (`features/*/queries`), as
//!   actions with the same export name: the SPA sends `workspaceId`, so the
//!   `/rpc` membership check runs (`assertCurrentUserCanAccessChatbot`).

use chatbotx_business::BusinessError;
use chatbotx_business::audit::AuditRecordInput;
use chatbotx_business::connection::credentials::{self, HttpUpstream};
use chatbotx_business::integration_openai_compatible::{
    self as openai_compatible, ConnectInput, OpenaiCompatibleRow, ProviderCheck,
};
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_db::repo::models::{
    IntegrationApi, IntegrationClaude, IntegrationDeepseek, IntegrationGemini, IntegrationMuse,
    IntegrationOpenai, IntegrationOpenrouter, IntegrationTelegram,
};
use chatbotx_db::repo::{Model, find_many};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public};
use crate::orpc::OrpcError;

/// The workspace actions of this module.
pub(super) const ACTIONS: &[&str] = &[
    "connectClaudeAction",
    "connectDeepSeekAction",
    "connectGeminiAction",
    "connectMuseAction",
    "connectOpenAIAction",
    "connectOpenRouterAction",
    "disconnectClaudeAction",
    "disconnectDeepSeekAction",
    "disconnectGeminiAction",
    "disconnectMuseAction",
    "disconnectOpenAIAction",
    "disconnectOpenRouterAction",
    "connectOpenaiCompatibleAction",
    "disconnectOpenaiCompatibleAction",
    "disconnectTelegramAction",
    "deleteWebchatAction",
    "findIntegrationOpenAI",
    "findIntegrationClaude",
    "findIntegrationDeepSeek",
    "findIntegrationGemini",
    "findIntegrationMuse",
    "findIntegrationOpenRouter",
    "listIntegrationOpenaiCompatible",
    "hasAIIntegration",
    "listIntegrationApis",
    "listIntegrationTelegrams",
    "listIntegrationWebchats",
];

/// `t("validation.invalidApiKey")` (en).
const INVALID_API_KEY: &str = "Invalid API key";
/// `t("openaiCompatible.validation.invalidBaseURL")` (en).
const INVALID_BASE_URL: &str = "Base URL is invalid or not allowed.";
/// `t("openaiCompatible.validation.presetAlreadyConnected")` (en).
const PRESET_ALREADY_CONNECTED: &str = "This preset is already connected for this workspace.";

/// The AI key provider of a connect / disconnect / find export.
fn ai_provider(name: &str) -> Option<&'static str> {
    let stem = name
        .strip_prefix("connect")
        .or_else(|| name.strip_prefix("disconnect"))
        .or_else(|| name.strip_prefix("findIntegration"))?;
    Some(match stem.strip_suffix("Action").unwrap_or(stem) {
        "Claude" => "claude",
        "DeepSeek" => "deepseek",
        "Gemini" => "gemini",
        "Muse" => "muse",
        "OpenAI" => "openai",
        "OpenRouter" => "openrouter",
        _ => return None,
    })
}

/// `connectTelegramAction` (an `authActionClient` action). Only the
/// settings path (an existing workspace) is ported; without `workspaceId`
/// (a new workspace named after the bot) it is "not ported".
pub(super) async fn auth_action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if name != "connectTelegramAction" {
        return None;
    }
    let input = In(arg(args, 0));
    let workspace_id = match input.opt_id("workspaceId") {
        Ok(Some(id)) => id,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };
    Some(
        async {
            let bot_token = input.str("botToken", true, 1, usize::MAX)?;
            if !super::workspaces::is_member(ctx, workspace_id).await? {
                return Err(action_fail(BusinessError::exception(
                    "Workspace not found",
                    "notFound",
                    404,
                )));
            }
            let mut config = Map::new();
            config.insert("secretText".into(), Value::String(bot_token));
            let row = credentials::connect_telegram(
                &ctx.api.db,
                &HttpUpstream,
                workspace_id,
                &config,
                Some(ctx.user_id),
            )
            .await
            .map_err(action_fail)?;
            // `onConnected`: the webhook secret and `setWebhook`.
            if let Some(inbox_id) = row.inbox_id {
                let broker = std::env::var("NEXT_PUBLIC_BROKER_URL")
                    .ok()
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| ctx.api.app_url.clone());
                credentials::register_telegram_webhook(
                    &ctx.api.db,
                    &HttpUpstream,
                    workspace_id,
                    inbox_id,
                    &broker,
                )
                .await
                .map_err(action_fail)?;
            }
            if row.created_at == row.updated_at {
                let telegram_id = telegram_id_of_inbox(ctx, workspace_id, row.inbox_id).await?;
                audit(
                    ctx,
                    workspace_id,
                    "connect",
                    format!("connected a new Telegram channel (#{telegram_id})"),
                )
                .await;
            }
            Ok(json!({ "workspaceId": workspace_id.to_string() }))
        }
        .await,
    )
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
        "connectOpenaiCompatibleAction" => connect_openai_compatible(ctx, arg(args, 1)).await,
        "disconnectOpenaiCompatibleAction" => {
            let id = bind_id(args, 1)?;
            openai_compatible::disconnect(db, &ctx.api.audit, ws, id)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "disconnectTelegramAction" => disconnect_telegram(ctx, bind_id(args, 1)?).await,
        "deleteWebchatAction" => {
            let id = bind_id(args, 1)?.to_string();
            public(ctx, "webchats.delete", &[("id", id)], None, None).await?;
            Ok(Value::Null)
        }
        "listIntegrationOpenaiCompatible" => {
            let rows = openai_compatible::list(db, ws).await.map_err(fail)?;
            Ok(Value::Array(
                rows.iter().map(openai_compatible_json).collect(),
            ))
        }
        "hasAIIntegration" => {
            let found = read(ctx, move |conn| {
                Ok(conn.query_row(
                    r#"SELECT EXISTS (SELECT 1 FROM "Integration" WHERE "workspaceId" = ?1
                       AND "integrationType" IN ('openai', 'gemini', 'deepseek', 'claude',
                           'openrouter', 'muse', 'openaiCompatible'))"#,
                    params![ws],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .await?;
            Ok(Value::Bool(found))
        }
        "listIntegrationApis" => {
            let rows: Vec<IntegrationApi> = read(ctx, move |conn| {
                find_many(
                    conn,
                    r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#,
                    &[&ws],
                )
            })
            .await?;
            Ok(json!({
                "data": rows.iter().map(|row| pick(row.to_json(), &[
                    "id", "name", "tokenPrefix", "callbackUrl", "enabled", "createdAt",
                ])).collect::<Vec<_>>(),
            }))
        }
        "listIntegrationTelegrams" => {
            let rows: Vec<IntegrationTelegram> = read(ctx, move |conn| {
                find_many(conn, r#"WHERE "workspaceId" = ?1"#, &[&ws])
            })
            .await?;
            Ok(json!({
                "data": rows.iter().map(|row| without_secret(row.to_json())).collect::<Vec<_>>(),
            }))
        }
        "listIntegrationWebchats" => {
            let input = In(arg(args, 0));
            let mut query = Vec::new();
            for key in ["page", "perPage"] {
                if let Some(value) = input.get(key) {
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    query.push(format!("{key}={text}"));
                }
            }
            let mut list = public(ctx, "webchats.list", &[], Some(&query.join("&")), None).await?;
            // `integrationWebchatService.list` rows are the full model; the
            // row menu links with `workspaceId`.
            if let Some(Value::Array(items)) = list.get_mut("data") {
                for item in items {
                    item["workspaceId"] = Value::String(ws.to_string());
                }
            }
            Ok(list)
        }
        _ => {
            let provider = ai_provider(name).ok_or_else(invalid)?;
            if name.starts_with("connect") {
                connect_ai_key(ctx, provider, arg(args, 1)).await
            } else if name.starts_with("disconnect") {
                disconnect_ai_key(ctx, provider).await
            } else {
                let row = find_ai_provider(ctx, provider).await?;
                // `findIntegrationOpenAI` wraps the row; the others answer it.
                Ok(if provider == "openai" {
                    json!({ "data": row })
                } else {
                    row
                })
            }
        }
    }
}

// ---- AI key providers -----------------------------------------------------

/// `createAiKeyConnectAction`: the live key check (a rejected key is the
/// `apiKey` field error), then `connectFromCredentials` with `allowUpdate`.
async fn connect_ai_key(ctx: &Ctx<'_>, provider: &str, input: &Value) -> Result<Value, OrpcError> {
    let Some(provider) = credentials::ai_key_provider(provider) else {
        return Err(invalid());
    };
    let config: Map<String, Value> = ["apiKey", "model", "temperature", "maxOutputTokens"]
        .into_iter()
        .filter_map(|key| {
            input
                .get(key)
                .filter(|v| !v.is_null())
                .map(|v| (key.to_string(), v.clone()))
        })
        .collect();
    match credentials::connect_ai_key_with(
        &ctx.api.db,
        &HttpUpstream,
        ctx.workspace_id,
        provider,
        &config,
        None,
        true,
    )
    .await
    {
        Ok(_) => Ok(Value::Null),
        Err(error) if error.code() == Some("connectionCredentialsRejected") => {
            Err(field_error("apiKey", INVALID_API_KEY))
        }
        Err(error) => Err(action_fail(error)),
    }
}

/// `createDisconnectAction`: through the workspace-singleton `Connection`
/// when there is one, else the legacy service disconnect (the `Integration`
/// row and its satellite).
async fn disconnect_ai_key(ctx: &Ctx<'_>, provider: &str) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let display = credentials::ai_key_provider(provider)
        .map(|p| p.display_name)
        .unwrap_or(provider);
    match credentials::find_workspace_connection(db, ws, provider)
        .await
        .map_err(action_fail)?
    {
        Some(connection) => {
            let id = connection.id;
            credentials::disconnect_integration(db, connection)
                .await
                .map_err(action_fail)?;
            audit(
                ctx,
                ws,
                "disconnect",
                format!("disconnected the {display} integration (#{id})"),
            )
            .await;
        }
        None => {
            let provider = provider.to_string();
            write(ctx, move |tx| {
                tx.execute(
                    r#"DELETE FROM "Integration" WHERE "workspaceId" = ?1 AND "integrationType" = ?2"#,
                    params![ws, provider],
                )?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(Value::Null)
}

/// `integration<Name>Service.findByWorkspaceId`: the satellite row, `null`
/// when not connected. `auth` keeps only `authType` (the UI reads its
/// presence; the key stays on the server).
async fn find_ai_provider(ctx: &Ctx<'_>, provider: &str) -> Result<Value, OrpcError> {
    fn first<M: Model>(
        conn: &Connection,
        ws: i64,
        json: fn(&M) -> Value,
    ) -> chatbotx_db::repo::Result<Option<Value>> {
        let rows: Vec<M> = find_many(conn, r#"WHERE "workspaceId" = ?1 LIMIT 1"#, &[&ws])?;
        Ok(rows.first().map(|row| without_secret(json(row))))
    }
    let ws = ctx.workspace_id;
    let provider = provider.to_string();
    let row = read(ctx, move |conn| {
        Ok(match provider.as_str() {
            "claude" => first(conn, ws, IntegrationClaude::to_json)?,
            "deepseek" => first(conn, ws, IntegrationDeepseek::to_json)?,
            "gemini" => first(conn, ws, IntegrationGemini::to_json)?,
            "muse" => first(conn, ws, IntegrationMuse::to_json)?,
            "openai" => first(conn, ws, IntegrationOpenai::to_json)?,
            _ => first(conn, ws, IntegrationOpenrouter::to_json)?,
        })
    })
    .await?;
    Ok(row.unwrap_or(Value::Null))
}

// ---- OpenAI-compatible ----------------------------------------------------

/// `connectOpenaiCompatibleAction`: base URL check, provider probe, then a
/// new integration (a taken preset is the `preset` field error).
// ponytail: the legacy service insert (no `Connection` row): the connection
// engine has no `openaiCompatible` adapter in Rust yet.
async fn connect_openai_compatible(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let api_key = input.str("apiKey", true, 1, usize::MAX)?;
    let name = input.str("name", true, 1, 255)?;
    let base_url = input.str("baseURL", true, 1, 2048)?;
    let preset = input
        .opt_str("preset")?
        .unwrap_or_else(|| "lmstudio".into());
    let Some(preset_model) = openai_compatible::default_model_of(&preset) else {
        return Err(invalid());
    };
    let default_model = match input.opt_str("defaultModel")? {
        Some(model) => {
            let model = model.trim().to_string();
            if model.is_empty() || model.chars().count() > 255 {
                return Err(invalid());
            }
            model
        }
        None => preset_model.to_string(),
    };
    let cloud = chatbotx_business::ai_mcp_server::is_cloud();
    let base_url = match openai_compatible::validate_base_url_for_environment(
        &base_url, cloud, &HttpDoh,
    )
    .await
    {
        Ok(url) => url,
        Err(error) if matches!(error.code(), Some("invalidBaseUrl" | "ssrfBlocked")) => {
            return Err(field_error("baseURL", INVALID_BASE_URL));
        }
        Err(error) => return Err(action_fail(error)),
    };
    match openai_compatible::verify_provider(Some(&api_key), &base_url, cloud, &HttpDoh)
        .await
        .map_err(action_fail)?
    {
        ProviderCheck::UnsafeBaseUrl => return Err(field_error("baseURL", INVALID_BASE_URL)),
        ProviderCheck::Unauthorized(_) => return Err(field_error("apiKey", INVALID_API_KEY)),
        ProviderCheck::Ok | ProviderCheck::CouldNotVerify => {}
    }
    let data = ConnectInput {
        name,
        preset,
        base_url,
        default_model,
        api_key,
        auto_reply: Some(input.opt_bool("autoReply")?.unwrap_or(false)),
        enabled: Some(input.opt_bool("enabled")?.unwrap_or(true)),
    };
    match openai_compatible::connect(
        &ctx.api.db,
        &ctx.api.audit,
        ctx.workspace_id,
        data,
        cloud,
        &HttpDoh,
    )
    .await
    {
        Ok(_) => Ok(Value::Null),
        Err(error) if error.code() == Some("openaiCompatiblePresetAlreadyConnected") => {
            Err(field_error("preset", PRESET_ALREADY_CONNECTED))
        }
        Err(error) => Err(action_fail(error)),
    }
}

/// `mapIntegrationOpenaiCompatibleResource`.
pub(crate) fn openai_compatible_json(row: &OpenaiCompatibleRow) -> Value {
    json!({
        "id": row.id.to_string(),
        "name": row.name,
        "preset": row.preset,
        "baseURL": row.base_url,
        "defaultModel": row.default_model,
        "enabled": row.enabled,
        "autoReply": row.auto_reply,
        "createdAt": crate::iso8601::format_millis(u64::try_from(row.created_at).unwrap_or_default()),
        "updatedAt": crate::iso8601::format_millis(u64::try_from(row.updated_at).unwrap_or_default()),
    })
}

// ---- Telegram -------------------------------------------------------------

/// `disconnectTelegramAction`: the channel's `Connection` goes through
/// `connectionService.disconnect` (`deleteWebhook`, then the inbox mirror).
async fn disconnect_telegram(ctx: &Ctx<'_>, id: i64) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let rows: Vec<IntegrationTelegram> = read(ctx, move |conn| {
        find_many(
            conn,
            r#"WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &ws],
        )
    })
    .await?;
    let telegram = rows
        .into_iter()
        .next()
        .ok_or_else(|| action_fail(BusinessError::not_found("Integration Telegram not found")))?;
    let inbox_id = telegram.inbox_id;
    let connection_id: Option<i64> = read(ctx, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        Ok(conn
            .query_row(
                r#"SELECT "id" FROM "Connection" WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
                params![inbox_id, ws],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await?;
    let connection = match connection_id {
        Some(connection_id) => {
            chatbotx_business::connection::get_for_workspace(db, connection_id, ws)
                .await
                .map_err(action_fail)?
        }
        None => None,
    }
    .ok_or_else(|| action_fail(BusinessError::not_found("Connection not found")))?;
    credentials::disconnect_telegram(db, &HttpUpstream, connection)
        .await
        .map_err(action_fail)?;
    audit(
        ctx,
        ws,
        "disconnect",
        format!("disconnected the Telegram channel (#{id})"),
    )
    .await;
    Ok(Value::Null)
}

async fn telegram_id_of_inbox(
    ctx: &Ctx<'_>,
    ws: i64,
    inbox_id: Option<i64>,
) -> Result<i64, OrpcError> {
    let Some(inbox_id) = inbox_id else {
        return Ok(0);
    };
    let rows: Vec<IntegrationTelegram> = read(ctx, move |conn| {
        find_many(
            conn,
            r#"WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
            &[&inbox_id, &ws],
        )
    })
    .await?;
    Ok(rows.first().map_or(0, |row| row.id))
}

// ---- helpers --------------------------------------------------------------

/// The bind argument at `index` (`workspaceIdAndIdRequestParams`).
fn bind_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    match arg(args, index) {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// `returnValidationErrors(schema, { [field]: { _errors: [message] } })`.
fn field_error(field: &str, message: &str) -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some(message));
    error.data = Some(json!({ "validationErrors": { field: { "_errors": [message] } } }));
    error
}

/// `auth` without the secret: `{ authType }`.
fn without_secret(mut row: Value) -> Value {
    if let Some(auth) = row.get("auth").filter(|auth| !auth.is_null()) {
        let auth_type = auth.get("authType").cloned().unwrap_or(Value::Null);
        row["auth"] = json!({ "authType": auth_type });
    }
    row
}

fn pick(row: Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| row.get(*key).map(|v| ((*key).to_string(), v.clone())))
            .collect(),
    )
}

async fn audit(ctx: &Ctx<'_>, workspace_id: i64, action: &str, detail: String) {
    ctx.api
        .audit
        .record(AuditRecordInput {
            user_id: Some(ctx.user_id.to_string()),
            workspace_id: Some(workspace_id.to_string()),
            ..AuditRecordInput::new(action, detail)
        })
        .await;
}

async fn read<T: Send + 'static>(
    ctx: &Ctx<'_>,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<T> + Send + 'static,
) -> Result<T, OrpcError> {
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        f(conn).map_err(BusinessError::Repo)
    })
    .await
    .map_err(fail)
}

async fn write<T: Send + 'static>(
    ctx: &Ctx<'_>,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<T> + Send + 'static,
) -> Result<T, OrpcError> {
    chatbotx_business::tx::write(&ctx.api.db, move |conn| {
        f(conn).map_err(BusinessError::Repo)
    })
    .await
    .map_err(action_fail)
}
