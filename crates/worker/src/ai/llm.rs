//! The `ai` SDK calls the worker makes (`streamText` / `generateText` with
//! tools), on the three provider wire formats: OpenAI chat completions
//! (OpenAI, DeepSeek, OpenRouter, Muse, OpenAI-compatible), Anthropic
//! messages and Gemini `generateContent`.
//!
//! ponytail: one non-streamed request per step. The original streams and
//! never sends a partial answer either (`sendParts: false`); add SSE when a
//! caller sends paragraphs while the model writes.

use std::time::Duration;

use chatbotx_jobs::BoxFuture;
use serde_json::{Map, Value, json};

/// `aiTimeouts.aiStep`: one model request.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(60);
/// `MUSE_LOW_BUDGET_THRESHOLD`.
const MUSE_LOW_BUDGET_THRESHOLD: i64 = 1024;
/// `MUSE_REASONING_HEADROOM_TOKENS`.
const MUSE_REASONING_HEADROOM_TOKENS: i64 = 1024;
/// `MUSE_MAX_TEMPERATURE`.
const MUSE_MAX_TEMPERATURE: f64 = 1.0;
/// `@ai-sdk/anthropic` needs `max_tokens`; its default for unknown models.
const ANTHROPIC_DEFAULT_MAX_TOKENS: i64 = 4096;

/// A JSON POST (status and body text; `Err` = no answer).
pub type HttpFuture<'a> = BoxFuture<'a, Result<(u16, String), String>>;

/// The HTTP transport of the model calls (a fake in tests).
pub trait AiHttp: Send + Sync {
    fn post<'a>(
        &'a self,
        url: &'a str,
        headers: &'a [(String, String)],
        body: &'a Value,
        timeout: Duration,
    ) -> HttpFuture<'a>;
}

/// [`AiHttp`] on reqwest.
#[derive(Debug, Clone, Default)]
pub struct ReqwestHttp(reqwest::Client);

impl AiHttp for ReqwestHttp {
    fn post<'a>(
        &'a self,
        url: &'a str,
        headers: &'a [(String, String)],
        body: &'a Value,
        timeout: Duration,
    ) -> HttpFuture<'a> {
        Box::pin(async move {
            let mut request = self
                .0
                .post(url)
                .timeout(timeout)
                .header("content-type", "application/json")
                .body(body.to_string());
            for (name, value) in headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("Cannot connect to API: {e}"))?;
            let status = response.status().as_u16();
            let text = response.text().await.map_err(|e| e.to_string())?;
            Ok((status, text))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    OpenAi,
    Anthropic,
    Gemini,
}

/// `transformRequestBody` of the OpenAI-compatible providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    None,
    /// `transformMuseRequestBody`.
    Muse,
    /// The `nearai` preset drops `reasoning_effort`.
    StripReasoning,
}

/// A language model instance (`provider(modelId)`).
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// `openai`, `claude`, ..., `openaiCompatible`.
    pub provider: String,
    pub wire: Wire,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model_id: String,
    pub transform: Transform,
}

/// One user content part.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// An image or file by URL (`data:` URLs carry the bytes).
    File {
        url: String,
        media_type: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub id: String,
    pub name: String,
    pub output: String,
    pub is_error: bool,
}

/// A `ModelMessage` (plus the tool turns of the step loop).
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    User(Vec<Part>),
    Assistant { text: String, calls: Vec<ToolCall> },
    Tool(Vec<ToolOutput>),
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Turn {
        Turn::User(vec![Part::Text(text.into())])
    }

    pub fn assistant(text: impl Into<String>) -> Turn {
        Turn::Assistant {
            text: text.into(),
            calls: Vec::new(),
        }
    }
}

/// A tool as the model sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON schema of the input object.
    pub parameters: Value,
}

/// The executable tool set (`ToolSet`).
pub trait Tools: Send + Sync {
    fn defs(&self) -> Vec<ToolDef>;
    /// The tool output text; `Err` is a tool error the model sees.
    fn call<'a>(&'a self, name: &'a str, args: Value) -> BoxFuture<'a, Result<String, String>>;
}

/// No tools.
pub struct NoTools;

impl Tools for NoTools {
    fn defs(&self) -> Vec<ToolDef> {
        Vec::new()
    }

