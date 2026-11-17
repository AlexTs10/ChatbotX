//! Credential connects (`packages/connections/src/credentials.ts`
//! `connectFromCredentials`, `internal.ts` `parseConfig` / `connectAndPersist`,
//! `business/src/connection/credential-providers.ts`, `upsert.ts`,
//! `integration-ai-provider/verify.ts`) for the AI key providers, plus their
//! `verify` / `disconnect` (`lifecycle.ts`). P3.6-D2 connections rest.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::repo::{self, new_id};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Map, Value, json};

use super::{COLUMNS, ConnectionRow, find_or_fail, repo_err, row};
use crate::{BusinessError, Result, tx};

/// One `ConnectionConfigField` of the catalog (`label` = the name: the
/// builder's `integrations.<p>.fields.*` keys are not in the API catalog).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigField {
    pub name: &'static str,
    /// `string` / `secret` / `url` / ...
    pub kind: &'static str,
    pub required: bool,
}

const fn secret(name: &'static str) -> ConfigField {
    ConfigField {
        name,
        kind: "secret",
        required: true,
    }
}

const fn url(name: &'static str) -> ConfigField {
    ConfigField {
        name,
        kind: "url",
        required: true,
    }
}

/// `adapter.provider.configFields` of an integration-kind provider.
pub fn config_fields(provider: &str) -> &'static [ConfigField] {
    const API_KEY: &[ConfigField] = &[secret("apiKey")];
    const ACTIVE_CAMPAIGN: &[ConfigField] = &[url("apiUrl"), secret("apiKey")];
    const DRIP: &[ConfigField] = &[secret("apiToken")];
    const OPENAI_COMPATIBLE: &[ConfigField] = &[url("baseURL"), secret("apiKey")];
    match provider {
        "activeCampaign" => ACTIVE_CAMPAIGN,
        "drip" => DRIP,
        "openaiCompatible" => OPENAI_COMPATIBLE,
        "claude" | "deepseek" | "gemini" | "getResponse" | "klaviyo" | "mailchimp"
        | "mailerLite" | "moosend" | "muse" | "openai" | "openrouter" | "sendGrid" => API_KEY,
        _ => &[],
    }
}

/// `adapter.credentialType` of an integration-kind provider (the platform
/// credential its OAuth connect needs).
pub fn credential_type(provider: &str) -> Option<&'static str> {
    match provider {
        "googleAds" => Some("googleAds"),
        "googleCalendar" | "googleSheets" => Some("google"),
        _ => None,
    }
}

/// The AI key provider of `credential-providers.ts` (`makeAiKeyProvider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiKeyProvider {
    pub provider: &'static str,
    pub display_name: &'static str,
    /// The satellite table.
    pub table: &'static str,
    /// `AI_KEY_PROVIDER_DEFAULTS`.
    pub model: &'static str,
    pub max_output_tokens: i64,
    /// The "list models" probe.
    pub verify_url: &'static str,
    /// HTTP statuses that mean "invalid key".
    pub invalid_statuses: &'static [u16],
}

const UNAUTHORIZED: &[u16] = &[401, 403];

/// The AI key provider of `provider` (not `openaiCompatible`).
pub fn ai_key_provider(provider: &str) -> Option<AiKeyProvider> {
    let p = |provider, display_name, table, model, max_output_tokens, verify_url, invalid| {
        Some(AiKeyProvider {
            provider,
            display_name,
            table,
            model,
            max_output_tokens,
            verify_url,
            invalid_statuses: invalid,
        })
    };
    match provider {
        "claude" => p(
            "claude",
            "Claude",
            "IntegrationClaude",
            "claude-sonnet-4-6",
            1024,
            "https://api.anthropic.com/v1/models",
            UNAUTHORIZED,
        ),
        "deepseek" => p(
            "deepseek",
            "DeepSeek",
            "IntegrationDeepseek",
            "deepseek-flash",
            1024,
            "https://api.deepseek.com/models",
            UNAUTHORIZED,
        ),
        "gemini" => p(
            "gemini",
            "Gemini",
            "IntegrationGemini",
            "gemini-3.5-flash",
            1024,
            "https://generativelanguage.googleapis.com/v1beta/models",
            &[400, 401, 403],
        ),
        "muse" => p(
            "muse",
            "Muse",
            "IntegrationMuse",
            "muse-spark-1.3",
            4096,
            "https://api.meta.ai/v1/models",
            UNAUTHORIZED,
        ),
        "openai" => p(
            "openai",
            "OpenAI",
            "IntegrationOpenai",
            "gpt-5.4-mini",
            1024,
            "https://api.openai.com/v1/models",
            UNAUTHORIZED,
        ),
        "openrouter" => p(
            "openrouter",
            "OpenRouter",
            "IntegrationOpenrouter",
            "openai/gpt-5.4-mini",
            1024,
            "https://openrouter.ai/api/v1/key",
            UNAUTHORIZED,
        ),
        _ => None,
    }
}

