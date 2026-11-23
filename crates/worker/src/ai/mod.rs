//! Q.W2 slice 1: the LLM runtime of `packages/ai` the worker calls
//! (`factory`, `streamText` with tools, the toolset, the prompt guards, the
//! AI context summary) and the agent runner of
//! `integration/handlers/shared/ai-agent-runner.ts`.

pub mod chunk;
pub mod context;
#[cfg(test)]
mod context_tests;
pub mod conversation_source;
#[cfg(test)]
mod conversation_source_tests;
pub mod extract;
mod office;
pub mod stream;
pub mod text;
pub mod toolset;

/// The LLM client moved to `chatbotx-business` so the API can call it (call summaries).
pub use chatbotx_business::ai::{llm, model};

use std::sync::Arc;

use chatbotx_business::ai::knowledge::{Embedder, HttpEmbedder};
use chatbotx_db::Db;
use chatbotx_db::repo::conversation_ai_context::find_conversation_ai_context_state;
use chatbotx_db::repo::message::{
    FindAiContextMessagesOptions, SinceTimeInput, find_ai_context_messages, get_safe_since_time,
};
use serde_json::Value;

use llm::{AiHttp, Generated, Model, NoTools, Request, ToolChoice, Turn};

/// `MAX_CONVERSATION_HISTORY`.
pub const MAX_CONVERSATION_HISTORY: i64 = 100;
/// `MAX_SUMMARY_LENGTH`.
pub const MAX_SUMMARY_LENGTH: usize = 1000;
/// `AI_MESSAGE_HISTORY_LOOKBACK_MS`.
pub const AI_MESSAGE_HISTORY_LOOKBACK_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// The model transport and the embedder (the provider SDK singletons).
pub struct Ai {
    pub http: Arc<dyn AiHttp>,
    pub embedder: Arc<dyn Embedder>,
}

impl Ai {
    /// The real provider APIs.
    pub fn live() -> Ai {
        Ai {
            http: Arc::new(llm::ReqwestHttp::default()),
            embedder: Arc::new(HttpEmbedder::new()),
        }
    }
}

impl Default for Ai {
    fn default() -> Ai {
        Ai::live()
    }
}

/// `findAIContextMessages` since the AI history marker as `(senderType,
/// text)`, oldest first (`buildAIMessages` / `buildAIAgentMessages` /
/// `getOrInitContext`).
pub async fn history(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    since: Option<i64>,
    in_out_only: bool,
) -> Result<Vec<(String, String)>, String> {
    db.read(move |conn| {
        let state = find_conversation_ai_context_state(conn, conversation_id, workspace_id)
            .map_err(crate::repo_err)?;
        let since = since.or_else(|| {
            state.as_ref().and_then(|s| {
                get_safe_since_time(
                    s.last_activity_at.map(SinceTimeInput::Date),
                    Some(AI_MESSAGE_HISTORY_LOOKBACK_MS),
                )
            })
        });
        let options = FindAiContextMessagesOptions {
            conversation_id,
            limit: MAX_CONVERSATION_HISTORY,
            marker_message_id: state.and_then(|s| s.ai_context_last_message_id),
            message_types: if in_out_only {
                vec!["incoming".into(), "outgoing".into()]
            } else {
                Vec::new()
            },
            since_time: Some(since.unwrap_or(0)),
            text_not_null: in_out_only,
            workspace_id,
        };
        let rows = find_ai_context_messages(conn, &options).map_err(crate::repo_err)?;
        Ok(rows
            .into_iter()
            .filter(|m| !in_out_only || matches!(m.message_type.as_str(), "incoming" | "outgoing"))
            .filter_map(|m| m.text.filter(|t| !t.is_empty()).map(|t| (m.sender_type, t)))
            .collect())
    })
    .await
    .map_err(|e| e.to_string())
}

/// A `(senderType, text)` history as model turns (contact = user).
pub fn history_turns(history: &[(String, String)]) -> Vec<Turn> {
    history
        .iter()
        .map(|(sender, text)| {
            if sender == "contact" {
                Turn::user(text.clone())
            } else {
                Turn::assistant(text.clone())
            }
        })
        .collect()
}

/// `aiContextStore.delete`.
pub async fn delete_context(db: &Arc<Db>, conversation_id: i64) {
    context::delete(db, conversation_id).await;
}

