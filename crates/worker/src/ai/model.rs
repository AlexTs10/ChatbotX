//! `packages/ai/src/server/factory.ts` + `openai-compatible.ts`: the AI
//! integration rows of a workspace as [`Model`]s.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::Value;

use super::llm::{Model, Transform, Wire};

/// `aiProviders`: `(provider, table, wire, base URL)`.
const PROVIDERS: [(&str, &str, Wire, &str); 6] = [
    (
        "openai",
        "IntegrationOpenai",
        Wire::OpenAi,
        "https://api.openai.com/v1",
    ),
    (
        "gemini",
        "IntegrationGemini",
        Wire::Gemini,
        "https://generativelanguage.googleapis.com/v1beta",
    ),
    (
        "claude",
        "IntegrationClaude",
        Wire::Anthropic,
        "https://api.anthropic.com/v1",
    ),
    (
        "deepseek",
        "IntegrationDeepseek",
        Wire::OpenAi,
        "https://api.deepseek.com",
    ),
    (
        "openrouter",
        "IntegrationOpenrouter",
        Wire::OpenAi,
        "https://openrouter.ai/api/v1",
    ),
    // `MUSE_BASE_URL`.
    (
        "muse",
        "IntegrationMuse",
        Wire::OpenAi,
        "https://api.meta.ai/v1",
    ),
];

/// `aiProviders.options`, in order.
pub fn providers() -> impl Iterator<Item = &'static str> {
    PROVIDERS.iter().map(|p| p.0)
}

/// `defaultAIModels`.
pub fn default_model(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "openai" => "gpt-5.4-mini",
        "gemini" => "gemini-3.5-flash",
        "claude" => "claude-sonnet-4-6",
        "deepseek" => "deepseek-flash",
        "openrouter" => "openai/gpt-5.4-mini",
        "muse" => "muse-spark-1.3",
        _ => return None,
    })
}

/// `normalizeAIModelId`: the legacy model ids mapped to current ones; the
/// DeepSeek legacy chat / reasoner ids throw.
pub fn normalize_model_id(provider: &str, model_id: &str) -> Result<String, String> {
    if provider == "deepseek" && matches!(model_id, "deepseek-chat" | "deepseek-reasoner") {
        return Err(format!(
            "DeepSeek legacy model {model_id} requires a mode-aware migration"
        ));
    }
    let mapped = match (provider, model_id) {
        ("deepseek", "deepseek-v4-flash") => "deepseek-flash",
        ("claude", "claude-3-5-sonnet-20241022" | "claude-3-sonnet-20240229") => {
            "claude-sonnet-4-6"
        }
        (
            "claude",
            "claude-3-5-haiku-20241022" | "claude-3-haiku-20240307" | "claude-4.5-haiku-20251001",
        ) => "claude-haiku-4-5-20251001",
        ("claude", "claude-3-opus-20240229") => "claude-opus-4-8",
        ("claude", "claude-opus-4.6") => "claude-opus-4-6",
        ("claude", "claude-4.5-sonnet-20250929" | "claude-sonnet-4.5-20250929") => {
            "claude-sonnet-4-5-20250929"
        }
        ("claude", "claude-4.5-opus-20251101") => "claude-opus-4-5-20251101",
        (
            "gemini",
            "gemini-3-flash" | "gemini-2.5-flash-lite" | "gemini-2.5-flash" | "gemini-2.5-pro",
        ) => "gemini-3.5-flash",
        ("gemini", "gemini-3.1-flash-image-preview") => "gemini-3.1-flash-image",
        ("gemini", "gemini-3-pro-image-preview") => "gemini-3-pro-image",
        ("openrouter", "anthropic/claude-3-5-sonnet") => "anthropic/claude-sonnet-4.5",
        ("openrouter", "anthropic/claude-3-5-haiku") => "anthropic/claude-haiku-4.5",
        ("openrouter", "google/gemini-2.5-pro-preview") => "google/gemini-2.5-pro",
        _ => model_id,
    };
    Ok(mapped.to_owned())
}

/// `secretTextAuthSchema.safeParse(auth)`.
pub(crate) fn secret_text(auth: &Value) -> Option<String> {
    let auth = match auth {
        Value::String(text) => serde_json::from_str(text).ok()?,
        other => other.clone(),
    };
    if auth.get("authType")?.as_str()? != "secretText" {
        return None;
    }
    let key = auth.get("secretText")?.as_str()?;
    (!key.is_empty()).then(|| key.to_owned())
}