impl AiKeyProvider {
    /// The probe headers of `verifyConfigByProvider`.
    fn headers(&self, api_key: &str) -> Vec<(String, String)> {
        match self.provider {
            "claude" => vec![
                ("x-api-key".into(), api_key.into()),
                ("anthropic-version".into(), "2023-06-01".into()),
            ],
            "gemini" => vec![("x-goog-api-key".into(), api_key.into())],
            _ => vec![("authorization".into(), format!("Bearer {api_key}"))],
        }
    }

    /// `AI_KEY_PROVIDER_CONFIG_COLUMNS` (+ `autoReplyVoice` / `voice` for
    /// openai).
    fn config_columns(&self) -> &'static [&'static str] {
        if self.provider == "openai" {
            &[
                "model",
                "maxOutputTokens",
                "prompt",
                "temperature",
                "autoReply",
                "autoReplyVoice",
                "voice",
            ]
        } else {
            &[
                "model",
                "maxOutputTokens",
                "prompt",
                "temperature",
                "autoReply",
            ]
        }
    }
}

/// One outbound HTTP request of a provider check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// The status and body of an answer; `Err` = no answer (network, timeout).
pub type UpstreamReply = std::result::Result<(u16, String), String>;

/// The provider HTTP client (injected; `HttpUpstream` in production).
pub trait Upstream: Send + Sync {
    fn send(&self, request: UpstreamRequest) -> impl Future<Output = UpstreamReply> + Send;
}

/// reqwest, 10 s timeout, no retries (`ky` with `retry: 0`).
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpUpstream;

fn http_client() -> std::result::Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Upstream for HttpUpstream {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        let client = http_client()?;
        let method =
            reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut builder = client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder
                .header("content-type", "application/json")
                .body(body);
        }
        let response = builder.send().await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Ok((status, body))
    }
}

/// `AiKeyValidation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyValidation {
    Valid,
    Invalid,
    Unknown,
}

/// `verifyAiProviderApiKey`: a 2xx is valid, an invalid status is invalid,
/// anything else (other status, no answer) is unknown.
pub async fn verify_ai_key(
    upstream: &impl Upstream,
    provider: &AiKeyProvider,
    api_key: &str,
) -> KeyValidation {
    let reply = upstream
        .send(UpstreamRequest {
            method: "GET",
            url: provider.verify_url.to_string(),
            headers: provider.headers(api_key),
            body: None,
        })
        .await;
    match reply {
        Ok((status, _)) if (200..300).contains(&status) => KeyValidation::Valid,
        Ok((status, _)) if provider.invalid_statuses.contains(&status) => KeyValidation::Invalid,
        Ok((status, _)) => {
            tracing::warn!(
                provider = provider.provider,
                status,
                "AI provider API key verification was inconclusive"
            );
            KeyValidation::Unknown
        }
        Err(error) => {
            tracing::warn!(provider = provider.provider, error = %error, "AI provider API key verification was inconclusive");
            KeyValidation::Unknown
        }
    }
}

