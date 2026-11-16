//! `apps/builder/src/features/ai-agents/api/public.ts` (`aiAgentsPublicRouter`)
//! and the request schemas of `schema/action.ts`.

use chatbotx_business::BusinessError;
use chatbotx_business::ai::models::chat_models;
use chatbotx_business::ai_agent::{self as service, AiAgentInput, ListAiAgents};
use chatbotx_db::repo::models::AiAgent;
use chatbotx_db::repo::utils::{PaginationInput, SortItem};
use serde_json::{Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "aiAgents.list",
    "aiAgents.create",
    "aiAgents.get",
    "aiAgents.update",
    "aiAgents.delete",
];

/// `MAX_WEB_SEARCH_AUTHORIZED_DOMAINS`.
const MAX_DOMAINS: usize = 20;
const MESSAGE_ROLES: &[&str] = &["user", "assistant", "system", "developer"];
/// The keys of `createPublicAIAgentRequest` (strict).
const PUBLIC_KEYS: &[&str] = &[
    "name",
    "prompt",
    "messages",
    "models",
    "temperature",
    "maxOutputTokens",
    "tools",
    "webSearchAuthorizedDomains",
    "isDefault",
    "isRichResponse",
];

/// `aiAgentResourceSchema`.
pub fn ai_agent_json(row: &AiAgent) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "workspaceId": id(row.workspace_id),
        "name": row.name,
        "prompt": row.prompt,
        "actionPrompt": row.action_prompt,
        "actionRules": row.action_rules.0,
        "messages": row.messages.0,
        "isDefault": row.is_default,
        "isRichResponse": row.is_rich_response,
        "tools": row.tools.0,
        "webSearchAuthorizedDomains": row.web_search_authorized_domains.0,
        "models": row.models.0,
        "temperature": row.temperature,
        "maxOutputTokens": row.max_output_tokens,
    })
}

/// `toPublicAIAgentResource`: without the private AI action fields.
pub fn public_ai_agent_json(row: &AiAgent) -> Value {
    let mut out = ai_agent_json(row);
    if let Value::Object(map) = &mut out {
        map.remove("actionPrompt");
        map.remove("actionRules");
    }
    out
}

/// `z.hostname()`.
fn is_hostname(text: &str) -> bool {
    let text = text.strip_suffix('.').unwrap_or(text);
    !text.is_empty()
        && text.len() <= 253
        && text.split('.').all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 63
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
                && bytes[0] != b'-'
                && bytes[bytes.len() - 1] != b'-'
        })
}

fn array_at<'a>(input: &mut Input, path: &[Seg], value: Option<&'a Value>) -> Option<&'a [Value]> {
    match value {
        Some(Value::Array(items)) => Some(items),
        None => {
            input.issue(
                path,
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(path, "invalid_type", "Invalid input: expected array");
            None
        }
    }
}

fn indexed(name: &str, index: usize) -> Path {
    vec![Seg::Key(name.into()), Seg::Index(index)]
}

fn messages_field(input: &mut Input, value: &Value) -> Option<Value> {
    let items = array_at(input, &key("messages"), Some(value))?;
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let at = indexed("messages", index);
        let role = input.enum_at(&child(&at, "role"), item.get("role"), MESSAGE_ROLES, true);
        let content = input.string_at(
            &child(&at, "content"),
            item.get("content"),
            Str::trimmed(1, 255),
            true,
        );
        match (role, content) {
            (Some(role), Some(content)) => out.push(json!({ "role": role, "content": content })),
            _ => ok = false,
        }
    }
    ok.then_some(Value::Array(out))
}

fn model_entry(item: &Value) -> Option<Value> {
    if item.get("kind").and_then(Value::as_str) == Some("openaiCompatible") {
        let text = |name: &str| {
            item.get(name)
                .and_then(Value::as_str)
                .map(|t| chatbotx_core::js::trim(t).to_string())
                .filter(|t| !t.is_empty())
        };
        return Some(json!({
            "kind": "openaiCompatible",
            "integrationId": text("integrationId")?,
            "model": text("model")?,
        }));
    }
    let provider = item.get("provider")?.as_str()?;
    let model = item.get("model")?.as_str()?;
    chat_models(provider)?
        .contains(&model)
        .then(|| json!({ "provider": provider, "model": model }))
}

fn models_field(input: &mut Input, value: &Value) -> Option<Value> {
    let items = array_at(input, &key("models"), Some(value))?;
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        match model_entry(item) {
            Some(entry) => out.push(entry),
            None => {
                input.issue(&indexed("models", index), "invalid_union", "Invalid input");
                ok = false;
            }
        }
    }
    ok.then_some(Value::Array(out))
}

fn tools_field(input: &mut Input, value: &Value) -> Option<Vec<String>> {
    let items = array_at(input, &key("tools"), Some(value))?;
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        match input.string_at(&indexed("tools", index), Some(item), Str::default(), true) {
            Some(text) => out.push(text),
            None => ok = false,
        }
    }
    ok.then_some(out)
}

