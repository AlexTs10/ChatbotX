//! `packages/ai/src/server/mcp-client.ts` (`McpClient`): JSON-RPC 2.0 over
//! the MCP Streamable HTTP transport (one POST per call; the answer is JSON
//! or an SSE stream whose last `data:` line is the result). Also the
//! transport of `validateAIMcpServer` (`@ai-sdk/mcp` `createMCPClient`):
//! [`McpClient::sdk`] + [`McpClient::sdk_tools`].
//!
//! The endpoint guard is injected ([`EndpointGuard`]); the default is
//! `assertSafeMcpEndpoint` ([`crate::ai_mcp_server::McpEndpointPolicy`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::errors::BusinessError;

/// `mcpConstants.protocolVersion`.
pub const PROTOCOL_VERSION: &str = "2024-11-05";
/// `mcpConstants.clientInfo`.
pub const CLIENT_NAME: &str = "AhaChat-Worker";
/// The `@ai-sdk/mcp` client name (`clientName` default).
pub const SDK_CLIENT_NAME: &str = "ai-sdk-mcp-client";
/// The protocol version the `@ai-sdk/mcp` HTTP transport sends first.
pub const SDK_PROTOCOL_VERSION: &str = "2025-11-25";
/// `aiTimeouts.httpDefault` / `mcpList`.
pub const HTTP_TIMEOUT: Duration = Duration::from_millis(30_000);
/// `aiTimeouts.mcpCall`.
pub const CALL_TIMEOUT: Duration = Duration::from_millis(60_000);
/// `ENDPOINT_CHECK_TTL_MS`.
const ENDPOINT_CHECK_TTL: Duration = Duration::from_millis(60_000);

/// A boxed future (object-safe trait methods).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `assertSafeMcpEndpoint(url)`: `Err` = the endpoint is refused.
pub trait EndpointGuard: Send + Sync {
    fn check<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<(), BusinessError>>;
}

/// `AIMcpServerAuth` (`none` / `token` / `header`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpAuth {
    None,
    Token(String),
    Header(Vec<(String, String)>),
}