    fn call<'a>(&'a self, name: &'a str, _args: Value) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move { Err(format!("Model tried to call unavailable tool '{name}'")) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolChoice {
    /// Not sent.
    Unset,
    Auto,
    None,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub system: Option<String>,
    pub messages: Vec<Turn>,
    pub max_output_tokens: Option<i64>,
    pub temperature: Option<f64>,
    /// `stopWhen: isStepCount(n)` (default 1).
    pub max_steps: usize,
    pub tool_choice: ToolChoice,
}

impl Request {
    pub fn new(system: Option<String>, messages: Vec<Turn>) -> Request {
        Request {
            system,
            messages,
            max_output_tokens: None,
            temperature: None,
            max_steps: 1,
            tool_choice: ToolChoice::Unset,
        }
    }
}

/// What the steps produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Generated {
    /// The text of each step.
    pub steps: Vec<String>,
    pub tool_calls: usize,
    pub tool_errors: usize,
}

impl Generated {
    /// `textStream`: every step's text with no separator.
    pub fn text(&self) -> String {
        self.steps.concat()
    }

    /// `toStepSeparatedTextStream`: a paragraph break between the texts of
    /// two steps.
    pub fn step_separated_text(&self) -> String {
        self.steps
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// `toFinalStepTextStream`.
    pub fn final_step_text(&self) -> String {
        self.steps.last().cloned().unwrap_or_default()
    }
}

/// `APICallError` (or a transport failure: `status` `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct LlmError {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub message: String,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One parsed model answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepAnswer {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

/// Runs the steps: a step with tool calls runs the tools and, below
/// `max_steps`, asks the model again with their outputs.
pub async fn generate(
    http: &dyn AiHttp,
    model: &Model,
    request: &Request,
    tools: &dyn Tools,
) -> Result<Generated, LlmError> {
    let defs = match request.tool_choice {
        ToolChoice::None => Vec::new(),
        _ => tools.defs(),
    };
    let mut turns = request.messages.clone();
    let mut generated = Generated::default();
    loop {
        let (url, headers, body) = build_request(model, request, &turns, &defs);
        let (status, text) = http
            .post(&url, &headers, &body, STEP_TIMEOUT)
            .await
            .map_err(|message| LlmError {
                status: None,
                body: None,
                message,
            })?;
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if !(200..300).contains(&status) {
            return Err(LlmError {
                status: Some(status),
                message: error_message(parsed.as_ref())
                    .unwrap_or_else(|| format!("Provider request failed with status {status}")),
                body: Some(text),
            });
        }
        let answer = parsed
            .as_ref()
            .and_then(|body| parse_answer(model.wire, body))
            .ok_or_else(|| LlmError {
                status: Some(status),
                body: Some(text.clone()),
                message: "Invalid JSON response".into(),
            })?;
        generated.steps.push(answer.text.clone());
        if answer.calls.is_empty() {
            return Ok(generated);
        }
        let mut outputs = Vec::new();
        for call in &answer.calls {
            generated.tool_calls += 1;
            let (output, is_error) = match tools.call(&call.name, call.args.clone()).await {
                Ok(output) => (output, false),
                Err(error) => {
                    generated.tool_errors += 1;
                    (error, true)
                }
            };
            outputs.push(ToolOutput {
                id: call.id.clone(),
                name: call.name.clone(),
                output,
                is_error,
            });
        }
        if generated.steps.len() >= request.max_steps.max(1) {
            return Ok(generated);
        }
        turns.push(Turn::Assistant {
            text: answer.text,
            calls: answer.calls,
        });
        turns.push(Turn::Tool(outputs));
    }
}

/// The provider's error text (`error.message`, `error` or `message`).
fn error_message(body: Option<&Value>) -> Option<String> {
    let body = body?;
    let error = body.get("error");
    error
        .and_then(|e| e.get("message"))
        .or(error.filter(|e| e.is_string()))
        .or(body.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// URL, headers and JSON body of one step.
pub fn build_request(
    model: &Model,
    request: &Request,
    turns: &[Turn],
    defs: &[ToolDef],
) -> (String, Vec<(String, String)>, Value) {
    let key = model.api_key.clone().unwrap_or_default();
    match model.wire {
        Wire::OpenAi => {
            let mut headers = Vec::new();
            if model.api_key.is_some() {
                headers.push(("Authorization".into(), format!("Bearer {key}")));
            }
            (
                format!("{}/chat/completions", model.base_url.trim_end_matches('/')),
                headers,
                openai_body(model, request, turns, defs),
            )
        }
        Wire::Anthropic => (
            format!("{}/messages", model.base_url.trim_end_matches('/')),
            vec![
                ("x-api-key".into(), key),
                ("anthropic-version".into(), "2023-06-01".into()),
            ],
            anthropic_body(model, request, turns, defs),
        ),
        Wire::Gemini => (
            format!(
                "{}/models/{}:generateContent",
                model.base_url.trim_end_matches('/'),
                model.model_id
            ),
            vec![("x-goog-api-key".into(), key)],
            gemini_body(request, turns, defs),
        ),
    }
}

fn set_opt<T: Into<Value>>(body: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        body.insert(key.into(), value.into());
    }
}

/// `@ai-sdk/openai` reasoning models take `max_completion_tokens` and no
/// temperature.
fn is_openai_reasoning(model: &Model) -> bool {
    model.provider == "openai"
        && (model.model_id.starts_with('o') || model.model_id.starts_with("gpt-5"))
}

fn openai_body(model: &Model, request: &Request, turns: &[Turn], defs: &[ToolDef]) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for turn in turns {
        match turn {
            Turn::User(parts) => {
                let content = match parts.as_slice() {
                    [Part::Text(text)] => json!(text),
                    parts => Value::Array(
                        parts
                            .iter()
                            .map(|part| match part {
                                Part::Text(text) => json!({ "type": "text", "text": text }),
                                Part::File { url, .. } => {
                                    json!({ "type": "image_url", "image_url": { "url": url } })
                                }
                            })
                            .collect(),
                    ),
                };
                messages.push(json!({ "role": "user", "content": content }));
            }
            Turn::Assistant { text, calls } => {
                let mut message = json!({ "role": "assistant", "content": text });
                if !calls.is_empty() {
                    message["tool_calls"] = calls
                        .iter()
                        .map(|c| {
                            json!({ "id": c.id, "type": "function",
                                    "function": { "name": c.name, "arguments": c.args.to_string() } })
                        })
                        .collect();
                }
                messages.push(message);
            }
            Turn::Tool(outputs) => {
                for out in outputs {
                    messages.push(
                        json!({ "role": "tool", "tool_call_id": out.id, "content": out.output }),
                    );
                }
            }
        }
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model.model_id));
    body.insert("messages".into(), Value::Array(messages));
    if is_openai_reasoning(model) {
        set_opt(
            &mut body,
            "max_completion_tokens",
            request.max_output_tokens,
        );
    } else {
        set_opt(&mut body, "max_tokens", request.max_output_tokens);
        set_opt(&mut body, "temperature", request.temperature);
    }
    if !defs.is_empty() {
        body.insert(
            "tools".into(),
            defs.iter()
                .map(|d| {
                    json!({ "type": "function", "function": {
                        "name": d.name, "description": d.description, "parameters": d.parameters } })
                })
                .collect(),
        );
    }
    match request.tool_choice {
        ToolChoice::Auto if !defs.is_empty() => {
            body.insert("tool_choice".into(), json!("auto"));
        }
        ToolChoice::None => {
            body.insert("tool_choice".into(), json!("none"));
        }
        _ => {}
    }
    match model.transform {
        Transform::None => {}
        Transform::StripReasoning => {
            body.remove("reasoning_effort");
        }
        Transform::Muse => muse_transform(&mut body),
    }
    Value::Object(body)
}