/// The `auth` of the provider's integration row (`aiIntegrationService.findBy`;
/// `auto_reply` filters on the flag as `getAIIntegrationInDB`).
pub fn native_auth(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    auto_reply: Option<bool>,
) -> chatbotx_db::rusqlite::Result<Option<Value>> {
    let Some((_, table, ..)) = PROVIDERS.iter().find(|p| p.0 == provider) else {
        return Ok(None);
    };
    let filter = match auto_reply {
        Some(true) => r#" AND "autoReply" = 1"#,
        Some(false) => r#" AND "autoReply" = 0"#,
        None => "",
    };
    conn.query_row(
        &format!(
            r#"SELECT "auth" FROM "{table}" WHERE "workspaceId" = ?1{filter} ORDER BY "id" LIMIT 1"#
        ),
        params![workspace_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|auth| {
        auth.map(|a| {
            a.and_then(|a| serde_json::from_str(&a).ok())
                .unwrap_or(Value::Null)
        })
    })
}

/// `createAIModelInstance`: an invalid auth or model id throws.
pub fn native_model(provider: &str, auth: &Value, model_id: &str) -> Result<Model, String> {
    let api_key =
        secret_text(auth).ok_or_else(|| "Invalid AI integration auth configuration".to_owned())?;
    let Some((_, _, wire, base_url)) = PROVIDERS.iter().find(|p| p.0 == provider) else {
        return Err(format!("Unsupported provider: {provider}"));
    };
    Ok(Model {
        provider: provider.to_owned(),
        wire: *wire,
        base_url: (*base_url).to_owned(),
        api_key: Some(api_key),
        model_id: normalize_model_id(provider, model_id)?,
        transform: if provider == "muse" {
            Transform::Muse
        } else {
            Transform::None
        },
    })
}

/// An `IntegrationOpenaiCompatible` row: `(enabled, baseURL, preset, auth,
/// defaultModel)`.
pub struct CompatibleRow {
    pub enabled: bool,
    pub base_url: String,
    pub preset: String,
    pub auth: Option<Value>,
    pub default_model: String,
}

/// `integrationOpenaiCompatibleService.findByWorkspaceIdAndId` (or, with
/// `id` `None`, the oldest enabled auto-reply row).
pub fn compatible_row(
    conn: &Connection,
    workspace_id: i64,
    id: Option<i64>,
    auto_reply: Option<bool>,
) -> chatbotx_db::rusqlite::Result<Option<CompatibleRow>> {
    let mut sql = String::from(
        r#"SELECT "enabled", "baseURL", "preset", "auth", "defaultModel" FROM "IntegrationOpenaiCompatible" WHERE "workspaceId" = ?1"#,
    );
    match id {
        Some(id) => sql.push_str(&format!(r#" AND "id" = {id}"#)),
        None => sql.push_str(r#" AND "enabled" = 1"#),
    }
    match auto_reply {
        Some(true) => sql.push_str(r#" AND "autoReply" = 1"#),
        Some(false) => sql.push_str(r#" AND "autoReply" = 0"#),
        None => {}
    }
    sql.push_str(r#" ORDER BY "createdAt", "id" LIMIT 1"#);
    conn.query_row(&sql, params![workspace_id], |r| {
        Ok(CompatibleRow {
            enabled: r.get(0)?,
            base_url: r.get(1)?,
            preset: r.get(2)?,
            auth: r
                .get::<_, Option<String>>(3)?
                .and_then(|a| serde_json::from_str(&a).ok()),
            default_model: r.get(4)?,
        })
    })
    .optional()
}

/// `createOpenaiCompatibleModelInstance`: a stored auth that is not a secret
/// text throws.
pub fn compatible_model(row: &CompatibleRow, model_id: &str) -> Result<Model, String> {
    let api_key = match &row.auth {
        None | Some(Value::Null) => None,
        Some(auth) => Some(
            secret_text(auth)
                .ok_or_else(|| "OpenAI-compatible provider auth config is invalid.".to_owned())?,
        ),
    };
    Ok(Model {
        provider: "openaiCompatible".into(),
        wire: Wire::OpenAi,
        base_url: row.base_url.clone(),
        api_key,
        model_id: model_id.to_owned(),
        transform: if row.preset == "nearai" {
            Transform::StripReasoning
        } else {
            Transform::None
        },
    })
}

/// Why [`resolve_flow_model`] has no model.
#[derive(Debug, Clone, PartialEq)]
pub enum NoModel {
    /// A missing / disabled integration (`{ ok: false, message }`).
    Missing(String),
    /// A throw (bad auth, legacy model id, DB error).
    Failed(String),
}

/// `resolveFlowAIModel`.
pub async fn resolve_flow_model(
    db: &Db,
    workspace_id: i64,
    provider: &str,
    integration_id: Option<&str>,
    model_id: &str,
) -> Result<Model, NoModel> {
    let (provider, model_id) = (provider.to_owned(), model_id.to_owned());
    let integration_id = integration_id.map(str::to_owned);
    db.read(move |conn| {
        if provider == "openaiCompatible" {
            let Some(id) = integration_id.as_deref().filter(|s| !s.is_empty()) else {
                return Ok(Err(NoModel::Missing(
                    "OpenAI-compatible integration is not configured".into(),
                )));
            };
            let row = match id.parse::<i64>() {
                Ok(id) => compatible_row(conn, workspace_id, Some(id), None)?,
                Err(_) => None,
            };
            return Ok(match row.filter(|r| r.enabled) {
                None => Err(NoModel::Missing(
                    "OpenAI-compatible integration is missing or disabled".into(),
                )),
                Some(row) => compatible_model(&row, &model_id).map_err(NoModel::Failed),
            });
        }
        Ok(match native_auth(conn, workspace_id, &provider, None)? {
            None => Err(NoModel::Missing(format!(
                "AI integration not found for provider {provider}"
            ))),
            Some(auth) => native_model(&provider, &auth, &model_id).map_err(NoModel::Failed),
        })
    })
    .await
    .unwrap_or_else(|e| Err(NoModel::Failed(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_model_ids_are_mapped() {
        assert_eq!(
            normalize_model_id("claude", "claude-3-opus-20240229").unwrap(),
            "claude-opus-4-8"
        );
        assert_eq!(normalize_model_id("openai", "gpt-4o").unwrap(), "gpt-4o");
        assert!(normalize_model_id("deepseek", "deepseek-chat").is_err());
    }

    #[test]
    fn invalid_auth_throws() {
        assert_eq!(
            native_model("openai", &serde_json::json!({}), "gpt-4o").unwrap_err(),
            "Invalid AI integration auth configuration"
        );
        let model = native_model(
            "muse",
            &serde_json::json!({ "authType": "secretText", "secretText": "k" }),
            "muse-spark-1.3",
        )
        .unwrap();
        assert_eq!(model.transform, Transform::Muse);
        assert_eq!(model.base_url, "https://api.meta.ai/v1");
    }
}