/// `summarizeConversation`'s model: the caller's preferred auto-reply
/// integration, then the first auto-reply provider (with its default model),
/// then the oldest auto-reply OpenAI-compatible integration.
async fn summarizer_model(db: &Db, workspace_id: i64, preferred: &Value) -> Option<Model> {
    let preferred = preferred.clone();
    db.read(move |conn| {
        for config in preferred.as_array().into_iter().flatten() {
            let model_id = config.get("model").and_then(Value::as_str).unwrap_or("");
            if config.get("kind").and_then(Value::as_str) == Some("openaiCompatible") {
                let id = config.get("integrationId").and_then(crate::id_of);
                if let Some(row) = id
                    .map(|id| model::compatible_row(conn, workspace_id, Some(id), Some(true)))
                    .transpose()?
                    .flatten()
                    .filter(|r| r.enabled)
                {
                    return Ok(model::compatible_model(&row, model_id).ok());
                }
                continue;
            }
            let provider = config.get("provider").and_then(Value::as_str).unwrap_or("");
            if let Some(auth) = model::native_auth(conn, workspace_id, provider, Some(true))? {
                return Ok(model::native_model(provider, &auth, model_id).ok());
            }
        }
        for provider in model::providers() {
            if let Some(auth) = model::native_auth(conn, workspace_id, provider, Some(true))? {
                let model_id = model::default_model(provider).unwrap_or_default();
                return Ok(model::native_model(provider, &auth, model_id).ok());
            }
        }
        Ok(model::compatible_row(conn, workspace_id, None, Some(true))?
            .and_then(|row| model::compatible_model(&row, &row.default_model.clone()).ok()))
    })
    .await
    .ok()
    .flatten()
}

/// `summarizeConversation` of `(role, content)` lines: a failure keeps the
/// existing summary.
pub async fn summarize(
    deps: &crate::Deps,
    workspace_id: i64,
    history: &[(String, String)],
    existing: Option<&str>,
    preferred: &Value,
) -> String {
    let existing = existing.unwrap_or("").to_owned();
    let Some(model) = summarizer_model(&deps.db, workspace_id, preferred).await else {
        tracing::warn!(
            workspace_id,
            "[summarizer] No active AI integration found for summarization"
        );
        return existing;
    };
    let lines = history
        .iter()
        .map(|(role, text)| format!("{role}: {text}"))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = if existing.is_empty() {
        [
            text::summarizer::CONVERSATION_HISTORY.to_owned(),
            lines,
            text::summarizer::NEW_SUMMARY_PROMPT.to_owned(),
        ]
        .join("\n\n")
    } else {
        [
            text::summarizer::previous_summary(&existing),
            text::summarizer::LATEST_MESSAGES.to_owned(),
            lines,
            text::summarizer::UPDATE_SUMMARY_PROMPT.to_owned(),
        ]
        .join("\n\n")
    };
    let ask = |prompt: String, max: i64, temperature: f64| {
        let mut request = Request::new(None, vec![Turn::user(prompt)]);
        request.max_output_tokens = Some(max);
        request.temperature = Some(temperature);
        request
    };
    let http = &*deps.ai.http;
    let result: Result<String, llm::LlmError> = async {
        let first = llm::generate(http, &model, &ask(prompt, 500, 0.3), &NoTools).await?;
        let mut summary = first.text().trim().to_owned();
        if summary.chars().count() > MAX_SUMMARY_LENGTH {
            let shorter = llm::generate(
                http,
                &model,
                &ask(text::summarizer::shorten_prompt(&summary), 400, 0.2),
                &NoTools,
            )
            .await?;
            summary = shorter.text().trim().to_owned();
        }
        Ok(summary)
    }
    .await;
    result.unwrap_or_else(|error| {
        tracing::error!(error = %error, workspace_id, provider = %model.provider, "[summarizer] Failed to generate summary");
        existing
    })
}

/// `ReplyByAIExecutionResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentReply {
    pub provider: String,
    pub model_id: String,
    pub used_fallback_text: bool,
    pub full_text: String,
}

/// An `AIAgent` row as the runner reads it.
#[derive(Debug, Clone)]
pub struct Agent {
    pub workspace_id: i64,
    pub prompt: Option<String>,
    pub models: Value,
    pub tools: Vec<String>,
    pub temperature: f64,
    pub max_output_tokens: i64,
}