/// `transformMuseRequestBody`.
fn muse_transform(body: &mut Map<String, Value>) {
    let max_tokens = body.get("max_tokens").and_then(Value::as_i64);
    if body.get("reasoning_effort").is_none_or(Value::is_null) {
        let effort = if max_tokens.is_some_and(|m| m < MUSE_LOW_BUDGET_THRESHOLD) {
            "minimal"
        } else {
            "low"
        };
        body.insert("reasoning_effort".into(), json!(effort));
    }
    if let Some(max_tokens) = max_tokens {
        body.insert(
            "max_tokens".into(),
            json!(max_tokens + MUSE_REASONING_HEADROOM_TOKENS),
        );
    }
    if body
        .get("temperature")
        .and_then(Value::as_f64)
        .is_some_and(|t| t > MUSE_MAX_TEMPERATURE)
    {
        body.insert("temperature".into(), json!(MUSE_MAX_TEMPERATURE));
    }
    match body.get("tool_choice").and_then(Value::as_str) {
        Some("none") => {
            body.remove("tools");
            body.remove("tool_choice");
        }
        Some("auto") | None => {}
        Some(_) => {
            body.insert("tool_choice".into(), json!("auto"));
        }
    }
}

/// A `data:<type>;base64,<data>` URL as `(type, data)`.
fn data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    Some((meta.strip_suffix(";base64")?, data))
}