/// `parseConfig`: each declared field, coerced to its type (only the string
/// types are used by the ported providers).
pub fn parse_config(
    fields: &[ConfigField],
    raw: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut parsed = Map::new();
    for field in fields {
        let value = raw.get(field.name);
        let missing =
            matches!(value, None | Some(Value::Null)) || value.and_then(Value::as_str) == Some("");
        if missing {
            if field.required {
                return Err(BusinessError::validation(
                    field.name,
                    format!("{} is required", field.name),
                ));
            }
            continue;
        }
        match value {
            Some(Value::String(text)) => {
                parsed.insert(field.name.to_string(), Value::String(text.clone()));
            }
            _ => {
                return Err(BusinessError::validation(
                    field.name,
                    format!("{} must be a string", field.name),
                ));
            }
        }
    }
    Ok(parsed)
}

pub(super) fn credentials_rejected() -> BusinessError {
    // `toPublicErrorMessage(ConnectionProviderRejectedError, fallback)`: not a
    // `ChatbotXException`, so the fallback.
    BusinessError::exception(
        "The provided credentials were rejected.",
        "connectionCredentialsRejected",
        400,
    )
}

fn provider_unavailable() -> BusinessError {
    provider_unavailable_status(502)
}

/// `connectionProviderUnavailableException(retryStatus)` (502 or 503).
pub(super) fn provider_unavailable_status(status: u16) -> BusinessError {
    BusinessError::exception(
        "The provider is temporarily unavailable. Please try again.",
        "connectionProviderUnavailable",
        status,
    )
}

pub(crate) fn already_connected() -> BusinessError {
    BusinessError::exception(
        "This provider is already connected in this workspace.",
        "connectionAlreadyConnected",
        409,
    )
}

fn is_active(status: &str) -> bool {
    matches!(status, "connected" | "degraded" | "needs_reauth")
}

/// `connectionRepository.findByProviderSourceId` (private query).
fn find_by_source(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    source_id: &str,
) -> Result<Option<ConnectionRow>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "Connection"
               WHERE "workspaceId" = ?1 AND "provider" = ?2 AND "sourceId" = ?3"#
        ),
        params![workspace_id, provider, source_id],
        row,
    )
    .optional()
    .map_err(repo_err)
}

