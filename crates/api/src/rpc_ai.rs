//! The AI procedures of the private `/rpc` router and the AI server actions
//! of the builder (`/rpc/actions/<name>`, P4-web §5.1), Q.D6.
//!
//! [`call`] runs one by its path (`aiFunctionsAPI.listAIFunctionsAuthenticatedAPI`,
//! `actions.createAIFunctionAction`, ...) after the `/rpc` session and
//! workspace auth; the `/rpc` router wires it in. Procedure input is the
//! `json` of the request; action input is `{ args: [...bind args, input?] }`.
//! The output is RPC-encoded by the caller (`{ json, meta }`, [`RpcOutput`]).

use std::sync::Arc;

use chatbotx_business::BusinessError;
use chatbotx_business::ai::mcp_client::EndpointGuard;
use chatbotx_business::ai_agent::{self as agents, ListAiAgents};
use chatbotx_business::ai_function as functions;
use chatbotx_business::ai_mcp_server::{self as mcp, McpEndpointPolicy};
use chatbotx_business::integration_ai_provider as ai_providers;
use chatbotx_core::variables::bot_field::BotFieldVariableTextResolution;
use chatbotx_db::repo::utils::{PaginationInput, SortItem};
use serde_json::{Map, Value, json};

use crate::handlers::ai_agents::{ai_agent_json, parse_ai_agent_input};
use crate::handlers::ai_functions::{ai_function_json, parse_ai_function_input};
use crate::handlers::ai_mcp_servers::{
    ai_mcp_server_json, auth_field, parse_ai_mcp_server_input, url_field,
};
use crate::handlers::input::{Input, Num, Seg, Str, is_bigint_id, key};
use crate::handlers::{ApiState, CREATING, FINDING, LISTING, MUTATING, business_error};
use crate::orpc::OrpcError;

/// The AI procedures and actions with a Rust handler.
pub const PORTED: &[&str] = &[
    "aiAgentsAPI.listAIAgentsAPI",
    "actions.updateIntegrationClaudeAction",
    "actions.updateIntegrationDeepSeekAction",
    "actions.updateGeminiAction",
    "actions.updateIntegrationMuseAction",
    "actions.updateIntegrationOpenRouterAction",
    "actions.updateIntegrationOpenAIAction",
    "actions.createAIAgentAction",
    "actions.updateAIAgentAction",
    "actions.deleteAIAgentAction",
    "aiFunctionsAPI.listAIFunctionsAuthenticatedAPI",
    "aiMcpServerAPIs.listAIMcpServersAuthenticatedAPI",
    "aiMcpServerAPIs.validateAIMcpServerAuthenticatedAPI",
    "actions.createAIFunctionAction",
    "actions.updateAIFunctionAction",
    "actions.deleteAIFunctionAction",
    "actions.createAIMcpServerAction",
    "actions.updateAIMcpServerAction",
    "actions.deleteAIMcpServerAction",
    "actions.deleteAIMcpServer",
    "actions.validateAIMcpServer",
];

/// The caller after the `/rpc` auth.
#[derive(Clone)]
pub struct RpcAiContext {
    pub workspace_id: i64,
    pub user_id: i64,
    /// The MCP endpoint guard (`assertSafeMcpEndpoint`).
    pub mcp_guard: Arc<dyn EndpointGuard>,
}

impl RpcAiContext {
    /// The context with this process's MCP endpoint policy.
    pub fn new(workspace_id: i64, user_id: i64) -> RpcAiContext {
        RpcAiContext {
            workspace_id,
            user_id,
            mcp_guard: Arc::new(McpEndpointPolicy::from_env()),
        }
    }
}

/// A procedure result: the output and the oRPC meta (`[type, ...path]`;
/// type 1 = Date).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RpcOutput {
    pub json: Value,
    pub meta: Vec<Value>,
}

impl RpcOutput {
    fn plain(json: Value) -> RpcOutput {
        RpcOutput {
            json,
            meta: Vec::new(),
        }
    }

    /// `{ json, meta }` (the RPC response body).
    pub fn body(&self) -> Value {
        if self.meta.is_empty() {
            json!({ "json": self.json })
        } else {
            json!({ "json": self.json, "meta": self.meta })
        }
    }
}