fn anthropic_body(model: &Model, request: &Request, turns: &[Turn], defs: &[ToolDef]) -> Value {
    let messages: Vec<Value> = turns
        .iter()
        .map(|turn| match turn {
            Turn::User(parts) => {
                let content: Vec<Value> = parts
                    .iter()
                    .map(|part| match part {
                        Part::Text(text) => json!({ "type": "text", "text": text }),
                        Part::File { url, media_type } => {
                            let kind = if media_type.starts_with("image/") { "image" } else { "document" };
                            match data_url(url) {
                                Some((media_type, data)) => json!({ "type": kind,
                                    "source": { "type": "base64", "media_type": media_type, "data": data } }),
                                None => json!({ "type": kind, "source": { "type": "url", "url": url } }),
                            }
                        }
                    })
                    .collect();
                json!({ "role": "user", "content": content })
            }
            Turn::Assistant { text, calls } => {
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for c in calls {
                    content.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": c.args }));
                }
                json!({ "role": "assistant", "content": content })
            }
            Turn::Tool(outputs) => json!({ "role": "user", "content": outputs.iter().map(|o| json!({
                "type": "tool_result", "tool_use_id": o.id, "content": o.output, "is_error": o.is_error,
            })).collect::<Vec<_>>() }),
        })
        .collect();
    let mut body = Map::new();
    body.insert("model".into(), json!(model.model_id));
    body.insert(
        "max_tokens".into(),
        json!(
            request
                .max_output_tokens
                .unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS)
        ),
    );
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        body.insert("system".into(), json!(system));
    }
    body.insert("messages".into(), Value::Array(messages));
    set_opt(&mut body, "temperature", request.temperature);
    if !defs.is_empty() {
        body.insert(
            "tools".into(),
            defs.iter()
                .map(|d| json!({ "name": d.name, "description": d.description, "input_schema": d.parameters }))
                .collect(),
        );
        if request.tool_choice == ToolChoice::Auto {
            body.insert("tool_choice".into(), json!({ "type": "auto" }));
        }
    }
    Value::Object(body)
}

/// The OpenAPI subset Gemini takes (no `$schema` / `additionalProperties`).
fn gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "$schema" | "additionalProperties"))
                .map(|(k, v)| (k.clone(), gemini_schema(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(gemini_schema).collect()),
        other => other.clone(),
    }
}

fn gemini_body(request: &Request, turns: &[Turn], defs: &[ToolDef]) -> Value {
    let contents: Vec<Value> = turns
        .iter()
        .map(|turn| match turn {
            Turn::User(parts) => json!({ "role": "user", "parts": parts.iter().map(|part| match part {
                Part::Text(text) => json!({ "text": text }),
                Part::File { url, media_type } => match data_url(url) {
                    Some((mime, data)) => json!({ "inlineData": { "mimeType": mime, "data": data } }),
                    None => json!({ "fileData": { "mimeType": media_type, "fileUri": url } }),
                },
            }).collect::<Vec<_>>() }),
            Turn::Assistant { text, calls } => {
                let mut parts = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for c in calls {
                    parts.push(json!({ "functionCall": { "name": c.name, "args": c.args } }));
                }
                json!({ "role": "model", "parts": parts })
            }
            Turn::Tool(outputs) => json!({ "role": "user", "parts": outputs.iter().map(|o| json!({
                "functionResponse": { "name": o.name, "response": { "name": o.name, "content": o.output } },
            })).collect::<Vec<_>>() }),
        })
        .collect();
    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(contents));
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        body.insert(
            "systemInstruction".into(),
            json!({ "parts": [{ "text": system }] }),
        );
    }
    let mut config = Map::new();
    set_opt(&mut config, "maxOutputTokens", request.max_output_tokens);
    set_opt(&mut config, "temperature", request.temperature);
    body.insert("generationConfig".into(), Value::Object(config));
    if !defs.is_empty() {
        let declarations: Vec<Value> = defs
            .iter()
            .map(|d| {
                let mut decl = json!({ "name": d.name, "description": d.description });
                let has_properties = d
                    .parameters
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|p| !p.is_empty());
                if has_properties {
                    decl["parameters"] = gemini_schema(&d.parameters);
                }
                decl
            })
            .collect();
        body.insert(
            "tools".into(),
            json!([{ "functionDeclarations": declarations }]),
        );
        if request.tool_choice == ToolChoice::Auto {
            body.insert(
                "toolConfig".into(),
                json!({ "functionCallingConfig": { "mode": "AUTO" } }),
            );
        }
    }
    Value::Object(body)
}

