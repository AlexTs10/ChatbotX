//! `apps/builder/src/features/integrations/api/public/ai.ts`
//! (`integrationsAiPublicRouter`): `getAiProvider` and `updateAiProvider`.
//! `connectAiProvider` / `disconnectAiProvider` run through the connection
//! engine (P3.6-D2 credential connects) and are not ported here.

use chatbotx_business::BusinessError;
use chatbotx_business::integration_ai_provider::{self as service, AiProviderRow};
use serde_json::{Value, json};

use super::input::{Input, key};
use super::{ApiState, Call, FINDING, MUTATING, Reply, business_error, id, json};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "integrations.getAiProvider",
    "integrations.updateAiProvider",
];

/// `aiProviderPathParam`.
const PROVIDERS: &[&str] = &["claude", "deepseek", "gemini", "muse", "openai"];

/// `publicAiProviderResource` (never the stored `auth`).
pub fn ai_provider_json(row: &AiProviderRow) -> Value {
    json!({
        "id": id(row.id),
        "model": row.model,
        "temperature": row.temperature,
        "maxOutputTokens": row.max_output_tokens,
        "autoReply": row.auto_reply,
        "hasApiKey": row.has_api_key,
    })
}

fn not_found(provider: &str, errors: super::ErrorMap) -> crate::orpc::OrpcError {
    business_error(
        BusinessError::not_found(format!("{provider} integration not found")),
        errors,
    )
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = if route_id == "integrations.updateAiProvider" {
        Input::from_body(call.params, call.body)
    } else {
        Input::from_query(call.params, call.query)
    };
    let value = input.get("provider").cloned();
    let provider = input.enum_at(&key("provider"), value.as_ref(), PROVIDERS, true);
    let auto_reply = if route_id == "integrations.updateAiProvider" {
        input.opt_bool("autoReply")
    } else {
        None
    };
    input.finish()?;
    let provider = provider.unwrap_or_default();
    let errors = if route_id == "integrations.getAiProvider" {
        FINDING
    } else {
        MUTATING
    };
    let existing = service::find(db, &provider, ws)
        .await
        .map_err(|e| business_error(e, errors))?
        .ok_or_else(|| not_found(&provider, errors))?;
    if route_id == "integrations.updateAiProvider" && auto_reply.is_some() {
        service::update_auto_reply(db, &state.audit, &provider, ws, auto_reply)
            .await
            .map_err(|e| business_error(e, MUTATING))?;
        let row = service::find(db, &provider, ws)
            .await
            .map_err(|e| business_error(e, MUTATING))?
            .ok_or_else(|| not_found(&provider, MUTATING))?;
        return Ok(json(200, ai_provider_json(&row)));
    }
    Ok(json(200, ai_provider_json(&existing)))
}
