//! Q.W2 slice 1: the AI flow steps that run in the integration worker
//! (`generate-text`, `generate-text-agent`, `delete-message-history`,
//! `analyze-image`, `extract-data`).
//!
//! A provider failure of `aiGenerateText` is written to `ErrorLog`
//! (`logStepProviderError`) through the error-log bus.

use chatbotx_business::contact_custom_field::{self, SetValueByKeyInput};
use chatbotx_db::repo::message::{SinceTimeInput, get_safe_since_time};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::{StepCtx, step_error};
use crate::ai::llm::{self, Part, Request, ToolChoice, Turn};
use crate::ai::{self, Agent, model, text, toolset};
use crate::flow::StepStatus;
use crate::{db_fail, id_of, str_of};

const ERROR_INSUFFICIENT_CREDITS: &str =
    "AI provider has insufficient credits. Please check your billing settings.";
const OPENAI_COMPATIBLE_MISSING: &str = "OpenAI-compatible integration is missing or disabled";

pub(crate) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    Ok(Some(match step_type {
        "aiGenerateText" => generate_text(ctx).await?,
        "aiGenerateTextAgent" => generate_text_agent(ctx).await?,
        "aiDeleteMessageHistory" => delete_message_history(ctx).await?,
        "aiAnalyzeImage" => analyze_image(ctx).await?,
        "aiExtractData" => extract_data(ctx).await?,
        _ => return Ok(None),
    }))
}

/// `buildAIMessages` / `buildAIAgentMessages`: the remembered in / out
/// history (since the AI marker), then the step's message.
async fn step_messages(
    ctx: &StepCtx<'_, '_>,
    remember: bool,
    message: &str,
) -> Result<Vec<Turn>, String> {
    let mut turns = Vec::new();
    if remember {
        let run = ctx.run;
        let since = get_safe_since_time(
            run.contact_inbox
                .last_message_at
                .or(run.conversation.last_activity_at)
                .map(SinceTimeInput::Date),
            Some(ai::AI_MESSAGE_HISTORY_LOOKBACK_MS),
        )
        .unwrap_or(0);
        let history = ai::history(
            &run.deps.db,
            ctx.workspace_id(),
            run.conversation.id,
            Some(since),
            true,
        )
        .await?;
        turns = ai::history_turns(&history);
    }
    if !message.is_empty() {
        turns.push(Turn::user(message));
    }
    Ok(turns)
}

/// `saveResultToCustomField`: a field that no longer exists is skipped.
async fn save_result(ctx: &StepCtx<'_, '_>, field: &str, full_text: String) -> Result<(), String> {
    let input = SetValueByKeyInput {
        workspace_id: ctx.workspace_id(),
        contact_id: ctx.contact_id(),
        keyword: field.to_owned(),
        value: full_text,
        allow_bot_fields: true,
        ..SetValueByKeyInput::default()
    };
    match contact_custom_field::set_value_by_key(&ctx.run.deps.db, input).await {
        Ok(_) => Ok(()),
        Err(err) if err.code() == Some("notFound") => {
            tracing::warn!(
                workspace_id = ctx.workspace_id(),
                contact_id = ctx.contact_id(),
                custom_field_id = field,
                "saveResultToCustomField target no longer exists; skipping"
            );
            Ok(())
        }
        Err(err) => Err(err.message()),
    }
}