/// The text and tool calls of one answer.
pub fn parse_answer(wire: Wire, body: &Value) -> Option<StepAnswer> {
    let mut answer = StepAnswer::default();
    match wire {
        Wire::OpenAi => {
            let message = body.pointer("/choices/0/message")?;
            answer.text = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            for call in message
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let args = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or_else(|| json!({}));
                answer.calls.push(ToolCall {
                    id: call.get("id").and_then(Value::as_str).unwrap_or("").into(),
                    name: call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    args,
                });
            }
        }
        Wire::Anthropic => {
            for block in body.get("content")?.as_array()? {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        answer
                            .text
                            .push_str(block.get("text").and_then(Value::as_str).unwrap_or(""));
                    }
                    Some("tool_use") => answer.calls.push(ToolCall {
                        id: block.get("id").and_then(Value::as_str).unwrap_or("").into(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                        args: block.get("input").cloned().unwrap_or_else(|| json!({})),
                    }),
                    _ => {}
                }
            }
        }
        Wire::Gemini => {
            let parts = body
                .pointer("/candidates/0/content/parts")
                .and_then(Value::as_array);
            for (index, part) in parts.into_iter().flatten().enumerate() {
                if part.get("thought") == Some(&Value::Bool(true)) {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    answer.text.push_str(text);
                }
                if let Some(call) = part.get("functionCall") {
                    answer.calls.push(ToolCall {
                        id: format!("call_{index}"),
                        name: call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .into(),
                        args: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    });
                }
            }
        }
    }
    Some(answer)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`AiHttp`]: answers in order, records each request.

    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeHttp {
        pub answers: Mutex<Vec<(u16, Value)>>,
        pub requests: Mutex<Vec<(String, Value)>>,
    }

    impl FakeHttp {
        pub fn new(answers: Vec<(u16, Value)>) -> FakeHttp {
            FakeHttp {
                answers: Mutex::new(answers),
                requests: Mutex::default(),
            }
        }

        /// An OpenAI chat answer with `text`.
        pub fn openai_text(text: &str) -> (u16, Value) {
            (
                200,
                json!({ "choices": [{ "message": { "role": "assistant", "content": text } }] }),
            )
        }

        pub fn requests(&self) -> Vec<(String, Value)> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl AiHttp for FakeHttp {
        fn post<'a>(
            &'a self,
            url: &'a str,
            _headers: &'a [(String, String)],
            body: &'a Value,
            _timeout: Duration,
        ) -> HttpFuture<'a> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_owned(), body.clone()));
            let mut answers = self.answers.lock().unwrap();
            let next = if answers.is_empty() {
                None
            } else {
                Some(answers.remove(0))
            };
            Box::pin(async move {
                let (status, body) = next.ok_or("no scripted answer")?;
                Ok((status, body.to_string()))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeHttp;
    use super::*;

    fn openai(model_id: &str) -> Model {
        Model {
            provider: "openai".into(),
            wire: Wire::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some("k".into()),
            model_id: model_id.into(),
            transform: Transform::None,
        }
    }

    struct Echo;
    impl Tools for Echo {
        fn defs(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "echo".into(),
                description: "d".into(),
                parameters: json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
            }]
        }
        fn call<'a>(
            &'a self,
            _name: &'a str,
            args: Value,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async move { Ok(format!("got {}", args["q"])) })
        }
    }

    #[tokio::test]
    async fn runs_tools_between_steps_and_separates_step_text() {
        let http = FakeHttp::new(vec![
            (
                200,
                json!({ "choices": [{ "message": { "content": "Looking.", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "echo", "arguments": "{\"q\":\"x\"}" } }
                ] } }] }),
            ),
            FakeHttp::openai_text("Done."),
        ]);
        let mut request = Request::new(Some("sys".into()), vec![Turn::user("hi")]);
        request.max_steps = 5;
        request.tool_choice = ToolChoice::Auto;
        let out = generate(&http, &openai("gpt-4o"), &request, &Echo)
            .await
            .unwrap();
        assert_eq!(out.step_separated_text(), "Looking.\n\nDone.");
        assert_eq!(out.text(), "Looking.Done.");
        assert_eq!(out.final_step_text(), "Done.");
        assert_eq!(out.tool_calls, 1);
        let requests = http.requests();
        assert_eq!(requests[0].0, "https://api.openai.com/v1/chat/completions");
        assert_eq!(requests[0].1["tool_choice"], "auto");
        let second = &requests[1].1["messages"];
        assert_eq!(second[3]["role"], "tool");
        assert_eq!(second[3]["content"], "got \"x\"");
    }

    #[tokio::test]
    async fn one_step_by_default_and_provider_errors_carry_the_status() {
        let http = FakeHttp::new(vec![(402, json!({ "error": { "message": "no credits" } }))]);
        let error = generate(
            &http,
            &openai("gpt-4o"),
            &Request::new(None, vec![Turn::user("hi")]),
            &NoTools,
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, Some(402));
        assert_eq!(error.message, "no credits");
    }

    #[test]
    fn reasoning_models_take_max_completion_tokens_and_no_temperature() {
        let mut request = Request::new(None, vec![Turn::user("hi")]);
        request.max_output_tokens = Some(100);
        request.temperature = Some(0.5);
        let (_, _, body) = build_request(&openai("gpt-5.4-mini"), &request, &request.messages, &[]);
        assert_eq!(body["max_completion_tokens"], 100);
        assert!(body.get("temperature").is_none());
        let (_, _, body) = build_request(&openai("gpt-4o"), &request, &request.messages, &[]);
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn muse_transform_matches_the_original() {
        // packages/ai `transformMuseRequestBody`.
        let mut body =
            json!({ "max_tokens": 250, "temperature": 2, "tool_choice": "required", "tools": [] })
                .as_object()
                .cloned()
                .unwrap();
        muse_transform(&mut body);
        assert_eq!(body["reasoning_effort"], "minimal");
        assert_eq!(body["max_tokens"], 250 + 1024);
        assert_eq!(body["temperature"], 1.0);
        assert_eq!(body["tool_choice"], "auto");
        let mut body = json!({ "tool_choice": "none", "tools": [] })
            .as_object()
            .cloned()
            .unwrap();
        muse_transform(&mut body);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("tools").is_none() && body.get("tool_choice").is_none());
    }

    #[test]
    fn anthropic_and_gemini_wire_formats() {
        let mut request = Request::new(Some("sys".into()), vec![Turn::user("hi")]);
        request.tool_choice = ToolChoice::Auto;
        let claude = Model {
            provider: "claude".into(),
            wire: Wire::Anthropic,
            base_url: "https://api.anthropic.com/v1".into(),
            ..openai("claude-sonnet-4-6")
        };
        let (url, headers, body) =
            build_request(&claude, &request, &request.messages, &Echo.defs());
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        let answer = parse_answer(
            Wire::Anthropic,
            &json!({ "content": [{ "type": "text", "text": "a" },
                                 { "type": "tool_use", "id": "t", "name": "echo", "input": { "q": 1 } }] }),
        )
        .unwrap();
        assert_eq!(answer.text, "a");
        assert_eq!(answer.calls[0].args["q"], 1);

        let gemini = Model {
            provider: "gemini".into(),
            wire: Wire::Gemini,
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            ..openai("gemini-3.5-flash")
        };
        let (url, _, body) = build_request(&gemini, &request, &request.messages, &Echo.defs());
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash:generateContent"
        );
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "echo");
        let answer = parse_answer(
            Wire::Gemini,
            &json!({ "candidates": [{ "content": { "parts": [
                { "text": "thinking", "thought": true }, { "text": "b" },
                { "functionCall": { "name": "echo", "args": {} } }] } }] }),
        )
        .unwrap();
        assert_eq!(answer.text, "b");
        assert_eq!(answer.calls[0].name, "echo");
    }
}
