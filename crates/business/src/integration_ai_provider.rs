//! The AI key providers (claude, deepseek, gemini, muse, openai,
//! openrouter): `integration-ai-provider/verify.ts` (the live API key
//! probe) and the per-provider services' `findByWorkspaceId` / `update`
//! (`packages/business/src/integration-<provider>/service.ts`).
//!
//! Connect and disconnect run through the connection engine
//! (`connectFromCredentials`, `connectionService.disconnect`), which is
//! P3.6-D2 work; [`verify_ai_provider_api_key`] is the probe its AI key
//! `ConnectionProvider` (`credential-providers.ts`) calls.

use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};

use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// `AiKeyValidation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiKeyValidation {
    Valid,
    Invalid,
    Unknown,
}

/// The probe URL, its headers and the statuses that reject the key.
pub type VerifyRequest = (&'static str, Vec<(&'static str, String)>, &'static [u16]);

/// `verifyConfigByProvider`: the probe URL, its headers and the statuses
/// that mean "this key is rejected".
pub fn verify_request(provider: &str, api_key: &str) -> Option<VerifyRequest> {
    const UNAUTHORIZED: &[u16] = &[401, 403];
    let bearer = || vec![("Authorization", format!("Bearer {api_key}"))];
    Some(match provider {
        "claude" => (
            "https://api.anthropic.com/v1/models",
            vec![
                ("x-api-key", api_key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
            UNAUTHORIZED,
        ),
        "deepseek" => ("https://api.deepseek.com/models", bearer(), UNAUTHORIZED),
        "gemini" => (
            "https://generativelanguage.googleapis.com/v1beta/models",
            vec![("x-goog-api-key", api_key.to_string())],
            &[400, 401, 403],
        ),
        "muse" => ("https://api.meta.ai/v1/models", bearer(), UNAUTHORIZED),
        "openai" => ("https://api.openai.com/v1/models", bearer(), UNAUTHORIZED),
        "openrouter" => ("https://openrouter.ai/api/v1/key", bearer(), UNAUTHORIZED),
        _ => return None,
    })
}

/// The verdict of a probe answer (`None` = no HTTP answer at all).
pub fn classify(invalid_statuses: &[u16], status: Option<u16>) -> AiKeyValidation {
    match status {
        Some(status) if (200..300).contains(&status) => AiKeyValidation::Valid,
        Some(status) if invalid_statuses.contains(&status) => AiKeyValidation::Invalid,
        _ => AiKeyValidation::Unknown,
    }
}

/// `verifyAiProviderApiKey`: a GET of the provider's list-models endpoint.
pub async fn verify_ai_provider_api_key(provider: &str, api_key: &str) -> AiKeyValidation {
    let Some((url, headers, invalid)) = verify_request(provider, api_key) else {
        return AiKeyValidation::Unknown;
    };
    let client = match reqwest::Client::builder().timeout(VERIFY_TIMEOUT).build() {
        Ok(client) => client,
        Err(_) => return AiKeyValidation::Unknown,
    };
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let status = request.send().await.ok().map(|r| r.status().as_u16());
    let verdict = classify(invalid, status);
    if verdict == AiKeyValidation::Unknown {
        tracing::warn!(
            provider,
            ?status,
            "AI provider API key verification was inconclusive"
        );
    }
    verdict
}

/// The satellite table and display name of an AI key provider.
pub fn provider_table(provider: &str) -> Option<(&'static str, &'static str)> {
    Some(match provider {
        "claude" => ("IntegrationClaude", "Claude"),
        "deepseek" => ("IntegrationDeepseek", "DeepSeek"),
        "gemini" => ("IntegrationGemini", "Gemini"),
        "muse" => ("IntegrationMuse", "Muse"),
        "openai" => ("IntegrationOpenai", "OpenAI"),
        "openrouter" => ("IntegrationOpenrouter", "OpenRouter"),
        _ => return None,
    })
}

/// The columns every AI key provider row has.
#[derive(Debug, Clone, PartialEq)]
pub struct AiProviderRow {
    pub id: i64,
    pub model: String,
    pub temperature: Option<f64>,
    pub max_output_tokens: i64,
    pub auto_reply: bool,
    pub has_api_key: bool,
}

fn unknown_provider(provider: &str) -> BusinessError {
    BusinessError::validation("provider", format!("Unknown AI provider: {provider}"))
}

/// `findByWorkspaceId`.
pub async fn find(db: &Db, provider: &str, workspace_id: i64) -> Result<Option<AiProviderRow>> {
    let (table, _) = provider_table(provider).ok_or_else(|| unknown_provider(provider))?;
    tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT "id", "model", "temperature", "maxOutputTokens", "autoReply", "auth" IS NOT NULL
                   FROM "{table}" WHERE "workspaceId" = ?1"#
            ),
            [workspace_id],
            |r| {
                Ok(AiProviderRow {
                    id: r.get(0)?,
                    model: r.get(1)?,
                    temperature: r.get(2)?,
                    max_output_tokens: r.get(3)?,
                    auto_reply: r.get(4)?,
                    has_api_key: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

/// `update(..., { autoReply })`: `Integration <Name> not found` (500) when
/// the provider is not connected. Audits `updated the <Name> integration
/// configuration` (OpenRouter: only when the value changed). An empty
/// change writes nothing (the original's drizzle `.set({})` throws).
pub async fn update_auto_reply(
    db: &Db,
    audit: &AuditService,
    provider: &str,
    workspace_id: i64,
    auto_reply: Option<bool>,
) -> Result<()> {
    let (table, name) = provider_table(provider).ok_or_else(|| unknown_provider(provider))?;
    let existing = find(db, provider, workspace_id).await?.ok_or_else(|| {
        BusinessError::exception(format!("Integration {name} not found"), "systemError", 500)
    })?;
    let Some(auto_reply) = auto_reply else {
        return Ok(());
    };
    tx::write(db, move |conn| {
        conn.execute(
            &format!(r#"UPDATE "{table}" SET "autoReply" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#),
            params![auto_reply, chatbotx_db::repo::now_ms(), existing.id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await?;
    if provider != "openrouter" || auto_reply != existing.auto_reply {
        audit
            .audit(
                "update",
                &format!("updated the {name} integration configuration"),
            )
            .await;
    }
    Ok(())
}

/// The per-provider `disconnect(workspaceId)` (a workspace without a
/// `Connection` row): deletes the parent `Integration` (the satellite
/// cascades) and audits `disconnected the <Name> integration`; a no-op
/// when nothing is connected.
pub async fn disconnect_legacy(
    db: &Db,
    audit: &AuditService,
    provider: &str,
    workspace_id: i64,
) -> Result<()> {
    let (table, name) = provider_table(provider).ok_or_else(|| unknown_provider(provider))?;
    let deleted = tx::write(db, move |conn| {
        conn.execute(
            &format!(
                r#"DELETE FROM "Integration" WHERE "id" =
                   (SELECT "integrationId" FROM "{table}" WHERE "workspaceId" = ?1)"#
            ),
            [workspace_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await?;
    if deleted > 0 {
        audit
            .audit(
                "disconnect",
                &format!("disconnected the {name} integration"),
            )
            .await;
    }
    Ok(())
}
