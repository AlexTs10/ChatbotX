//! `apps/builder/src/features/integrations/api/public/crud.ts`
//! (`integrations.list / get / tokenErrors`) and `public/ai.ts`
//! (`integrationsAiPublicRouter`: `getAiProvider`, `connectAiProvider`,
//! `updateAiProvider`, `disconnectAiProvider`). Connect and disconnect run
//! through the connection engine (`connectFromCredentials` with
//! `allowUpdate`, `connectionService.disconnect`). No AI integration cache
//! in the Rust worker: nothing to invalidate.

use chatbotx_business::BusinessError;
use chatbotx_business::connection::credentials::{self, HttpUpstream};
use chatbotx_business::integration::{self, IntegrationRow};
use chatbotx_business::integration_ai_provider::{self as service, AiProviderRow};
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Str, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json, no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "integrations.list",
    "integrations.get",
    "integrations.tokenErrors",
    "integrations.getAiProvider",
    "integrations.connectAiProvider",
    "integrations.updateAiProvider",
    "integrations.disconnectAiProvider",
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

/// `publicIntegrationResource`.
pub fn integration_json(row: &IntegrationRow) -> Value {
    json!({
        "id": id(row.id),
        "integrationType": row.integration_type,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
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
    match route_id {
        "integrations.list" | "integrations.get" | "integrations.tokenErrors" => {
            crud(state, route_id, call).await
        }
        _ => ai(state, route_id, call).await,
    }
}

async fn crud(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = Input::from_query(call.params, call.query);
    match route_id {
        "integrations.list" => {
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let rows = integration::list_by_workspace(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            // `paginateInMemory`.
            let total = rows.len() as i64;
            let page_count = ((total + per_page - 1) / per_page).max(1);
            let data: Vec<Value> = rows
                .iter()
                .skip(((page - 1) * per_page) as usize)
                .take(per_page as usize)
                .map(integration_json)
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "integrations.get" => {
            let text = input.string("id", Str::default()).unwrap_or_default();
            input.finish()?;
            let row_id = text.parse::<i64>().unwrap_or(-1);
            integration::find_for_workspace(db, row_id, ws)
                .await
                .map_err(|e| business_error(e, FINDING))?
                .map(|row| json(200, integration_json(&row)))
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Integration not found"), FINDING)
                })
        }
        _ => {
            input.finish()?;
            let rows = integration::token_refresh_errors(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows
                .iter()
                .map(|row| {
                    json!({ "id": id(row.id), "channel": row.channel, "name": row.name, "error": row.error })
                })
                .collect();
            Ok(json(200, json!({ "data": data })))
        }
    }
}

async fn ai(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let body = matches!(
        route_id,
        "integrations.updateAiProvider" | "integrations.connectAiProvider"
    );
    let mut input = if body {
        Input::from_body(call.params, call.body)
    } else {
        Input::from_query(call.params, call.query)
    };
    let value = input.get("provider").cloned();
    let provider = input.enum_at(&key("provider"), value.as_ref(), PROVIDERS, true);
    let auto_reply = if route_id == "integrations.updateAiProvider" {
        input.opt_coerced_bool("autoReply")
    } else {
        None
    };
    let config = (route_id == "integrations.connectAiProvider").then(|| connect_config(&mut input));
    input.finish()?;
    let provider = provider.unwrap_or_default();
    match route_id {
        "integrations.connectAiProvider" => {
            let config = config.flatten().unwrap_or_default();
            let ai_key = credentials::ai_key_provider(&provider)
                .ok_or_else(|| super::internal_error("AI key provider"))?;
            credentials::connect_ai_key_with(db, &HttpUpstream, ws, ai_key, &config, None, true)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let row = service::find(db, &provider, ws)
                .await
                .map_err(|e| business_error(e, MUTATING))?
                .ok_or_else(|| not_found(&provider, MUTATING))?;
            return Ok(json(200, ai_provider_json(&row)));
        }
        "integrations.disconnectAiProvider" => {
            let connection = credentials::find_workspace_connection(db, ws, &provider)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            match connection {
                Some(row) => {
                    credentials::disconnect_integration(db, row)
                        .await
                        .map_err(|e| business_error(e, MUTATING))?;
                }
                None => service::disconnect_legacy(db, &state.audit, &provider, ws)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?,
            }
            return Ok(no_content());
        }
        _ => {}
    }
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

/// `connectAiProviderRequest` as the `config` of `connectFromCredentials`.
fn connect_config(input: &mut Input) -> Option<Map<String, Value>> {
    let api_key = input.string(
        "apiKey",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let model = input.string(
        "model",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let value = input.get("temperature").cloned();
    let temperature = input.coerce_number_at(
        &key("temperature"),
        value.as_ref(),
        Num::range(0.0, 2.0),
        true,
    );
    let value = input.get("maxOutputTokens").cloned();
    let max_output_tokens = input.coerce_number_at(
        &key("maxOutputTokens"),
        value.as_ref(),
        Num::int(1.0, 8192.0),
        true,
    );
    let mut config = Map::new();
    config.insert("apiKey".into(), json!(api_key?));
    config.insert("model".into(), json!(model?));
    config.insert("temperature".into(), json!(temperature?));
    config.insert("maxOutputTokens".into(), json!(max_output_tokens? as i64));
    Some(config)
}