/// `handleAIGenerateText`.
async fn generate_text(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let resolved = ctx
        .resolve_variables(
            &json!({ "system": ctx.str("system"), "text": ctx.str("text") }),
            false,
        )
        .await;
    let resolved = match resolved {
        Ok(value) => value,
        Err(err) => return Ok(step_error(ctx, &err)),
    };
    let system = str_of(&resolved, "system");
    let message = str_of(&resolved, "text").unwrap_or("");
    let remember = ctx.step.get("remember") == Some(&Value::Bool(true));
    let turns = match step_messages(ctx, remember, message).await {
        Ok(turns) => turns,
        Err(err) => return Ok(step_error(ctx, &err)),
    };
    let provider = ctx.str("provider").unwrap_or("");
    let model = match model::resolve_flow_model(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        provider,
        ctx.str("integrationId"),
        ctx.str("model").unwrap_or(""),
    )
    .await
    {
        Ok(model) => model,
        Err(model::NoModel::Missing(message)) => {
            tracing::warn!(
                workspace_id = ctx.workspace_id(),
                step_id = ctx.str("id"),
                provider,
                reason = %message,
                "Failed to resolve AI model for generate text step"
            );
            return Ok(step_error(ctx, &message));
        }
        Err(model::NoModel::Failed(message)) => return Ok(step_error(ctx, &message)),
    };
    let tool_ids = string_list(ctx.step.get("tools"));
    let deps = ctx.run.deps;
    let tools = toolset::load(&deps.db, &deps.ai.embedder, ctx.workspace_id(), &tool_ids).await;
    let mut request = Request::new(
        Some(text::with_channel_length_instruction(
            system,
            Some(ctx.run.contact_inbox.channel.as_str()),
        )),
        turns,
    );
    request.max_output_tokens = ctx.step.get("maxOutputTokens").and_then(Value::as_i64);
    request.temperature = ctx.step.get("temperature").and_then(Value::as_f64);
    request.tool_choice = if tools.is_empty() {
        ToolChoice::Unset
    } else {
        ToolChoice::Auto
    };
    match llm::generate(&*deps.ai.http, &model, &request, &tools).await {
        Ok(generated) => {
            let field = ctx.str("outputFieldId").unwrap_or("");
            match save_result(ctx, field, generated.text()).await {
                Ok(()) => Ok(StepStatus::Success),
                Err(err) => Ok(step_error(ctx, &err)),
            }
        }
        Err(err) if err.status == Some(402) => {
            log_step_provider_error(ctx, &err).await;
            tracing::error!(err = %err, "AI provider insufficient credits");
            Ok(step_error(ctx, ERROR_INSUFFICIENT_CREDITS))
        }
        Err(err) => {
            log_step_provider_error(ctx, &err).await;
            tracing::error!(err = %err, "An error occurred while generating text");
            Ok(step_error(ctx, &err.message))
        }
    }
}

