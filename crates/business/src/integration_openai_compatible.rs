//! `packages/business/src/integration-openai-compatible/` (service and
//! `validate-base-url.ts`), the presets of
//! `packages/ai/src/openai-compatible/presets.ts` (default models) and the
//! builder probe `verifyOpenaiCompatibleProvider`.

use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::repo::{Json, new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::net::ssrf_guard::{DohFetch, assert_public_url_with};
use crate::tx;

const MAX_BASE_URL_LENGTH: usize = 2048;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// `openaiCompatibleProviderPresets` and each preset's `defaultModel`.
pub const PRESETS: &[(&str, &str)] = &[
    ("custom", "gpt-4o-mini"),
    ("nim", "nvidia/llama-3.3-nemotron-super-49b-v1.5"),
    ("lmstudio", "local-model"),
    ("heroku", "claude-3-5-haiku"),
    (
        "clarifai",
        "https://clarifai.com/openai/chat-completion/models/gpt-oss-120b",
    ),
    ("nearai", "zai-org/GLM-5.1-FP8"),
];

/// `resolveOpenaiCompatibleDefaultModel`.
pub fn default_model_of(preset: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(p, _)| *p == preset).map(|(_, m)| *m)
}

fn invalid_base_url() -> BusinessError {
    BusinessError::exception(
        "OpenAI-compatible base URL is invalid.",
        "invalidBaseUrl",
        400,
    )
}

/// `normalizeOpenaiCompatibleBaseUrl`: trimmed, http(s), at most 2048
/// characters, WHATWG-serialized.
pub fn normalize_base_url(base_url: &str) -> Result<String> {
    let trimmed = chatbotx_core::js::trim(base_url);
    if trimmed.is_empty() || chatbotx_core::js::utf16_len(trimmed) > MAX_BASE_URL_LENGTH {
        return Err(invalid_base_url());
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| invalid_base_url())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(invalid_base_url());
    }
    Ok(parsed.to_string())
}

/// `validateOpenaiCompatibleBaseUrlForEnvironment`: in the cloud edition
/// the URL must be public (`ssrfBlocked`).
pub async fn validate_base_url_for_environment(
    base_url: &str,
    cloud: bool,
    doh: &impl DohFetch,
) -> Result<String> {
    let normalized = normalize_base_url(base_url)?;
    if !cloud {
        return Ok(normalized);
    }
    assert_public_url_with(&normalized, "OpenAI-compatible base URL", doh)
        .await
        .map_err(|_| {
            BusinessError::exception(
                "OpenAI-compatible base URL is not allowed.",
                "ssrfBlocked",
                400,
            )
        })?;
    Ok(normalized)
}

/// `verifyOpenaiCompatibleProvider` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCheck {
    Ok,
    /// Ok with `warning: "could_not_verify"`.
    CouldNotVerify,
    Unauthorized(u16),
    UnsafeBaseUrl,
}

/// The verdict of a `GET <baseURL>/models` answer (`None` = no answer).
pub fn classify_models_probe(status: Option<u16>) -> ProviderCheck {
    match status {
        Some(status) if (200..300).contains(&status) => ProviderCheck::Ok,
        Some(status @ (401 | 403)) => ProviderCheck::Unauthorized(status),
        _ => ProviderCheck::CouldNotVerify,
    }
}

/// `<baseURL>/models` (one trailing slash of the path dropped first).
pub fn models_url(normalized: &str) -> Result<String> {
    let mut url = url::Url::parse(normalized).map_err(|_| invalid_base_url())?;
    let path = url
        .path()
        .strip_suffix('/')
        .unwrap_or(url.path())
        .to_string();
    url.set_path(&format!("{path}/models"));
    Ok(url.to_string())
}