/// `runAIAgentRunner`: each configured model in order until one answers.
/// `preferred` = one model config to run alone (`preferredModel`),
/// `provider` = only the agent's models of that native provider.
pub async fn run_agent(
    deps: &crate::Deps,
    agent: &Agent,
    messages: Vec<Turn>,
    summary: &str,
    prompt: &str,
    preferred: Option<Value>,
    provider: Option<&str>,
) -> Option<AgentReply> {
    let configs: Vec<Value> = match preferred {
        Some(one) => vec![one],
        None => agent
            .models
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| {
                provider.is_none_or(|p| c.get("provider").and_then(Value::as_str) == Some(p))
            })
            .collect(),
    };
    let tools = toolset::load(
        &deps.db,
        &deps.ai.embedder,
        agent.workspace_id,
        &agent.tools,
    )
    .await;
    let complete = if summary.is_empty() {
        prompt.to_owned()
    } else {
        format!("Conversation Context: {summary}\n\n{prompt}")
    };
    let system = text::guarded_prompt(&complete, &tools);
    for config in configs {
        let compatible = config.get("kind").and_then(Value::as_str) == Some("openaiCompatible");
        let provider_name = if compatible {
            "openaiCompatible".to_owned()
        } else {
            config
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let model_id = config
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let model = match agent_model(&deps.db, agent.workspace_id, &config).await {
            Ok(Some(model)) => model,
            Ok(None) => continue,
            Err(err) => {
                tracing::error!(err = %err, provider = %provider_name, workspace_id = agent.workspace_id, "[ai-agent-runner] runAIReplyInternal failed");
                continue;
            }
        };
        let mut request = Request::new(Some(system.clone()), messages.clone());
        request.max_output_tokens = Some(agent.max_output_tokens);
        request.temperature = Some(agent.temperature);
        request.max_steps = 5;
        request.tool_choice = if tools.is_empty() {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let started = std::time::Instant::now();
        let result: Result<Generated, llm::LlmError> =
            llm::generate(&*deps.ai.http, &model, &request, &tools).await;
        tracing::info!(provider = %provider_name, duration_ms = started.elapsed().as_millis() as u64, responded = result.as_ref().is_ok_and(|g| !g.step_separated_text().is_empty()), "[ai-agent-runner] fallback chain provider attempt");
        let generated = match result {
            Ok(generated) => generated,
            Err(err) => {
                tracing::error!(err = %err, provider = %provider_name, workspace_id = agent.workspace_id, "[ai-agent-runner] runAIReplyInternal failed");
                continue;
            }
        };
        let full_text = generated.step_separated_text();
        if !full_text.is_empty() {
            return Some(AgentReply {
                provider: provider_name,
                model_id,
                used_fallback_text: false,
                full_text,
            });
        }
        if generated.tool_calls > 0 {
            return Some(AgentReply {
                provider: provider_name,
                model_id,
                used_fallback_text: true,
                full_text: text::FALLBACK_LOOKUP.to_owned(),
            });
        }
    }
    None
}

/// `createAgentModel`: `None` for a missing (or disabled) integration.
pub(crate) async fn agent_model(
    db: &Db,
    workspace_id: i64,
    config: &Value,
) -> Result<Option<Model>, String> {
    model_of(db, workspace_id, config, None).await
}

/// `createReplyModel`: as [`agent_model`], only from an auto-reply integration.
pub(crate) async fn reply_model(
    db: &Db,
    workspace_id: i64,
    config: &Value,
) -> Result<Option<Model>, String> {
    model_of(db, workspace_id, config, Some(true)).await
}

async fn model_of(
    db: &Db,
    workspace_id: i64,
    config: &Value,
    auto_reply: Option<bool>,
) -> Result<Option<Model>, String> {
    let config = config.clone();
    db.read(move |conn| {
        let model_id = config.get("model").and_then(Value::as_str).unwrap_or("");
        if config.get("kind").and_then(Value::as_str) == Some("openaiCompatible") {
            let row = match config.get("integrationId").and_then(crate::id_of) {
                Some(id) => model::compatible_row(conn, workspace_id, Some(id), auto_reply)?,
                None => None,
            };
            return Ok(row
                .filter(|r| r.enabled)
                .map(|row| model::compatible_model(&row, model_id)));
        }
        let provider = config.get("provider").and_then(Value::as_str).unwrap_or("");
        Ok(
            model::native_auth(conn, workspace_id, provider, auto_reply)?
                .map(|auth| model::native_model(provider, &auth, model_id)),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .transpose()
}