/// `logStepProviderError(aiErrorLogProvider(step.provider), ...)`.
async fn log_step_provider_error(ctx: &StepCtx<'_, '_>, err: &llm::LlmError) {
    let provider = crate::events::ai_error_log_provider(ctx.str("provider").unwrap_or("openai"));
    let input = crate::events::ProviderError {
        provider: provider.to_owned(),
        workspace_id: ctx.workspace_id().to_string(),
        contact_id: Some(ctx.contact_id().to_string()),
        source_id: Some(ctx.run.contact_inbox.source_id.clone()),
        error: json!({ "message": err.message, "statusCode": err.status }),
        ..Default::default()
    };
    crate::events::log_provider_errors(ctx.run.deps.events.as_ref(), &[input]).await;
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// `aiAgentService.findBy({ id, workspaceId })`.
async fn load_agent(ctx: &StepCtx<'_, '_>, id: i64) -> Result<Option<Agent>, JobError> {
    let ws = ctx.workspace_id();
    ctx.run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "prompt", "models", "tools", "temperature", "maxOutputTokens" FROM "AIAgent" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| {
                        let json = |i| -> chatbotx_db::rusqlite::Result<Value> {
                            Ok(r.get::<_, Option<String>>(i)?
                                .and_then(|t| serde_json::from_str(&t).ok())
                                .unwrap_or(Value::Null))
                        };
                        Ok(Agent {
                            workspace_id: ws,
                            prompt: r.get(0)?,
                            models: json(1)?,
                            tools: string_list(Some(&json(2)?)),
                            temperature: r.get(3)?,
                            max_output_tokens: r.get(4)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// `handleAIGenerateTextAgent`.
async fn generate_text_agent(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let Some(agent_id) = ctx.step.get("aiAgentId").and_then(id_of) else {
        return Ok(step_error(ctx, "AI Agent not found"));
    };
    let Some(agent) = load_agent(ctx, agent_id).await? else {
        tracing::error!(
            workspace_id = ctx.workspace_id(),
            ai_agent_id = agent_id,
            "[ai-generate-text-agent] AI Agent not found"
        );
        return Ok(step_error(ctx, "AI Agent not found"));
    };
    let deps = ctx.run.deps;
    let summary = ai::context_summary(
        deps,
        ctx.workspace_id(),
        ctx.run.conversation.id,
        &agent.models,
    )
    .await;
    let message = match ctx
        .resolve_variables(&json!(ctx.str("message").unwrap_or("")), false)
        .await
    {
        Ok(Value::String(message)) => message,
        Ok(_) => String::new(),
        Err(err) => return Ok(step_error(ctx, &err)),
    };
    let remember = ctx.step.get("rememberConversation") == Some(&Value::Bool(true));
    let turns = match step_messages(ctx, remember, &message).await {
        Ok(turns) => turns,
        Err(err) => return Ok(step_error(ctx, &err)),
    };
    let provider = ctx.str("provider").unwrap_or("");
    // `resolvePreferredModel`.
    let preferred = (provider == "openaiCompatible")
        .then(|| {
            let integration = ctx.str("integrationId").map(str::trim).unwrap_or("");
            let model = ctx.str("model").map(str::trim).unwrap_or("");
            (!integration.is_empty() && !model.is_empty()).then(|| {
                json!({ "kind": "openaiCompatible", "integrationId": ctx.str("integrationId"),
                        "model": ctx.str("model") })
            })
        })
        .flatten();
    let native = (provider != "openaiCompatible" && model::providers().any(|p| p == provider))
        .then_some(provider);
    if preferred.is_none() && native.is_none() {
        tracing::error!(
            workspace_id = ctx.workspace_id(),
            step_id = ctx.str("id"),
            provider,
            "[ai-generate-text-agent] Unsupported AI Agent provider"
        );
        return Ok(step_error(ctx, "Unsupported AI Agent provider"));
    }
    // `contactVariableService.replaceAll` of the agent prompt (no spintax).
    let prompt = match agent.prompt.as_deref().filter(|p| !p.is_empty()) {
        None => String::new(),
        Some(prompt) => match ctx.resolve_variables(&json!(prompt), false).await {
            Ok(Value::String(prompt)) => prompt,
            _ => prompt.to_owned(),
        },
    };
    let is_preferred = preferred.is_some();
    let reply = ai::run_agent(
        deps,
        &agent,
        turns,
        &summary,
        &prompt,
        preferred,
        if is_preferred { None } else { native },
    )
    .await;
    let Some(reply) = reply else {
        if provider == "openaiCompatible" {
            return Ok(step_error(ctx, OPENAI_COMPATIBLE_MISSING));
        }
        return Ok(StepStatus::Success);
    };
    if let Some(field) = ctx.str("outputFieldId").filter(|f| !f.is_empty())
        && !reply.full_text.is_empty()
        && let Err(err) = save_result(ctx, field, reply.full_text).await
    {
        return Ok(step_error(ctx, &err));
    }
    Ok(StepStatus::Success)
}

/// `resetAIHistoryMarker`: the marker moves to the conversation's last
/// message (narrow window since `lastMessageAt`, then the whole history)
/// after the cached context is deleted.
async fn reset_marker(
    ctx: &StepCtx<'_, '_>,
    conversation_id: i64,
    workspace_id: i64,
) -> Result<(), String> {
    let deps = ctx.run.deps;
    let last_message_at = ctx.run.contact_inbox.last_message_at;
    let narrow = get_safe_since_time(last_message_at.map(SinceTimeInput::Date), None).unwrap_or(0);
    let last = deps
        .db
        .read(move |conn| {
            let find = |since: i64| {
                conn.query_row(
                    r#"SELECT "id" FROM "Message" WHERE "conversationId" = ?1 AND "workspaceId" = ?2 AND "createdAt" >= ?3 ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#,
                    params![conversation_id, workspace_id, since],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
            };
            let mut last = find(narrow)?;
            if last.is_none() && last_message_at.is_some() {
                last = find(0)?;
            }
            Ok(last)
        })
        .await
        .map_err(|e| e.to_string())?;
    ai::delete_context(&deps.db, conversation_id).await;
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Conversation" SET "aiContextLastMessageId" = ?1 WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                params![last, conversation_id, workspace_id],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

/// `handleAIDeleteMessageHistory`: the conversation, and from a comment
/// conversation also the contact's DM conversation.
// ponytail: no per-conversation lock (`aiContextStore.runExclusive`); the
// SQLite writer serializes the marker writes.
async fn delete_message_history(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let conversation = &ctx.run.conversation;
    let result = async {
        reset_marker(ctx, conversation.id, conversation.workspace_id).await?;
        if conversation.source_id.is_none() {
            return Ok(());
        }
        let (ws, contact) = (conversation.workspace_id, conversation.contact_id);
        let dm = ctx
            .run
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL LIMIT 1"#,
                        params![ws, contact],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?)
            })
            .await
            .map_err(|e| e.to_string())?;
        match dm {
            Some(dm) if dm != conversation.id => reset_marker(ctx, dm, ws).await,
            _ => Ok(()),
        }
    }
    .await;
    match result {
        Ok(()) => Ok(StepStatus::Success),
        Err(err) => {
            tracing::error!(err = %err, workspace_id = conversation.workspace_id, conversation_id = conversation.id, action = "aiDeleteMessageHistory", "[ai-delete-message-history] Step failed");
            Ok(step_error(ctx, &err))
        }
    }
}

/// `resolveFlowAIModel` for a step; `Err` is the step result (logged).
async fn step_model(ctx: &StepCtx<'_, '_>, label: &str) -> Result<llm::Model, StepStatus> {
    let provider = ctx.str("provider").unwrap_or("");
    let integration = (provider == "openaiCompatible")
        .then(|| ctx.str("integrationId"))
        .flatten();
    let resolved = model::resolve_flow_model(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        provider,
        integration,
        ctx.str("model").unwrap_or(""),
    )
    .await;
    match resolved {
        Ok(model) => Ok(model),
        Err(model::NoModel::Missing(message) | model::NoModel::Failed(message)) => {
            tracing::warn!(
                workspace_id = ctx.workspace_id(),
                conversation_id = ctx.run.conversation.id,
                step_id = ctx.str("id"),
                provider,
                reason = %message,
                "[{label}] Failed to resolve AI model"
            );
            Err(step_error(ctx, &message))
        }
    }
}

/// `isImageUrl`: an http(s) URL whose path ends in an image extension.
fn is_image_url(url: &str) -> bool {
    let s = url.trim().to_ascii_lowercase();
    let Some(rest) = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
    else {
        return false;
    };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let path = path.find('/').map_or("", |i| &path[i..]);
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// `handleAIAnalyzeImage`: the prompt and the image of a custom field to
/// the model; the answer is saved to `outputFieldId`.
async fn analyze_image(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let model = match step_model(ctx, "ai-analyze-image").await {
        Ok(model) => model,
        Err(status) => return Ok(status),
    };
    let db = &ctx.run.deps.db;
    let image = crate::condition::find_custom_field_value(
        db,
        &ctx.contact_id().to_string(),
        ctx.str("inputFieldId").unwrap_or(""),
    )
    .await
    .map_err(db_fail)?
    .filter(|url| !url.is_empty() && is_image_url(url));
    let Some(image) = image else {
        return Ok(step_error(ctx, "Invalid image URL provided"));
    };
    let prompt = match ctx
        .resolve_variables(ctx.step.get("prompt").unwrap_or(&Value::Null), false)
        .await
    {
        Ok(Value::String(prompt)) => prompt,
        Ok(other) => js_text(&other),
        Err(err) => return Ok(step_error(ctx, &err)),
    };
    let mut request = Request::new(
        Some(text::with_channel_length_instruction(
            None,
            Some(ctx.run.contact_inbox.channel.as_str()),
        )),
        vec![Turn::User(vec![
            Part::Text(prompt),
            Part::File {
                url: image,
                media_type: "image/*".into(),
            },
        ])],
    );
    request.max_output_tokens = ctx.step.get("maxOutputTokens").and_then(Value::as_i64);
    request.temperature = ctx.step.get("temperature").and_then(Value::as_f64);
    let generated = llm::generate(&*ctx.run.deps.ai.http, &model, &request, &llm::NoTools).await;
    let result = match generated {
        Ok(generated) => match ctx.str("outputFieldId").filter(|f| !f.is_empty()) {
            Some(field) => save_result(ctx, field, generated.text()).await,
            None => Ok(()),
        },
        Err(err) => {
            log_step_provider_error(ctx, &err).await;
            Err(err.message)
        }
    };
    match result {
        Ok(()) => Ok(StepStatus::Success),
        Err(message) => {
            tracing::error!(err = %message, "[ai-analyze-image] Step failed");
            Ok(step_error(ctx, &message))
        }
    }
}

/// `String(value)` for a resolved prompt that is not a string.
fn js_text(value: &Value) -> String {
    chatbotx_core::js::string_of(value)
}

/// `stringifyFieldValue`.
fn stringify_field_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `getInputValue` of `handleAIExtractData`: the typed text (variables
/// resolved, the file attribute replaced) or the input field's value.
async fn extract_input(ctx: &StepCtx<'_, '_>, input_type: &str) -> Result<Option<String>, String> {
    let input_field = ctx.str("inputFieldId").unwrap_or("");
    if input_type == "text" {
        let resolved = ctx
            .resolve_variables(&json!(input_field.trim()), false)
            .await?;
        let mut text = resolved
            .as_str()
            .map_or_else(|| js_text(&resolved), str::to_owned);
        if let Some(file) = ctx.step.get("file").filter(|f| f.is_object()) {
            let attribute = file.get("attribute").map(js_text).unwrap_or_default();
            let value = file.get("value").map(js_text).unwrap_or_default();
            // `String.replace` with a string pattern: the first match only.
            text = text.replacen(&attribute, &value, 1);
        }
        return Ok((!text.is_empty()).then_some(text));
    }
    let value = crate::condition::find_custom_field_value(
        &ctx.run.deps.db,
        &ctx.contact_id().to_string(),
        input_field,
    )
    .await?;
    Ok(value.filter(|v| !chatbotx_core::js::trim(v).is_empty()))
}

/// `handleAIExtractData`: the model fills one nullable string per extract
/// field; each non-null value is saved to its custom field.
async fn extract_data(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let fields: Vec<(String, String, Option<String>)> = ctx
        .step
        .get("extractFields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            let text = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_owned);
            (
                text("key").unwrap_or_default(),
                text("customFieldId").unwrap_or_default(),
                text("description").filter(|d| !d.is_empty()),
            )
        })
        .collect();
    if fields.is_empty() {
        tracing::debug!("No extract fields configured");
        return Ok(StepStatus::Skip);
    }
    let input_type = ctx.str("inputType").unwrap_or("text").to_owned();
    let outcome = async {
        let Some(input) = extract_input(ctx, &input_type).await? else {
            tracing::debug!("Input field is empty");
            return Ok(Some(StepStatus::Skip));
        };
        let model = match step_model(ctx, "ai-extract-data").await {
            Ok(model) => model,
            Err(status) => return Ok(Some(status)),
        };
        let description: Vec<String> = fields
            .iter()
            .map(|(key, _, d)| d.as_ref().map_or_else(|| format!("- {key}"), |d| format!("- {key}: {d}")))
            .collect();
        let system = format!(
            "You are a data extraction expert. Extract the following information from the provided {input_type}.\nFields to extract:\n{}",
            description.join("\n")
        );
        let mut parts = vec![match input_type.as_str() {
            "text" => Part::Text(input),
            "image" => Part::File { url: input, media_type: "image/*".into() },
            _ => Part::File { url: input, media_type: "application/pdf".into() },
        }];
        parts.push(Part::Text(format!("Please extract data from this {input_type}")));
        let mut properties = serde_json::Map::new();
        for (key, _, _) in &fields {
            properties.insert(
                key.clone(),
                json!({ "anyOf": [{ "type": "string" }, { "type": "null" }], "description": format!("The value for {key}") }),
            );
        }
        let keys: Vec<&String> = properties.keys().collect();
        let schema = json!({
            "type": "object",
            "properties": properties,
            "required": keys,
            "additionalProperties": false,
        });
        let request = Request::new(Some(system), vec![Turn::User(parts)]);
        let data = match llm::generate_object(&*ctx.run.deps.ai.http, &model, &request, schema).await {
            Ok(data) => data,
            Err(err) => {
                log_step_provider_error(ctx, &err).await;
                if err.status == Some(402) {
                    tracing::error!(err = %err, "AI provider insufficient credits");
                    return Ok(Some(step_error(ctx, ERROR_INSUFFICIENT_CREDITS)));
                }
                return Err(err.message);
            }
        };
        for (key, field, _) in &fields {
            match data.get(key) {
                None | Some(Value::Null) => {}
                Some(value) => save_result(ctx, field, stringify_field_value(value)).await?,
            }
        }
        Ok::<_, String>(None)
    }
    .await;
    match outcome {
        Ok(Some(status)) => Ok(status),
        Ok(None) => Ok(StepStatus::Success),
        Err(message) => {
            tracing::error!(
                workspace_id = ctx.workspace_id(),
                conversation_id = ctx.run.conversation.id,
                step_id = ctx.str("id"),
                err = %message,
                reason = "ai_generation_failed",
                "Error in handleAIExtractData"
            );
            send_text_and_wait(ctx, "Error extracting data").await?;
            Ok(step_error(ctx, &message))
        }
    }
}

/// `sendMessageWithRender(conversationId, text, undefined, { metadata,
/// isBulkBroadcast })` + `waitForChatJobCompletion`.
async fn send_text_and_wait(ctx: &StepCtx<'_, '_>, text: &str) -> Result<(), JobError> {
    let run = ctx.run;
    let mut data = serde_json::Map::new();
    data.insert(
        "conversationId".into(),
        json!(run.conversation.id.to_string()),
    );
    data.insert("text".into(), json!(text));
    if let Some(metadata) = run.metadata() {
        data.insert("metadata".into(), metadata.clone());
    }
    if let Some(bulk) = run.is_bulk_broadcast {
        data.insert("isBulkBroadcast".into(), json!(bulk));
    }
    let added = run
        .deps
        .jobs
        .add(
            chatbotx_jobs::QueueName::Chat,
            chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION,
            json!({ "type": chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION, "data": Value::Object(data) }),
            chatbotx_jobs::JobOptions::default(),
        )
        .await
        .map_err(db_fail)?;
    let target = chatbotx_jobs::WaitTarget {
        queue: Some(chatbotx_jobs::QueueName::Chat),
        job_id: added.job_id,
        ..chatbotx_jobs::WaitTarget::default()
    };
    let context = chatbotx_jobs::chat_wait::ChatWaitContext {
        conversation_id: Some(run.conversation.id.to_string()),
        step_id: None,
    };
    chatbotx_jobs::chat_wait::wait_for_chat_job_completion(
        run.deps.waiter.as_ref(),
        Some(&target),
        &context,
    )
    .await;
    Ok(())
}