/// A satellite column value from the request `config` (unvalidated, as the
/// original: the column constraints are the check).
fn sql_value(value: &Value) -> chatbotx_db::rusqlite::types::Value {
    use chatbotx_db::rusqlite::types::Value as Sql;
    match value {
        Value::Null => Sql::Null,
        Value::Bool(flag) => Sql::Integer(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .map(Sql::Integer)
            .or_else(|| number.as_f64().map(Sql::Real))
            .unwrap_or(Sql::Null),
        Value::String(text) => Sql::Text(text.clone()),
        other => Sql::Text(other.to_string()),
    }
}

/// `connectFromCredentials` for an AI key provider: config check, the live
/// key check, then the `Integration` + satellite rows and the `connected`
/// `Connection` (a disconnected row of the same source is revived).
pub async fn connect_ai_key(
    db: &Db,
    upstream: &impl Upstream,
    workspace_id: i64,
    provider: AiKeyProvider,
    config: &Map<String, Value>,
    actor_user_id: Option<i64>,
) -> Result<ConnectionRow> {
    connect_ai_key_with(
        db,
        upstream,
        workspace_id,
        provider,
        config,
        actor_user_id,
        false,
    )
    .await
}

/// [`connect_ai_key`] with `allowUpdate`: `true` (the builder connect
/// actions) updates an active connection of the same source in place
/// instead of answering `connectionAlreadyConnected`.
pub async fn connect_ai_key_with(
    db: &Db,
    upstream: &impl Upstream,
    workspace_id: i64,
    provider: AiKeyProvider,
    config: &Map<String, Value>,
    actor_user_id: Option<i64>,
    allow_update: bool,
) -> Result<ConnectionRow> {
    let fields = config_fields(provider.provider);
    let parsed = parse_config(fields, config)?;
    let columns = provider.config_columns();
    let extra: Vec<(String, Value)> = config
        .iter()
        .filter(|(key, _)| {
            !fields.iter().any(|field| field.name == key.as_str())
                || columns.contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let rejected: Vec<&str> = extra
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| !columns.contains(key))
        .collect();
    if let Some(first) = rejected.first() {
        return Err(BusinessError::validation(
            first,
            format!(
                "Unsupported config field(s) for {}: {}",
                provider.provider,
                rejected.join(", ")
            ),
        ));
    }
    let api_key = parsed
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match verify_ai_key(upstream, &provider, &api_key).await {
        KeyValidation::Valid => {}
        KeyValidation::Invalid => return Err(credentials_rejected()),
        KeyValidation::Unknown => return Err(provider_unavailable()),
    }
    let auth = json!({ "authType": "secretText", "secretText": api_key }).to_string();
    let target = SatelliteTarget {
        provider: provider.provider,
        display_name: provider.display_name,
        table: provider.table,
        ai_defaults: Some((provider.model, provider.max_output_tokens)),
    };
    persist_key_connection(
        db,
        workspace_id,
        target,
        auth,
        extra,
        actor_user_id,
        allow_update,
    )
    .await
}

/// The `Connection` + satellite target of a key connect
/// (`makeWorkspaceIntegrationBinding`).
#[derive(Debug, Clone, Copy)]
pub(super) struct SatelliteTarget {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub table: &'static str,
    /// `AI_KEY_PROVIDER_DEFAULTS` (`model`, `maxOutputTokens`); `None` for a
    /// satellite with only `auth`.
    pub ai_defaults: Option<(&'static str, i64)>,
}

/// The write half of `connectFromCredentials`: revive / update / insert the
/// satellite and the `connected` `Connection`.
pub(super) async fn persist_key_connection(
    db: &Db,
    workspace_id: i64,
    provider: SatelliteTarget,
    auth: String,
    extra: Vec<(String, Value)>,
    actor_user_id: Option<i64>,
    allow_update: bool,
) -> Result<ConnectionRow> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let existing = find_by_source(tx, workspace_id, provider.provider, "workspace")?;
        if !allow_update && existing.as_ref().is_some_and(|row| is_active(&row.status)) {
            return Err(already_connected());
        }
        // `saveOrInsertSatellite`: update the satellite of a revived row, else
        // a new `Integration` + satellite.
        let mut integration_id = None;
        if let Some(existing_id) = existing.as_ref().and_then(|row| row.integration_id) {
            let mut sets = vec![
                r#""auth" = ?3"#.to_string(),
                r#""updatedAt" = ?4"#.to_string(),
            ];
            let mut binds: Vec<chatbotx_db::rusqlite::types::Value> = vec![
                existing_id.into(),
                workspace_id.into(),
                auth.clone().into(),
                now.into(),
            ];
            for (key, value) in &extra {
                binds.push(sql_value(value));
                sets.push(format!(r#""{key}" = ?{}"#, binds.len()));
            }
            let updated = tx
                .execute(
                    &format!(
                        r#"UPDATE "{}" SET {} WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                        provider.table,
                        sets.join(", ")
                    ),
                    chatbotx_db::rusqlite::params_from_iter(binds.iter()),
                )
                .map_err(repo_err)?;
            if updated > 0 {
                integration_id = Some(existing_id);
            }
        }
        let integration_id = match integration_id {
            Some(id) => id,
            None => insert_satellite(tx, workspace_id, &provider, &auth, &extra, now)?,
        };
        let id = match existing {
            Some(existing) => {
                tx.execute(
                    r#"UPDATE "Connection" SET "sourceId" = 'workspace', "integrationId" = ?3,
                           "displayName" = ?4, "lastError" = NULL, "status" = 'connected',
                           "statusReason" = NULL, "connectedAt" = ?5, "disconnectedAt" = NULL,
                           "updatedAt" = ?5
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![
                        existing.id,
                        workspace_id,
                        integration_id,
                        provider.display_name,
                        now
                    ],
                )
                .map_err(repo_err)?;
                existing.id
            }
            None => {
                let id = new_id();
                tx.execute(
                    r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId",
                           "provider", "kind", "integrationId", "sourceId", "displayName",
                           "status", "createdBy", "connectedAt")
                       VALUES (?1, ?2, ?2, ?3, ?4, 'integration', ?5, 'workspace', ?6,
                           'connected', ?7, ?2)"#,
                    params![
                        id,
                        now,
                        workspace_id,
                        provider.provider,
                        integration_id,
                        provider.display_name,
                        actor_user_id
                    ],
                )
                .map_err(|error| unique_or(error, "Connection"))?;
                id
            }
        };
        find_or_fail(tx, id, workspace_id)
    })
    .await
}

/// A unique violation on `table` is `connectionAlreadyConnected`.
fn unique_or(error: chatbotx_db::rusqlite::Error, table: &str) -> BusinessError {
    if error
        .to_string()
        .contains(&format!("UNIQUE constraint failed: {table}"))
    {
        already_connected()
    } else {
        repo_err(error)
    }
}

/// `makeWorkspaceIntegrationBinding.insertRow`: the parent `Integration` and
/// the satellite (`defaultConfigValues`, then the request config, then the
/// system columns).
fn insert_satellite(
    conn: &Connection,
    workspace_id: i64,
    provider: &SatelliteTarget,
    auth: &str,
    extra: &[(String, Value)],
    now: i64,
) -> Result<i64> {
    let integration_id = new_id();
    conn.execute(
        r#"INSERT INTO "Integration" ("id", "createdAt", "updatedAt", "workspaceId", "integrationType")
           VALUES (?1, ?2, ?2, ?3, ?4)"#,
        params![integration_id, now, workspace_id, provider.provider],
    )
    .map_err(repo_err)?;
    let mut values: Vec<(String, chatbotx_db::rusqlite::types::Value)> = Vec::new();
    if let Some((model, max_output_tokens)) = provider.ai_defaults {
        values.push(("model".into(), model.to_string().into()));
        values.push(("maxOutputTokens".into(), max_output_tokens.into()));
    }
    for (key, value) in extra {
        values.retain(|(column, _)| column != key);
        values.push((key.clone(), sql_value(value)));
    }
    values.extend([
        ("id".to_string(), new_id().into()),
        ("createdAt".to_string(), now.into()),
        ("updatedAt".to_string(), now.into()),
        ("workspaceId".to_string(), workspace_id.into()),
        ("integrationId".to_string(), integration_id.into()),
        ("auth".to_string(), auth.to_string().into()),
    ]);
    let columns: Vec<String> = values.iter().map(|(c, _)| format!(r#""{c}""#)).collect();
    let marks: Vec<String> = (1..=values.len()).map(|i| format!("?{i}")).collect();
    conn.execute(
        &format!(
            r#"INSERT INTO "{}" ({}) VALUES ({})"#,
            provider.table,
            columns.join(", "),
            marks.join(", ")
        ),
        chatbotx_db::rusqlite::params_from_iter(values.iter().map(|(_, v)| v)),
    )
    .map_err(|error| unique_or(error, provider.table))?;
    Ok(integration_id)
}

/// The stored `secretText` of an AI key connection's satellite.
fn load_api_key(
    conn: &Connection,
    provider: &AiKeyProvider,
    current: &ConnectionRow,
) -> Result<String> {
    let auth: Option<String> = conn
        .query_row(
            &format!(
                r#"SELECT "auth" FROM "{}" WHERE "integrationId" = ?1 AND "workspaceId" = ?2"#,
                provider.table
            ),
            params![current.integration_id, current.workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)?;
    let auth = auth.ok_or_else(|| {
        BusinessError::Repo(repo::Error::msg(format!(
            "Unable to load auth for {} integration {}",
            provider.table,
            current.integration_id.unwrap_or_default()
        )))
    })?;
    Ok(serde_json::from_str::<Value>(&auth)
        .ok()
        .and_then(|value| {
            value
                .get("secretText")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default())
}

/// `connectionService.verify` of an AI key connection (the caller checked
/// that it is active): the live key check, then `verify.ok` (connected),
/// `markUnhealthy(token_revoked)` (needs_reauth) or `verify.failed_non_auth`
/// (degraded, `verify_failed`).
pub async fn verify_ai_key_connection(
    db: &Db,
    upstream: &impl Upstream,
    provider: AiKeyProvider,
    current: ConnectionRow,
) -> Result<ConnectionRow> {
    let key_row = current.clone();
    let api_key = tx::read(db, move |conn| load_api_key(conn, &provider, &key_row)).await?;
    let (status, reason) = match verify_ai_key(upstream, &provider, &api_key).await {
        KeyValidation::Valid => ("connected", None),
        KeyValidation::Invalid => ("needs_reauth", Some("token_revoked")),
        KeyValidation::Unknown => ("degraded", Some("verify_failed")),
    };
    set_health(db, current, status, reason).await
}

/// The `verify.ok` / `markUnhealthy(token_revoked)` / `verify.failed_non_auth`
/// write of a verify.
pub(super) async fn set_health(
    db: &Db,
    current: ConnectionRow,
    status: &'static str,
    reason: Option<&'static str>,
) -> Result<ConnectionRow> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "Connection" SET "status" = ?3, "statusReason" = ?4, "updatedAt" = ?5
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![current.id, current.workspace_id, status, reason, now],
        )
        .map_err(repo_err)?;
        find_or_fail(tx, current.id, current.workspace_id)
    })
    .await
}

/// `connectionService.disconnect` of an integration with a workspace
/// satellite: `user.disconnect`, then the parent `Integration` row is
/// deleted (the FK cascade removes the satellite and the `Connection`). The
/// answer is the transitioned row.
pub async fn disconnect_integration(db: &Db, current: ConnectionRow) -> Result<ConnectionRow> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let updated = super::mark_disconnected(tx, &current, now)?;
        if let Some(integration_id) = current.integration_id {
            tx.execute(
                r#"DELETE FROM "Integration" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![integration_id, current.workspace_id],
            )
            .map_err(repo_err)?;
        }
        Ok(updated)
    })
    .await
}