/// Meta of the Date fields `fields` of each row of `rows` at `prefix`.
fn date_meta(prefix: &[Value], rows: usize, fields: &[&str]) -> Vec<Value> {
    let mut out = Vec::new();
    for index in 0..rows {
        for field in fields {
            let mut entry = vec![json!(1)];
            entry.extend_from_slice(prefix);
            entry.push(json!(index));
            entry.push(json!(field));
            out.push(Value::Array(entry));
        }
    }
    out
}

/// Meta of the Date fields of one object at `prefix`.
fn object_date_meta(prefix: &[Value], fields: &[&str]) -> Vec<Value> {
    fields
        .iter()
        .map(|field| {
            let mut entry = vec![json!(1)];
            entry.extend_from_slice(prefix);
            entry.push(json!(field));
            Value::Array(entry)
        })
        .collect()
}

const TIMESTAMPS: &[&str] = &["createdAt", "updatedAt"];

/// `BAD_REQUEST "Input validation failed"` (oRPC input schema).
pub fn validation_failed() -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error
}

/// next-safe-action `validationErrors` (formatted errors) of zod issues.
fn formatted(issues: &[Value]) -> Value {
    let mut root = json!({ "_errors": [] });
    for issue in issues {
        let message = issue.get("message").cloned().unwrap_or(Value::Null);
        let path = issue
            .get("path")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut node = &mut root;
        for seg in &path {
            let key = match seg {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            node = node
                .as_object_mut()
                .map(|obj| obj.entry(key).or_insert_with(|| json!({ "_errors": [] })))
                .unwrap_or_else(|| unreachable!());
        }
        if let Some(errors) = node.get_mut("_errors").and_then(Value::as_array_mut) {
            errors.push(message);
        }
    }
    root
}

/// A next-safe-action validation failure (`returnValidationErrors` or the
/// input schema): `400 BAD_REQUEST` with `data.validationErrors`.
pub fn validation_errors(errors: Value) -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error.data = Some(json!({ "validationErrors": errors }));
    error
}

/// An action's parsed input (`args[n]`) or its validation failure.
fn action_input(args: &[Value], index: usize) -> Input {
    match args.get(index) {
        Some(Value::Object(map)) => Input::from_object(map.clone()),
        _ => Input::from_object(Map::new()),
    }
}

fn finish_action(input: Input) -> Result<(), OrpcError> {
    input.finish().map_err(|error| {
        let issues = error
            .data
            .as_ref()
            .and_then(|d| d.get("issues"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        validation_errors(formatted(&issues))
    })
}

/// A bind arg that must be a bigint string (`zodBigintAsString`).
fn bind_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    match args.get(index).and_then(Value::as_str) {
        Some(text) if is_bigint_id(text) => text.parse().map_err(|_| validation_failed()),
        _ => Err(validation_failed()),
    }
}

/// `nameAlreadyExists` of a `validation` error, any other error as is.
fn name_taken(error: BusinessError, feature: &str, errors: crate::handlers::ErrorMap) -> OrpcError {
    if error.code() == Some("validation") {
        return validation_errors(json!({
            "_errors": [],
            "name": { "_errors": [format!("{feature} name already exists")] },
        }));
    }
    business_error(error, errors)
}

/// `bulkUpdateIdsRequest` (`{ ids: zodBigintAsString[] }`).
fn bulk_ids(input: &mut Input) -> Vec<i64> {
    let items = match input.get("ids").cloned() {
        Some(Value::Array(items)) => items,
        _ => {
            input.issue(&key("ids"), "invalid_type", "Invalid input: expected array");
            return Vec::new();
        }
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            input.id_at(
                &[Seg::Key("ids".into()), Seg::Index(index)],
                Some(item),
                true,
            )
        })
        .collect()
}

/// `listAIAgentsRequest` (`page` 1, `perPage` 10, `sort` newest first).
fn parse_list_ai_agents(input: &mut Input) -> ListAiAgents {
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int(
        "perPage",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let mut sort = Vec::new();
    match input.get("sort").cloned() {
        None => sort.push(SortItem {
            id: "createdAt".into(),
            desc: true,
        }),
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let at = vec![Seg::Key("sort".into()), Seg::Index(index)];
                let id = input.string_at(
                    &crate::handlers::input::child(&at, "id"),
                    item.get("id"),
                    Str::default(),
                    true,
                );
                let desc = input.bool_at(
                    &crate::handlers::input::child(&at, "desc"),
                    item.get("desc"),
                    true,
                );
                if let (Some(id), Some(desc)) = (id, desc) {
                    sort.push(SortItem { id, desc });
                }
            }
        }
        Some(_) => input.issue(
            &key("sort"),
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    let name = input.opt_string("name", Str::default());
    ListAiAgents {
        paging: PaginationInput {
            page: Some(page.unwrap_or(1)),
            per_page: Some(per_page.unwrap_or(10)),
        },
        sort,
        name,
    }
}

/// Runs the procedure or action at `path`; `None` = not an AI path.
pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcAiContext,
    input: &Value,
) -> Option<Result<RpcOutput, OrpcError>> {
    if !PORTED.contains(&path) {
        return None;
    }
    Some(run(api, path, context, input).await)
}