/// `verifyOpenaiCompatibleProvider`.
pub async fn verify_provider(
    api_key: Option<&str>,
    base_url: &str,
    cloud: bool,
    doh: &impl DohFetch,
) -> Result<ProviderCheck> {
    let normalized = match validate_base_url_for_environment(base_url, cloud, doh).await {
        Ok(url) => url,
        Err(error) if matches!(error.code(), Some("invalidBaseUrl" | "ssrfBlocked")) => {
            return Ok(ProviderCheck::UnsafeBaseUrl);
        }
        Err(error) => return Err(error),
    };
    let url = models_url(&normalized)?;
    let Ok(client) = reqwest::Client::builder().timeout(VERIFY_TIMEOUT).build() else {
        return Ok(ProviderCheck::CouldNotVerify);
    };
    let mut request = client.get(url);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    let status = request.send().await.ok().map(|r| r.status().as_u16());
    Ok(classify_models_probe(status))
}

/// An `IntegrationOpenaiCompatible` row (no secret).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenaiCompatibleRow {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub integration_id: i64,
    pub name: String,
    pub preset: String,
    pub base_url: String,
    pub default_model: String,
    pub auto_reply: bool,
    pub enabled: bool,
    pub auth: Option<Value>,
}

const COLUMNS: &str = r#""id", "createdAt", "updatedAt", "integrationId", "name", "preset", "baseURL", "defaultModel", "autoReply", "enabled", "auth""#;

fn row(r: &chatbotx_db::rusqlite::Row<'_>) -> chatbotx_db::rusqlite::Result<OpenaiCompatibleRow> {
    let auth: Option<String> = r.get(10)?;
    Ok(OpenaiCompatibleRow {
        id: r.get(0)?,
        created_at: r.get(1)?,
        updated_at: r.get(2)?,
        integration_id: r.get(3)?,
        name: r.get(4)?,
        preset: r.get(5)?,
        base_url: r.get(6)?,
        default_model: r.get(7)?,
        auto_reply: r.get(8)?,
        enabled: r.get(9)?,
        auth: auth.and_then(|a| serde_json::from_str(&a).ok()),
    })
}

fn db_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `listByWorkspaceId` (oldest first).
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<OpenaiCompatibleRow>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "IntegrationOpenaiCompatible" WHERE "workspaceId" = ?1 ORDER BY "createdAt" ASC, "id" ASC"#
            ))
            .map_err(db_err)?;
        let rows = stmt
            .query_map([workspace_id], row)
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    })
    .await
}

/// `findByWorkspaceIdAndId`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<OpenaiCompatibleRow>> {
    tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT {COLUMNS} FROM "IntegrationOpenaiCompatible" WHERE "workspaceId" = ?1 AND "id" = ?2"#
            ),
            params![workspace_id, id],
            row,
        )
        .optional()
        .map_err(db_err)
    })
    .await
}

/// `OpenaiCompatiblePresetAlreadyConnectedError`.
pub fn preset_already_connected() -> BusinessError {
    BusinessError::exception(
        "OpenAI-compatible preset is already connected",
        "openaiCompatiblePresetAlreadyConnected",
        409,
    )
}

fn create_auth(api_key: Option<&str>) -> Option<Value> {
    let secret = chatbotx_core::js::trim(api_key?);
    (!secret.is_empty()).then(|| json!({ "authType": "secretText", "secretText": secret }))
}