/// `connectionStateService.findByProviderSourceId({ provider, sourceId:
/// "workspace" })`: the workspace-singleton connection of `provider`.
pub async fn find_workspace_connection(
    db: &Db,
    workspace_id: i64,
    provider: &str,
) -> Result<Option<ConnectionRow>> {
    let provider = provider.to_string();
    tx::read(db, move |conn| {
        find_by_source(conn, workspace_id, &provider, "workspace")
    })
    .await
}

/// The providers with a live (not `disconnected`) connection in the
/// workspace (`connectionStateService.listProvidersWithStatus`).
pub async fn live_providers(db: &Db, workspace_id: i64) -> Result<Vec<String>> {
    tx::read(db, move |conn| {
        let mut statement = conn
            .prepare(
                r#"SELECT DISTINCT "provider" FROM "Connection"
                   WHERE "workspaceId" = ?1 AND "status" <> 'disconnected'"#,
            )
            .map_err(repo_err)?;
        let rows = statement
            .query_map(params![workspace_id], |r| r.get::<_, String>(0))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `platformCredentialService.resolveForOwner` presence: the owner's own
/// row (not set to use the platform one), else the platform row.
pub async fn has_platform_credential(
    db: &Db,
    owner_id: Option<i64>,
    credential_type: &'static str,
) -> Result<bool> {
    tx::read(db, move |conn| {
        // ponytail: presence only; the original also fails (no credential)
        // when the stored config does not decrypt.
        let found: Option<i64> = conn
            .query_row(
                r#"SELECT 1 FROM "PlatformCredential"
                   WHERE "type" = ?2 AND (("userId" = ?1 AND NOT "usePlatformCredential")
                       OR "userId" IS NULL)
                   LIMIT 1"#,
                params![owner_id, credential_type],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        Ok(found.is_some())
    })
    .await
}

// ---- telegram (`integrations/telegram` `connection` block) -----------------

/// `https://api.telegram.org/bot<token>/<method>`.
fn telegram_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{token}/{method}")
}

/// The outcome of a Telegram call: `Ok` = 2xx, `Err(Some(status))` = an
/// HTTP error, `Err(None)` = no answer.
async fn telegram_call(
    upstream: &impl Upstream,
    token: &str,
    method: &'static str,
    http_method: &'static str,
    body: Option<Value>,
) -> std::result::Result<(), Option<u16>> {
    let reply = upstream
        .send(UpstreamRequest {
            method: http_method,
            url: telegram_url(token, method),
            headers: Vec::new(),
            body: body.map(|b| b.to_string()),
        })
        .await;
    match reply {
        Ok((status, _)) if (200..300).contains(&status) => Ok(()),
        Ok((status, _)) => Err(Some(status)),
        Err(error) => {
            tracing::error!(error = %error, "Telegram API call failed: {method}");
            Err(None)
        }
    }
}

/// `describe(auth)`: the bot id before the `:` of the token.
fn telegram_bot_id(token: &str) -> Option<&str> {
    let (bot_id, _) = token.split_once(':')?;
    (!bot_id.is_empty() && bot_id.bytes().all(|b| b.is_ascii_digit())).then_some(bot_id)
}

/// `connectFromCredentials` for `telegram`: `getMe`, then a `connected`
/// channel (`Inbox` + `IntegrationTelegram` + `Connection`; a disconnected
/// row of the same bot is revived with its inbox).
pub async fn connect_telegram(
    db: &Db,
    upstream: &impl Upstream,
    workspace_id: i64,
    config: &Map<String, Value>,
    actor_user_id: Option<i64>,
) -> Result<ConnectionRow> {
    const FIELDS: &[ConfigField] = &[secret("secretText")];
    let parsed = parse_config(FIELDS, config)?;
    if let Some(key) = config.keys().find(|key| key.as_str() != "secretText") {
        let rejected: Vec<&str> = config
            .keys()
            .map(String::as_str)
            .filter(|k| *k != "secretText")
            .collect();
        return Err(BusinessError::validation(
            key,
            format!(
                "Unsupported config field(s) for telegram: {}",
                rejected.join(", ")
            ),
        ));
    }
    let token = parsed
        .get("secretText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match telegram_call(upstream, &token, "getMe", "GET", None).await {
        Ok(()) => {}
        // `providerFailureStatus`: a 5xx is retryable (502); a 4xx and a
        // transport error (`parseOriginError` falls back to 400) are a
        // rejection.
        Err(Some(status)) if status >= 500 => return Err(provider_unavailable()),
        Err(_) => return Err(credentials_rejected()),
    }
    let Some(bot_id) = telegram_bot_id(&token).map(str::to_string) else {
        return Err(credentials_rejected());
    };
    let auth = json!({ "authType": "secretText", "secretText": token }).to_string();
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let existing = find_by_source(tx, workspace_id, "telegram", &bot_id)?;
        if existing.as_ref().is_some_and(|row| is_active(&row.status)) {
            return Err(already_connected());
        }
        let revived_inbox = match existing.as_ref().and_then(|row| row.inbox_id) {
            Some(inbox_id) => {
                let updated = tx
                    .execute(
                        r#"UPDATE "IntegrationTelegram" SET "auth" = ?3, "updatedAt" = ?4
                           WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
                        params![inbox_id, workspace_id, auth, now],
                    )
                    .map_err(repo_err)?;
                (updated > 0).then_some(inbox_id)
            }
            None => None,
        };
        let inbox_id = match (revived_inbox, existing.as_ref().and_then(|row| row.inbox_id)) {
            (Some(inbox_id), _) => inbox_id,
            (None, reused) => {
                let inbox_id = match reused {
                    Some(inbox_id) => inbox_id,
                    None => {
                        let inbox_id = new_id();
                        tx.execute(
                            r#"INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name",
                                   "channel", "sourceId", "workspaceId", "status")
                               VALUES (?1, ?2, ?2, 'Telegram bot', 'telegram', ?3, ?4, 'connected')"#,
                            params![inbox_id, now, bot_id, workspace_id],
                        )
                        .map_err(repo_err)?;
                        inbox_id
                    }
                };
                tx.execute(
                    r#"INSERT INTO "IntegrationTelegram" ("id", "createdAt", "updatedAt", "auth",
                           "botId", "name", "workspaceId", "inboxId")
                       VALUES (?1, ?2, ?2, ?3, ?4, 'Telegram bot', ?5, ?6)"#,
                    params![new_id(), now, auth, bot_id, workspace_id, inbox_id],
                )
                .map_err(|error| unique_or(error, "IntegrationTelegram"))?;
                inbox_id
            }
        };
        // The inbox mirror of `connect.completed`.
        tx.execute(
            r#"UPDATE "Inbox" SET "status" = 'connected', "disconnectedAt" = NULL,
                   "disconnectReason" = NULL, "updatedAt" = ?3
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![inbox_id, workspace_id, now],
        )
        .map_err(repo_err)?;
        let id = match existing {
            Some(existing) => {
                tx.execute(
                    r#"UPDATE "Connection" SET "inboxId" = ?3, "displayName" = 'Telegram bot',
                           "lastError" = NULL, "status" = 'connected', "statusReason" = NULL,
                           "connectedAt" = ?4, "disconnectedAt" = NULL, "updatedAt" = ?4
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![existing.id, workspace_id, inbox_id, now],
                )
                .map_err(repo_err)?;
                existing.id
            }
            None => {
                let id = new_id();
                tx.execute(
                    r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId",
                           "provider", "kind", "channel", "inboxId", "sourceId", "displayName",
                           "status", "createdBy", "connectedAt")
                       VALUES (?1, ?2, ?2, ?3, 'telegram', 'channel', 'telegram', ?4, ?5,
                           'Telegram bot', 'connected', ?6, ?2)"#,
                    params![id, now, workspace_id, inbox_id, bot_id, actor_user_id],
                )
                .map_err(|error| unique_or(error, "Connection"))?;
                id
            }
        };
        find_or_fail(tx, id, workspace_id)
    })
    .await
}

