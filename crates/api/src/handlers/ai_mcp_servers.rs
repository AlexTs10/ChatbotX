//! `apps/builder/src/features/ai-mcp-servers/api/public.ts`
//! (`aiMcpServersPublicRouter`).

use chatbotx_business::BusinessError;
use chatbotx_business::ai_mcp_server::{self as service, AiMcpServerInput};
use chatbotx_db::repo::models::AiMcpServer;
use chatbotx_db::repo::utils::PaginationInput;
use serde_json::{Map, Value, json};

use super::input::{Input, Path, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "aiMcpServers.list",
    "aiMcpServers.create",
    "aiMcpServers.get",
    "aiMcpServers.update",
    "aiMcpServers.delete",
];

/// `aiMcpServerResource` with `auth` as stored (the builder UI's resource).
pub fn ai_mcp_server_json(row: &AiMcpServer) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "url": row.url,
        "auth": row.auth.0,
        "availableTools": row.available_tools.0,
        "selectedTools": row.selected_tools.0,
        "workspaceId": id(row.workspace_id),
    })
}

/// `publicAIMcpServerAuth`: the type and header names, never a secret.
pub fn public_auth(auth: &Value) -> Value {
    match auth.get("type").and_then(Value::as_str) {
        Some("header") => json!({
            "type": "header",
            "headers": auth
                .get("headers")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|h| json!({ "header": h.get("header").cloned().unwrap_or(Value::Null) }))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        }),
        Some(kind) => json!({ "type": kind }),
        None => json!({ "type": "none" }),
    }
}

/// `publicAIMcpServerResource`.
pub fn public_ai_mcp_server_json(row: &AiMcpServer) -> Value {
    let mut out = ai_mcp_server_json(row);
    out["auth"] = public_auth(&row.auth.0);
    out
}

/// `z.url()` at `name`.
pub fn url_field(input: &mut Input, name: &str) -> Option<String> {
    let text = input.string(name, Str::default())?;
    if url::Url::parse(&text).is_err() {
        input.issue(&key(name), "invalid_format", "Invalid URL");
        return None;
    }
    Some(text)
}

/// `aiMcpServerAuth` (`private`: the builder form's token must be static
/// text plus `{{bot_field:<id>}}` variables).
pub fn auth_field(input: &mut Input, private: bool) -> Option<Value> {
    let path = key("auth");
    let Some(Value::Object(auth)) = input.get("auth").cloned() else {
        match input.get("auth") {
            None => input.issue(
                &path,
                "invalid_type",
                "Invalid input: expected object, received undefined",
            ),
            Some(_) => input.issue(&path, "invalid_type", "Invalid input: expected object"),
        }
        return None;
    };
    let spec = Str::trimmed(1, usize::MAX);
    match auth.get("type").and_then(Value::as_str) {
        Some("none") => Some(json!({ "type": "none" })),
        Some("token") => {
            let token = input.string_at(&child(&path, "token"), auth.get("token"), spec, true)?;
            if private
                && !matches!(
                    chatbotx_core::variables::bot_field::parse_bot_field_variable_text(&token),
                    chatbotx_core::variables::bot_field::BotFieldVariableTextParseResult::Valid { .. }
                )
            {
                input.issue(&child(&path, "token"), "custom", "Invalid input");
                return None;
            }
            Some(json!({ "type": "token", "token": token }))
        }
        Some("header") => {
            let headers_path = child(&path, "headers");
            let Some(Value::Array(items)) = auth.get("headers").cloned() else {
                input.issue(
                    &headers_path,
                    "invalid_type",
                    "Invalid input: expected array",
                );
                return None;
            };
            let mut out = Vec::new();
            let mut ok = true;
            for (index, item) in items.iter().enumerate() {
                let mut at: Path = headers_path.clone();
                at.push(Seg::Index(index));
                let header = input.string_at(&child(&at, "header"), item.get("header"), spec, true);
                let value = input.string_at(&child(&at, "value"), item.get("value"), spec, true);
                match (header, value) {
                    (Some(header), Some(value)) => {
                        out.push(json!({ "header": header, "value": value }));
                    }
                    _ => ok = false,
                }
            }
            ok.then(|| json!({ "type": "header", "headers": out }))
        }
        _ => {
            input.issue(&child(&path, "type"), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `createAIMcpServerRequest` (= `updateAIMcpServerRequest`).
pub fn parse_ai_mcp_server_input(input: &mut Input, private: bool) -> Option<AiMcpServerInput> {
    let url = url_field(input, "url");
    let auth = auth_field(input, private);
    let name = input.string("name", Str::trimmed(1, usize::MAX));
    let available_tools = match input.get("availableTools").cloned() {
        Some(Value::Object(map)) => Some(map),
        None => {
            input.issue(
                &key("availableTools"),
                "invalid_type",
                "Invalid input: expected record, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(
                &key("availableTools"),
                "invalid_type",
                "Invalid input: expected record",
            );
            None
        }
    };
    let selected_tools = match input.get("selectedTools").cloned() {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let path = vec![Seg::Key("selectedTools".into()), Seg::Index(index)];
                if let Some(text) = input.string_at(&path, Some(item), Str::default(), true) {
                    out.push(text);
                }
            }
            Some(out)
        }
        None => {
            input.issue(
                &key("selectedTools"),
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(
                &key("selectedTools"),
                "invalid_type",
                "Invalid input: expected array",
            );
            None
        }
    };
    if input.failed() {
        return None;
    }
    Some(AiMcpServerInput {
        name: name?,
        url: url?,
        auth: auth?,
        available_tools: available_tools.unwrap_or_else(Map::new),
        selected_tools: selected_tools?,
    })
}

fn not_found(errors: super::ErrorMap) -> crate::orpc::OrpcError {
    business_error(BusinessError::not_found("AI MCP server not found"), errors)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "aiMcpServers.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let page = service::list(
                db,
                ws,
                PaginationInput {
                    page: Some(page),
                    per_page: Some(per_page),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(public_ai_mcp_server_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "aiMcpServers.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let server_id = input.id("id");
            input.finish()?;
            service::find(db, ws, server_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?
                .map(|row| json(200, public_ai_mcp_server_json(&row)))
                .ok_or_else(|| not_found(FINDING))
        }
        "aiMcpServers.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_mcp_server_input(&mut input, false);
            input.finish()?;
            service::create(db, &state.audit, ws, data.unwrap_or_default())
                .await
                .map(|row| json(201, public_ai_mcp_server_json(&row)))
                .map_err(|e| business_error(e, CREATING))
        }
        "aiMcpServers.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_mcp_server_input(&mut input, false);
            let server_id = input.id("id");
            input.finish()?;
            service::update(
                db,
                &state.audit,
                ws,
                server_id.unwrap_or_default(),
                data.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?
            .map(|row| json(200, public_ai_mcp_server_json(&row)))
            .ok_or_else(|| not_found(MUTATING))
        }
        "aiMcpServers.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let server_id = input.id("id");
            input.finish()?;
            let deleted = service::delete(db, &state.audit, ws, server_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if deleted.is_empty() {
                return Err(not_found(MUTATING));
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