async fn run(
    api: &ApiState,
    path: &str,
    context: &RpcAiContext,
    input: &Value,
) -> Result<RpcOutput, OrpcError> {
    let db = &api.db;
    let ws = context.workspace_id;
    let args = input
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match path {
        "aiAgentsAPI.listAIAgentsAPI" => {
            let mut parsed = Input::from_object(input.as_object().cloned().unwrap_or_default());
            let list = parse_list_ai_agents(&mut parsed);
            parsed.finish().map_err(|_| validation_failed())?;
            let page = agents::list(db, ws, list)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(ai_agent_json).collect();
            Ok(RpcOutput {
                meta: date_meta(&[json!("data")], data.len(), TIMESTAMPS),
                json: json!({ "data": data, "pageCount": page.page_count }),
            })
        }
        "actions.createAIAgentAction" => {
            let mut parsed = action_input(&args, 1);
            let data = parse_ai_agent_input(&mut parsed, false, false);
            finish_action(parsed)?;
            agents::create(db, &api.audit, ws, data)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        "actions.updateAIAgentAction" => {
            let id = bind_id(&args, 1)?;
            let mut parsed = action_input(&args, 2);
            let data = parse_ai_agent_input(&mut parsed, true, false);
            finish_action(parsed)?;
            let row = agents::update(db, &api.audit, ws, id, data)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(RpcOutput {
                json: ai_agent_json(&row),
                meta: object_date_meta(&[], TIMESTAMPS),
            })
        }
        "actions.deleteAIAgentAction" => {
            let mut parsed = action_input(&args, 1);
            let ids = bulk_ids(&mut parsed);
            finish_action(parsed)?;
            agents::delete(db, &api.audit, ws, ids)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        "actions.updateIntegrationClaudeAction"
        | "actions.updateIntegrationDeepSeekAction"
        | "actions.updateGeminiAction"
        | "actions.updateIntegrationMuseAction"
        | "actions.updateIntegrationOpenRouterAction"
        | "actions.updateIntegrationOpenAIAction" => {
            let provider = match path {
                "actions.updateIntegrationClaudeAction" => "claude",
                "actions.updateIntegrationDeepSeekAction" => "deepseek",
                "actions.updateGeminiAction" => "gemini",
                "actions.updateIntegrationMuseAction" => "muse",
                "actions.updateIntegrationOpenRouterAction" => "openrouter",
                _ => "openai",
            };
            // OpenAI binds `[workspaceId, id]`; the row id is not needed:
            // the satellite row is unique per workspace.
            let index = if provider == "openai" {
                bind_id(&args, 1)?;
                2
            } else {
                1
            };
            let mut parsed = action_input(&args, index);
            let auto_reply = parsed.opt_bool("autoReply");
            finish_action(parsed)?;
            ai_providers::update_auto_reply(db, &api.audit, provider, ws, auto_reply)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        "aiFunctionsAPI.listAIFunctionsAuthenticatedAPI" => {
            let page = functions::list(db, ws, PaginationInput::default())
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(ai_function_json).collect();
            Ok(RpcOutput {
                meta: date_meta(&[json!("data")], data.len(), TIMESTAMPS),
                json: json!({ "data": data }),
            })
        }
        "aiMcpServerAPIs.listAIMcpServersAuthenticatedAPI" => {
            let page = mcp::list(db, ws, PaginationInput::default())
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(ai_mcp_server_json).collect();
            Ok(RpcOutput {
                meta: date_meta(&[json!("data")], data.len(), TIMESTAMPS),
                json: json!({ "data": data }),
            })
        }
        "aiMcpServerAPIs.validateAIMcpServerAuthenticatedAPI" => {
            let mut parsed = Input::from_object(input.as_object().cloned().unwrap_or_default());
            let url = url_field(&mut parsed, "url");
            let auth = auth_field(&mut parsed, true);
            parsed.finish().map_err(|_| validation_failed())?;
            let (url, auth) = (url.unwrap_or_default(), auth.unwrap_or_default());
            // `normalizeError` + `new ChatbotXException(message, code)`: 400.
            mcp::validate(db, Arc::clone(&context.mcp_guard), ws, &url, &auth)
                .await
                .map(|tools| RpcOutput::plain(Value::Object(tools)))
                .map_err(|error| {
                    let code = error.code().unwrap_or("businessError").to_string();
                    business_error(
                        BusinessError::exception(error.message(), &code, 400),
                        LISTING,
                    )
                })
        }
        "actions.validateAIMcpServer" => {
            let mut parsed = action_input(&args, 0);
            let url = url_field(&mut parsed, "url");
            let auth = auth_field(&mut parsed, true);
            finish_action(parsed)?;
            mcp::validate(
                db,
                Arc::clone(&context.mcp_guard),
                ws,
                &url.unwrap_or_default(),
                &auth.unwrap_or_default(),
            )
            .await
            .map(|tools| RpcOutput::plain(Value::Object(tools)))
            .map_err(|e| business_error(e, LISTING))
        }
        "actions.createAIFunctionAction" => {
            let mut parsed = action_input(&args, 1);
            let data = parse_ai_function_input(&mut parsed);
            finish_action(parsed)?;
            functions::create(db, &api.audit, ws, data.unwrap_or_default())
                .await
                .map_err(|e| name_taken(e, "AI Function", CREATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        "actions.updateAIFunctionAction" => {
            let id = bind_id(&args, 1)?;
            let mut parsed = action_input(&args, 2);
            let data = parse_ai_function_input(&mut parsed);
            finish_action(parsed)?;
            let row = functions::update(db, &api.audit, ws, id, data.unwrap_or_default())
                .await
                .map_err(|e| name_taken(e, "AI Function", MUTATING))?;
            Ok(RpcOutput {
                json: ai_function_json(&row),
                meta: object_date_meta(&[], TIMESTAMPS),
            })
        }
        "actions.deleteAIFunctionAction" => {
            let id = bind_id(&args, 1)?;
            functions::delete(db, &api.audit, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        "actions.createAIMcpServerAction" | "actions.updateAIMcpServerAction" => {
            let update = path == "actions.updateAIMcpServerAction";
            let id = if update {
                Some(bind_id(&args, 1)?)
            } else {
                None
            };
            let mut parsed = action_input(&args, if update { 2 } else { 1 });
            let data = parse_ai_mcp_server_input(&mut parsed, true);
            finish_action(parsed)?;
            let data = data.unwrap_or_default();
            if let Some(token) = data.auth.get("token").and_then(Value::as_str) {
                let resolution = mcp::resolve_bot_field_variable_text(db, ws, token)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?;
                if !matches!(resolution, BotFieldVariableTextResolution::Resolved { .. }) {
                    return Err(validation_errors(json!({
                        "_errors": [],
                        "auth": { "_errors": [], "token": { "_errors": ["Invalid API key"] } },
                    })));
                }
            }
            match id {
                None => {
                    mcp::create(db, &api.audit, ws, data)
                        .await
                        .map_err(|e| name_taken(e, "MCP Server", CREATING))?;
                }
                Some(id) => {
                    if mcp::find(db, ws, id)
                        .await
                        .map_err(|e| business_error(e, FINDING))?
                        .is_none()
                    {
                        return Err(business_error(
                            BusinessError::not_found("MCP Server not found"),
                            MUTATING,
                        ));
                    }
                    mcp::update(db, &api.audit, ws, id, data)
                        .await
                        .map_err(|e| name_taken(e, "MCP Server", MUTATING))?;
                }
            }
            Ok(RpcOutput::plain(Value::Null))
        }
        "actions.deleteAIMcpServerAction" | "actions.deleteAIMcpServer" => {
            let id = if path == "actions.deleteAIMcpServer" {
                // `deleteAIMcpServer({ workspaceId, aiMcpServerId })`.
                args.first()
                    .and_then(|a| a.get("aiMcpServerId"))
                    .and_then(Value::as_str)
                    .and_then(|t| t.parse().ok())
                    .ok_or_else(validation_failed)?
            } else {
                bind_id(&args, 1)?
            };
            if mcp::find(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?
                .is_none()
            {
                return Err(business_error(
                    BusinessError::not_found("MCP Server not found"),
                    MUTATING,
                ));
            }
            mcp::delete(db, &api.audit, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(RpcOutput::plain(Value::Null))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