/// The stored bot token of a telegram connection.
async fn telegram_token(db: &Db, current: &ConnectionRow) -> Result<Option<String>> {
    let inbox_id = current.inbox_id;
    let workspace_id = current.workspace_id;
    tx::read(db, move |conn| {
        let auth: Option<String> = conn
            .query_row(
                r#"SELECT "auth" FROM "IntegrationTelegram" WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
                params![inbox_id, workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        Ok(auth.and_then(|auth| {
            serde_json::from_str::<Value>(&auth)
                .ok()?
                .get("secretText")?
                .as_str()
                .map(str::to_string)
        }))
    })
    .await
}

/// `connectionService.verify` of a telegram connection (the caller checked
/// that it is active): `getMe`; a 401 is `token_revoked` (needs_reauth),
/// another failure `verify_failed` (degraded).
pub async fn verify_telegram(
    db: &Db,
    upstream: &impl Upstream,
    current: ConnectionRow,
) -> Result<ConnectionRow> {
    let token = telegram_token(db, &current).await?.ok_or_else(|| {
        BusinessError::Repo(repo::Error::msg(
            "Unable to load auth for IntegrationTelegram",
        ))
    })?;
    let (status, reason) = match telegram_call(upstream, &token, "getMe", "GET", None).await {
        Ok(()) => ("connected", None),
        Err(Some(401)) => ("needs_reauth", Some("token_revoked")),
        Err(_) => ("degraded", Some("verify_failed")),
    };
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "Connection" SET "status" = ?3, "statusReason" = ?4, "updatedAt" = ?5
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![current.id, current.workspace_id, status, reason, now],
        )
        .map_err(repo_err)?;
        find_or_fail(tx, current.id, current.workspace_id)
    })
    .await
}