fn domains_field(input: &mut Input, value: &Value) -> Option<Vec<String>> {
    let name = "webSearchAuthorizedDomains";
    let items = array_at(input, &key(name), Some(value))?;
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let at = child(&indexed(name, index), "value");
        let Some(text) = input.string_at(&at, item.get("value"), Str::trimmed(0, usize::MAX), true)
        else {
            ok = false;
            continue;
        };
        if !is_hostname(&text) {
            input.issue(&at, "invalid_format", "Invalid hostname");
            ok = false;
            continue;
        }
        out.push(text);
    }
    if items.len() > MAX_DOMAINS {
        input.issue(
            &key(name),
            "too_big",
            &format!("Too big: expected array to have <={MAX_DOMAINS} items"),
        );
        ok = false;
    }
    ok.then_some(out)
}

/// `createAIAgentRequest` (`partial`: `updateAIAgentRequest`; `public`: the
/// public variant without `actionPrompt` / `actionRules`).
pub fn parse_ai_agent_input(input: &mut Input, partial: bool, public: bool) -> AiAgentInput {
    // A required key is checked when sent, or always on create.
    let wanted = |input: &Input, name: &str| !partial || input.has(name);
    let mut data = AiAgentInput::default();
    if wanted(input, "name") {
        data.name = input.string("name", Str::trimmed(1, 255));
    }
    if wanted(input, "prompt") {
        data.prompt = input.string("prompt", Str::trimmed(1, 10_000));
    }
    for (name, required) in [("messages", true), ("models", true), ("tools", true)] {
        if !wanted(input, name) {
            continue;
        }
        let Some(value) = input.get(name).cloned() else {
            if required {
                input.issue(
                    &key(name),
                    "invalid_type",
                    "Invalid input: expected array, received undefined",
                );
            }
            continue;
        };
        match name {
            "messages" => data.messages = messages_field(input, &value),
            "models" => data.models = models_field(input, &value),
            _ => data.tools = tools_field(input, &value),
        }
    }
    if wanted(input, "temperature") {
        data.temperature = input.number("temperature", Num::range(0.0, 2.0));
    }
    if wanted(input, "maxOutputTokens") {
        // ponytail: the STRICT integer column refuses a fraction; the
        // original (Postgres integer) fails it later with a 500.
        data.max_output_tokens = input
            .number("maxOutputTokens", Num::int(1.0, 32_768.0))
            .map(|n| n as i64);
    }
    if wanted(input, "isDefault") {
        data.is_default = input.bool("isDefault");
    }
    data.is_rich_response = input
        .opt_bool("isRichResponse")
        .or((!partial).then_some(false));
    data.web_search_authorized_domains = match input.get("webSearchAuthorizedDomains").cloned() {
        Some(value) => domains_field(input, &value),
        None => (!partial).then(Vec::new),
    };
    if public {
        if !partial {
            let unknown: Vec<String> = input
                .obj
                .keys()
                .filter(|k| !PUBLIC_KEYS.contains(&k.as_str()))
                .map(|k| format!("\"{k}\""))
                .collect();
            if !unknown.is_empty() {
                let plural = if unknown.len() > 1 { "s" } else { "" };
                input.issue(
                    &[],
                    "unrecognized_keys",
                    &format!("Unrecognized key{plural}: {}", unknown.join(", ")),
                );
            }
        }
        // The update intersection strips the private keys (DECISIONS Q.D6).
        return data;
    }
    data.action_prompt = match input.get("actionPrompt").cloned() {
        None => (!partial).then_some(None),
        Some(Value::Null) => Some(None),
        Some(value) => input
            .string_at(
                &key("actionPrompt"),
                Some(&value),
                Str::trimmed(0, 10_000),
                true,
            )
            .map(|text| (!text.is_empty()).then_some(text)),
    };
    data.action_rules = match input.get("actionRules") {
        None => (!partial).then(|| json!([])),
        Some(value) => Some(value.clone()),
    };
    data
}

fn not_found(errors: super::ErrorMap) -> crate::orpc::OrpcError {
    business_error(BusinessError::not_found("AI agent not found"), errors)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "aiAgents.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let page = service::list(
                db,
                ws,
                ListAiAgents {
                    paging: PaginationInput {
                        page: Some(page),
                        per_page: Some(per_page),
                    },
                    sort: vec![SortItem {
                        id: "createdAt".into(),
                        desc: true,
                    }],
                    name: None,
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(public_ai_agent_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "aiAgents.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let agent_id = input.id("id");
            input.finish()?;
            service::find(db, ws, agent_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?
                .map(|row| json(200, public_ai_agent_json(&row)))
                .ok_or_else(|| not_found(FINDING))
        }
        "aiAgents.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_agent_input(&mut input, false, true);
            input.finish()?;
            service::create(db, &state.audit, ws, data)
                .await
                .map(|row| json(201, public_ai_agent_json(&row)))
                .map_err(|e| business_error(e, CREATING))
        }
        "aiAgents.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_agent_input(&mut input, true, true);
            let agent_id = input.id("id");
            input.finish()?;
            service::update(db, &state.audit, ws, agent_id.unwrap_or_default(), data)
                .await
                .map(|row| json(200, public_ai_agent_json(&row)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "aiAgents.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let agent_id = input.id("id");
            input.finish()?;
            service::delete(db, &state.audit, ws, vec![agent_id.unwrap_or_default()])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