fn preset_taken(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    preset: Option<&str>,
    exclude_id: Option<i64>,
) -> Result<bool> {
    let Some(preset) = preset.filter(|p| *p != "custom") else {
        return Ok(false);
    };
    let found: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "IntegrationOpenaiCompatible" WHERE "workspaceId" = ?1 AND "preset" = ?2 AND "id" IS NOT ?3 LIMIT 1"#,
            params![workspace_id, preset, exclude_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(found.is_some())
}

/// `ConnectOpenaiCompatibleInput`.
#[derive(Debug, Clone, Default)]
pub struct ConnectInput {
    pub name: String,
    pub preset: String,
    pub base_url: String,
    pub default_model: String,
    pub api_key: String,
    pub auto_reply: Option<bool>,
    pub enabled: Option<bool>,
}

/// `connect` (the legacy service path; the builder action connects through
/// the connection engine).
pub async fn connect(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    input: ConnectInput,
    cloud: bool,
    doh: &impl DohFetch,
) -> Result<i64> {
    let auth = create_auth(Some(&input.api_key));
    let base_url = validate_base_url_for_environment(&input.base_url, cloud, doh).await?;
    let id = tx::write(db, move |conn| {
        if preset_taken(conn, workspace_id, Some(&input.preset), None)? {
            return Err(preset_already_connected());
        }
        let integration_id = new_id();
        conn.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, 'openaiCompatible')"#,
            params![integration_id, workspace_id],
        )
        .map_err(db_err)?;
        let id = new_id();
        conn.execute(
            r#"INSERT INTO "IntegrationOpenaiCompatible" ("id", "auth", "autoReply", "baseURL", "defaultModel", "enabled",
                 "integrationId", "name", "preset", "workspaceId") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                id,
                auth.map(Json),
                input.auto_reply.unwrap_or(false),
                base_url,
                input.default_model,
                input.enabled.unwrap_or(true),
                integration_id,
                input.name,
                input.preset,
                workspace_id,
            ],
        )
        .map_err(db_err)?;
        Ok(id)
    })
    .await?;
    audit
        .audit("connect", "connected a new OpenAI-compatible integration")
        .await;
    Ok(id)
}

/// `UpdateOpenaiCompatibleInput` (`None` = not given).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateInput {
    pub name: Option<String>,
    pub preset: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub api_key: Option<String>,
    pub auto_reply: Option<bool>,
    pub enabled: Option<bool>,
}

/// `update`: `OpenAI-compatible integration not found` (500) for an unknown
/// id; a changed base URL is validated for the environment; audits.
pub async fn update(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    data: UpdateInput,
    cloud: bool,
    doh: &impl DohFetch,
) -> Result<()> {
    let existing = find(db, workspace_id, id).await?.ok_or_else(|| {
        BusinessError::exception(
            "OpenAI-compatible integration not found",
            "systemError",
            500,
        )
    })?;
    let base_url = match data.base_url.as_deref() {
        None => None,
        Some(raw) => {
            let normalized = normalize_base_url(raw)?;
            Some(if normalized == existing.base_url {
                normalized
            } else {
                validate_base_url_for_environment(&normalized, cloud, doh).await?
            })
        }
    };
    let auth = data.api_key.as_deref().and_then(|k| create_auth(Some(k)));
    tx::write(db, move |conn| {
        if preset_taken(conn, workspace_id, data.preset.as_deref(), Some(id))? {
            return Err(preset_already_connected());
        }
        conn.execute(
            r#"UPDATE "IntegrationOpenaiCompatible" SET
                 "name" = coalesce(?1, "name"), "preset" = coalesce(?2, "preset"),
                 "baseURL" = coalesce(?3, "baseURL"), "defaultModel" = coalesce(?4, "defaultModel"),
                 "auth" = coalesce(?5, "auth"), "autoReply" = coalesce(?6, "autoReply"),
                 "enabled" = coalesce(?7, "enabled"), "updatedAt" = ?8
               WHERE "id" = ?9 AND "workspaceId" = ?10"#,
            params![
                data.name,
                data.preset,
                base_url,
                data.default_model,
                auth.map(Json),
                data.auto_reply,
                data.enabled,
                now_ms(),
                id,
                workspace_id,
            ],
        )
        .map_err(db_err)?;
        Ok(())
    })
    .await?;
    audit
        .audit(
            "update",
            "updated the OpenAI-compatible integration configuration",
        )
        .await;
    Ok(())
}

/// `disconnect` (legacy path): deletes the `Integration` row (cascades).
pub async fn disconnect(db: &Db, audit: &AuditService, workspace_id: i64, id: i64) -> Result<()> {
    let Some(existing) = find(db, workspace_id, id).await? else {
        return Ok(());
    };
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "Integration" WHERE "id" = ?1"#,
            [existing.integration_id],
        )
        .map_err(db_err)?;
        Ok(())
    })
    .await?;
    audit
        .audit(
            "disconnect",
            "disconnected the OpenAI-compatible integration",
        )
        .await;
    Ok(())
}