/// `connectionService.disconnect` of a telegram connection: `deleteWebhook`
/// (a failure is kept as `lastError`), then `user.disconnect` with the inbox
/// mirror; the satellite row is kept (`onDisconnect: "keep_row"`).
pub async fn disconnect_telegram(
    db: &Db,
    upstream: &impl Upstream,
    current: ConnectionRow,
) -> Result<ConnectionRow> {
    let mut teardown_error = None;
    match telegram_token(db, &current).await {
        Ok(Some(token)) => {
            let body = json!({ "drop_pending_updates": false });
            if telegram_call(upstream, &token, "deleteWebhook", "POST", Some(body))
                .await
                .is_err()
            {
                teardown_error = Some("Provider-side teardown failed");
            }
        }
        Ok(None) => teardown_error = Some("Provider authentication was unavailable for teardown"),
        Err(_) => teardown_error = Some("Provider-side teardown failed"),
    }
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        let updated = super::mark_disconnected(tx, &current, now)?;
        if let Some(error) = teardown_error {
            tx.execute(
                r#"UPDATE "Connection" SET "lastError" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![current.id, current.workspace_id, error],
            )
            .map_err(repo_err)?;
            return find_or_fail(tx, current.id, current.workspace_id);
        }
        Ok(updated)
    })
    .await
}