impl McpAuth {
    /// The stored `auth` JSON (an unknown shape is `none`).
    pub fn from_json(value: &Value) -> McpAuth {
        match value.get("type").and_then(Value::as_str) {
            Some("token") => McpAuth::Token(
                value
                    .get("token")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ),
            Some("header") => McpAuth::Header(
                value
                    .get("headers")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .map(|h| {
                                let text = |k: &str| {
                                    h.get(k)
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string()
                                };
                                (text("header"), text("value"))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            _ => McpAuth::None,
        }
    }

    /// The request headers of this auth.
    pub fn headers(&self) -> Vec<(String, String)> {
        match self {
            McpAuth::None => Vec::new(),
            McpAuth::Token(token) => vec![("Authorization".into(), format!("Bearer {token}"))],
            McpAuth::Header(headers) => headers.clone(),
        }
    }
}

/// `normalizeMcpContent`: a content array whose first item is text becomes
/// that text; anything else is kept.
pub fn normalize_mcp_content(content: &Value) -> Value {
    if let Some(first) = content.as_array().and_then(|items| items.first())
        && first.get("type").and_then(Value::as_str) == Some("text")
        && let Some(text) = first.get("text").and_then(Value::as_str)
    {
        return Value::String(text.to_string());
    }
    content.clone()
}

/// `omitEmptyPagingToken`.
pub fn omit_empty_paging_token(args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = args.clone();
    if out.get("paging_token").and_then(Value::as_str) == Some("") {
        out.remove("paging_token");
    }
    out
}

/// One `tools/call` result.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallResult {
    pub is_error: bool,
    pub content: Value,
}

/// `McpClient`.
pub struct McpClient {
    url: String,
    name: String,
    headers: Vec<(String, String)>,
    guard: Arc<dyn EndpointGuard>,
    protocol_version: &'static str,
    client_name: &'static str,
    http: reqwest::Client,
    session_id: Option<String>,
    initialized: bool,
    legacy: bool,
    request_counter: u64,
    endpoint_checked_until: Option<Instant>,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // `redirect: "error"`.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

impl McpClient {
    /// `new McpClient({ url, auth, name })`.
    pub fn new(
        url: &str,
        auth: &McpAuth,
        name: Option<&str>,
        guard: Arc<dyn EndpointGuard>,
    ) -> McpClient {
        McpClient {
            url: url.to_string(),
            name: name.unwrap_or("MCP Server").to_string(),
            headers: auth.headers(),
            guard,
            protocol_version: PROTOCOL_VERSION,
            client_name: CLIENT_NAME,
            http: http_client(),
            session_id: None,
            initialized: false,
            legacy: false,
            request_counter: 0,
            endpoint_checked_until: None,
        }
    }

    /// The `@ai-sdk/mcp` HTTP client of `validateAIMcpServer` (the headers
    /// are already resolved; the caller ran the endpoint guard).
    pub fn sdk(url: &str, headers: Vec<(String, String)>) -> McpClient {
        struct Pass;
        impl EndpointGuard for Pass {
            fn check<'a>(&'a self, _url: &'a str) -> BoxFuture<'a, Result<(), BusinessError>> {
                Box::pin(async { Ok(()) })
            }
        }
        McpClient {
            headers,
            protocol_version: SDK_PROTOCOL_VERSION,
            client_name: SDK_CLIENT_NAME,
            ..McpClient::new(url, &McpAuth::None, None, Arc::new(Pass))
        }
    }

    /// `ensureSafeEndpoint`: a passed check is reused for 60 s; a refused
    /// one is not cached.
    async fn ensure_safe_endpoint(&mut self) -> Result<(), BusinessError> {
        if self
            .endpoint_checked_until
            .is_some_and(|until| until > Instant::now())
        {
            return Ok(());
        }
        self.guard.check(&self.url).await?;
        self.endpoint_checked_until = Some(Instant::now() + ENDPOINT_CHECK_TTL);
        Ok(())
    }

    fn next_request_id(&mut self) -> u64 {
        self.request_counter = (self.request_counter + 1) % 1_000_000;
        u64::try_from(chatbotx_db::repo::now_ms()).unwrap_or_default() + self.request_counter
    }

    /// One JSON-RPC call; `Ok(None)` for a notification.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        notification: bool,
    ) -> Result<Option<Map<String, Value>>, BusinessError> {
        self.ensure_safe_endpoint().await?;
        let mut body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if !notification {
            body["id"] = json!(self.next_request_id());
        }
        let mut builder = self
            .http
            .post(&self.url)
            .timeout(timeout)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if self.protocol_version == SDK_PROTOCOL_VERSION {
            builder = builder.header("mcp-protocol-version", self.protocol_version);
        }
        if let Some(session) = &self.session_id {
            builder = builder.header("mcp-session-id", session);
        }
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let outcome = async {
            let response = builder
                .body(body.to_string())
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if let Some(session) = response
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
            {
                self.session_id = Some(session.to_string());
            }
            let text = response.text().await.map_err(|e| e.to_string())?;
            if notification {
                return Ok(None);
            }
            parse_rpc_response(&text).map(Some)
        }
        .await;
        outcome.map_err(|message| {
            tracing::error!(method, url = %self.url, error = %message, "[McpClient][{}] Request failed", self.name);
            BusinessError::exception(message, "mcpRequestFailed", 502)
        })
    }

    /// `initialize` (+ the `notifications/initialized` notification). A
    /// "method not found" answer switches to legacy mode.
    pub async fn initialize(&mut self) -> Result<(), BusinessError> {
        if self.initialized {
            return Ok(());
        }
        let params = json!({
            "protocolVersion": self.protocol_version,
            "capabilities": {},
            "clientInfo": { "name": self.client_name, "version": "1.0.0" },
        });
        match self
            .request("initialize", params, HTTP_TIMEOUT, false)
            .await
        {
            Ok(_) => {
                if let Err(error) = self
                    .request("notifications/initialized", json!({}), HTTP_TIMEOUT, true)
                    .await
                {
                    tracing::warn!(error = %error.message(), "[McpClient][{}] Failed to send initialized notification", self.name);
                }
                self.initialized = true;
                self.legacy = false;
                Ok(())
            }
            Err(error) if error.message().to_lowercase().contains("method not found") => {
                self.legacy = true;
                self.initialized = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// `listTools` (one page, like the original).
    pub async fn list_tools(&mut self) -> Result<Vec<Value>, BusinessError> {
        self.initialize().await?;
        let result = self
            .request("tools/list", json!({}), HTTP_TIMEOUT, false)
            .await?;
        Ok(result
            .and_then(|r| r.get("tools").and_then(Value::as_array).cloned())
            .unwrap_or_default())
    }

    /// `callTool`.
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        args: &Map<String, Value>,
    ) -> Result<ToolCallResult, BusinessError> {
        self.initialize().await?;
        let params = json!({ "name": tool_name, "arguments": omit_empty_paging_token(args) });
        let result = self
            .request("tools/call", params, CALL_TIMEOUT, false)
            .await?
            .ok_or_else(|| {
                BusinessError::exception(
                    format!(
                        "[McpClient][{}] No result returned from tool call: {tool_name}",
                        self.name
                    ),
                    "mcpRequestFailed",
                    502,
                )
            })?;
        Ok(ToolCallResult {
            is_error: result.get("isError") == Some(&Value::Bool(true)),
            content: result.get("content").cloned().unwrap_or(Value::Null),
        })
    }

    /// `getLegacyStatus`.
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }

    /// `client.tools()` of `@ai-sdk/mcp` (every page), JSON-serialized as
    /// `validateAIMcpServer` returns it: per tool `{ description?, title?,
    /// metadata, inputSchema: { jsonSchema }, type: "dynamic", _meta? }`.
    pub async fn sdk_tools(&mut self) -> Result<Map<String, Value>, BusinessError> {
        self.initialize().await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self
                .request("tools/list", params, HTTP_TIMEOUT, false)
                .await?
                .unwrap_or_default();
            tools.extend(
                result
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        let mut out = Map::new();
        for tool in tools {
            let Some(name) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            out.insert(name.to_string(), sdk_tool_json(self.client_name, &tool));
        }
        Ok(out)
    }
}

/// The JSON of one `dynamicTool` built by `toolsFromDefinitions`.
fn sdk_tool_json(client_name: &str, tool: &Value) -> Value {
    let name = tool.get("name").cloned().unwrap_or(Value::Null);
    let annotations = tool.get("annotations").and_then(Value::as_object);
    let title = tool
        .get("title")
        .filter(|t| !t.is_null())
        .or_else(|| {
            annotations
                .and_then(|a| a.get("title"))
                .filter(|t| !t.is_null())
        })
        .cloned();
    let mut metadata = Map::new();
    metadata.insert("clientName".into(), json!(client_name));
    metadata.insert("toolName".into(), name);
    if let Some(title) = &title {
        metadata.insert("title".into(), title.clone());
    }
    if let Some(annotations) = annotations {
        let mut kept = Map::new();
        for key in [
            "title",
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            if let Some(value) = annotations.get(key).filter(|v| !v.is_null()) {
                kept.insert(key.into(), value.clone());
            }
        }
        metadata.insert("annotations".into(), Value::Object(kept));
    }
    let mut schema = tool
        .get("inputSchema")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if schema.get("properties").is_none_or(Value::is_null) {
        schema.insert("properties".into(), json!({}));
    }
    schema.insert("additionalProperties".into(), Value::Bool(false));
    let mut out = Map::new();
    if let Some(description) = tool.get("description").filter(|d| !d.is_null()) {
        out.insert("description".into(), description.clone());
    }
    if let Some(title) = title {
        out.insert("title".into(), title);
    }
    out.insert("metadata".into(), Value::Object(metadata));
    out.insert("inputSchema".into(), json!({ "jsonSchema": schema }));
    out.insert("type".into(), json!("dynamic"));
    if let Some(meta) = tool.get("_meta").filter(|m| !m.is_null()) {
        out.insert("_meta".into(), meta.clone());
    }
    Value::Object(out)
}

/// The JSON-RPC answer of one call (plain JSON or SSE, last `data:` line):
/// the `result` object, or the error message.
pub fn parse_rpc_response(text: &str) -> Result<Map<String, Value>, String> {
    let trimmed = text.trim();
    let payload = if trimmed.contains("data:") {
        trimmed
            .split('\n')
            .filter(|line| line.starts_with("data:"))
            .map(|line| line["data:".len()..].trim())
            .next_back()
            .unwrap_or("{}")
    } else {
        trimmed
    };
    let parsed: Value = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    if parsed.get("jsonrpc").is_some_and(Value::is_string)
        && let Some(error) = parsed.get("error")
        && error.get("code").is_some_and(Value::is_number)
        && let Some(message) = error.get("message").and_then(Value::as_str)
    {
        return Err(message.to_string());
    }
    match (parsed.get("jsonrpc"), parsed.get("result")) {
        (Some(Value::String(_)), Some(Value::Object(result))) => Ok(result.clone()),
        _ => Err("Invalid JSON-RPC 2.0 response".to_string()),
    }
}
