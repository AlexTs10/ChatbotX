//! The automated response after the keywords (`automated-response/index.ts`,
//! `product-search.ts`, `global-router.ts`, `replies.ts`, `default-reply.ts`):
//! Search Products, the Global Router, the default AI agent's reply with its
//! system tools, and the throttled default reply flow.
//!
//! ponytail: no overall `aiTotal` abort across the provider attempts (each
//! HTTP call has its own timeout).

use std::sync::{Arc, LazyLock, Mutex};

use chatbotx_business::decision::{self, EvalDeps, InboundEval, Router};
use chatbotx_business::events::Event;
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_core::decision_config::GlobalRouterDestination;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::repo::message::{
    FindAiContextMessagesOptions, FindTriggerMessageOptions, MessageWithAttachments,
    SinceTimeInput, find_ai_context_messages, find_trigger_message, get_safe_since_time,
};
use chatbotx_db::repo::models::{AiAgent, Attachment, ContactInbox, Conversation};
use chatbotx_db::repo::{find_one, iso, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use chatbotx_kv::lock::LockMap;
use serde_json::{Value, json};

use crate::ai::context::{self, AiMessage};
use crate::ai::llm::{self, Request, ToolChoice, Turn};
use crate::ai::{self, stream, text, toolset};
use crate::{Deps, db_fail, fail};

/// `TRIGGER_MESSAGE_LOOKBACK_MS`.
const TRIGGER_MESSAGE_LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const IMAGE_MIME_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
    "image/gif",
];
const DOCUMENT_MIME_TYPES: [&str; 3] = [
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/msword",
];
const HANDOFF_POLICY: &str = "HANDOFF POLICY (REQUIRED):
- Only call 'connect_user_to_human' if the user explicitly asks for a human agent OR if you cannot resolve the issue after 2-3 attempts.
- If the user's intent is ambiguous, ask for confirmation (e.g., 'Would you like to speak with a human agent?') before calling the tool.
- Do NOT call this tool for greetings, small talk, or issues that can be resolved using other available tools.
- After calling the tool, inform the user that they are being connected to a human agent.";
const WEB_SEARCH_UNAVAILABLE: &str = "WEB SEARCH AVAILABILITY (REQUIRED):
- Web search is configured for this agent but is unavailable for the current provider or domain policy.
- Do not claim that you searched, browsed, or looked up live web information.
- Answer only from the conversation and available tools, or ask the user for clarification if live information is required.";
const HANDOFF_REPLY: &str =
    "I'm connecting you to a human agent who can better assist you. Please stay on the line.";

/// `distributedLock` of `global-router:*` (one process).
static ROUTER_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);
/// `distributedLock` of `decision:eval:*` for the router profile.
static EVAL_LOCKS: LazyLock<LockMap> = LazyLock::new(LockMap::new);

/// `AutomatedResponseProcessResult.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeywordStatus {
    Handled,
    NoEligibleMessage,
    NoMatch,
}

fn normalize_mime(value: &str) -> String {
    value.split(';').next().unwrap_or("").trim().to_lowercase()
}

fn is_image(attachment: &Attachment) -> bool {
    IMAGE_MIME_TYPES.contains(&normalize_mime(&attachment.mime_type).as_str())
}

fn is_document(attachment: &Attachment) -> bool {
    DOCUMENT_MIME_TYPES.contains(&normalize_mime(&attachment.mime_type).as_str())
}

/// The workspace columns the automated response reads.
struct WorkspaceReply {
    default_reply: Option<i64>,
    frequency: &'static str,
    timezone: String,
}

/// `defaultReplyFrequencies.safeParse(..) ?? allTime`.
fn frequency_of(value: &str) -> &'static str {
    match value {
        "oncePerHour" => "oncePerHour",
        "oncePerDay" => "oncePerDay",
        _ => "allTime",
    }
}

/// `DEFAULT_REPLY_FREQUENCY_WINDOW_SECONDS`.
fn frequency_window_seconds(frequency: &str) -> i64 {
    match frequency {
        "oncePerHour" => 3600,
        "oncePerDay" => 86_400,
        _ => 0,
    }
}

/// `processAutomatedResponse` after `automatedResponseService.process`.
pub(crate) async fn after_keywords(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    message_id: Option<i64>,
    status: KeywordStatus,
) -> Result<(), JobError> {
    if status == KeywordStatus::Handled {
        return Ok(());
    }
    let ws = conversation.workspace_id;
    let workspace = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "defaultReply", "defaultReplyFrequency", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    params![ws],
                    |r| {
                        Ok(WorkspaceReply {
                            default_reply: r
                                .get::<_, Option<String>>(0)?
                                .and_then(|id| id.parse().ok()),
                            frequency: frequency_of(&r.get::<_, String>(1)?),
                            timezone: r.get(2)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some(workspace) = workspace else {
        return Ok(());
    };
    let trigger = match message_id {
        Some(id) => {
            let options = FindTriggerMessageOptions {
                conversation_id: conversation.id,
                id,
                require_complete_results: true,
                since_time: get_safe_since_time(
                    Some(SinceTimeInput::Ms(
                        contact_inbox
                            .last_message_at
                            .unwrap_or(contact_inbox.created_at),
                    )),
                    Some(TRIGGER_MESSAGE_LOOKBACK_MS),
                )
                .unwrap_or(0),
                workspace_id: ws,
            };
            deps.db
                .read(move |conn| find_trigger_message(conn, &options).map_err(crate::repo_err))
                .await
                .map_err(db_fail)?
        }
        None => None,
    };
    if trigger.is_none() {
        tracing::warn!(
            contact_inbox_id = contact_inbox.id,
            conversation_id = conversation.id,
            message_id,
            workspace_id = ws,
            "Automated response trigger message was not found"
        );
    }
    let reply = Reply {
        deps,
        conversation,
        contact_inbox,
        message_id,
        trigger: trigger.as_ref(),
        workspace: &workspace,
    };
    let contact_text = trigger
        .as_ref()
        .filter(|t| t.message.sender_type == "contact")
        .and_then(|t| t.message.text.clone())
        .filter(|t| !t.is_empty());

    // "Search Products": only once the keywords read this message and missed.
    if status == KeywordStatus::NoMatch
        && let (Some(text), Some(id)) = (&contact_text, message_id)
        && reply.products(text, id).await
    {
        return Ok(());
    }
    if trigger.is_some() {
        match reply.global_router().await {
            RouterResult::Success | RouterResult::Terminal | RouterResult::Contention => {
                return Ok(());
            }
            RouterResult::RetryableFailure => {
                tracing::error!(
                    conversation_id = conversation.id,
                    message_id,
                    workspace_id = ws,
                    "Global Router dispatch is retryable"
                );
                return Err(fail("Global Router dispatch failed before completion"));
            }
            RouterResult::Other => {}
        }
    }
    if let Err(err) = reply.agent_or_default_reply().await {
        tracing::error!(err = %err, conversation_id = conversation.id, workspace_id = ws, "[automated-response] triggerAutomatedResponse failed");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultReply {
    Triggered,
    Throttled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouterResult {
    Success,
    Terminal,
    Contention,
    RetryableFailure,
    /// absent / invalid / noEligibleMessage / unavailable: the default path runs.
    Other,
}

/// `ReplyByAIExecutionResult` (only a responding attempt is one).
struct AiResult {
    provider: String,
    used_fallback_text: bool,
}

/// One automated response run.
struct Reply<'a> {
    deps: &'a Deps,
    conversation: &'a Conversation,
    contact_inbox: &'a ContactInbox,
    message_id: Option<i64>,
    trigger: Option<&'a MessageWithAttachments>,
    workspace: &'a WorkspaceReply,
}

/// The `trackingContext` of a bot response to the trigger message.
fn tracking(
    conversation: &Conversation,
    message_id: Option<i64>,
    provider: &str,
    response_type: &str,
    start: i64,
    trigger_type: &str,
) -> Option<Value> {
    message_id.map(|id| {
        json!({
            "aiProvider": provider,
            "conversationId": conversation.id.to_string(),
            "messageId": id.to_string(),
            "responseType": response_type,
            "startTime": start,
            "triggerType": trigger_type,
            "workspaceId": conversation.workspace_id.to_string(),
        })
    })
}

impl Reply<'_> {
    fn trigger_created_at(&self) -> Option<String> {
        self.trigger.map(|t| iso(t.message.created_at))
    }

    /// `message:bot_received` for this trigger (no row without one).
    async fn bot_received(&self, fields: Value) {
        let Some(id) = self.message_id else { return };
        crate::events::emit_bot_received(
            self.deps.events.as_ref(),
            self.conversation.workspace_id,
            self.conversation.id,
            self.contact_inbox,
            &id.to_string(),
            fields,
        )
        .await;
    }

    /// `replyWithMatchingProducts`: best effort.
    async fn products(&self, text: &str, message_id: i64) -> bool {
        let built = chatbotx_business::webshop::product_search::build_product_search_reply(
            &self.deps.db,
            &self.deps.app_url,
            self.conversation.workspace_id,
            &self.contact_inbox.channel,
            text,
        )
        .await;
        let step = match built {
            Ok(Some(step)) => step,
            Ok(None) => return false,
            Err(err) => {
                tracing::warn!(err = %err, conversation_id = self.conversation.id, message_id, "Product search by message failed");
                return false;
            }
        };
        let added = self
            .deps
            .jobs
            .add(
                QueueName::Chat,
                crate::chat::SEND_FLOW_MESSAGE,
                json!({ "type": crate::chat::SEND_FLOW_MESSAGE, "data": {
                    "conversationId": self.conversation.id.to_string(),
                    "contactInboxId": self.contact_inbox.id.to_string(),
                    // Not part of any flow, like the cart CHECK OUT reply.
                    "flowId": "",
                    "step": step,
                }}),
                JobOptions {
                    job_id: Some(format!("product-search-{message_id}")),
                    ..JobOptions::default()
                },
            )
            .await;
        match added {
            Ok(_) => true,
            Err(err) => {
                tracing::warn!(err = %err, conversation_id = self.conversation.id, message_id, "Product search by message failed");
                false
            }
        }
    }

    /// `triggerDefaultReplyFlow`.
    async fn default_reply_flow(&self, tracking: Option<Value>) -> Result<DefaultReply, JobError> {
        let Some(flow_id) = self.workspace.default_reply else {
            return Ok(DefaultReply::Skipped);
        };
        let ws = self.conversation.workspace_id;
        let usable = self
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1 AND "currentVersionId" IS NOT NULL"#,
                        params![flow_id, ws],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .map_err(db_fail)?;
        if !usable {
            tracing::warn!(
                workspace_id = ws,
                default_reply_flow_id = flow_id,
                "[default-reply] configured default reply flow is missing, inactive, or has no published version"
            );
            return Ok(DefaultReply::Skipped);
        }
        let subject = chatbotx_db::repo::automation_throttle::ThrottleSubject {
            workspace_id: ws,
            contact_inbox_id: self.contact_inbox.id,
            throttle_type: "defaultReply".into(),
            subject_id: 0,
        };
        // `randomUUID()`: the column checks a 36-char claim id.
        let claim_id = crate::channels::api::random_uuid();
        let window = frequency_window_seconds(self.workspace.frequency);
        let (claim_subject, claim) = (subject.clone(), claim_id.clone());
        let outcome = chatbotx_db::repo::write(&self.deps.db, move |tx| {
            chatbotx_db::repo::automation_throttle::claim_automation_throttle(
                tx,
                &claim_subject,
                window,
                &claim,
                now_ms(),
            )
        })
        .await
        .map_err(db_fail)?;
        if matches!(
            outcome,
            chatbotx_db::repo::automation_throttle::ThrottleClaimOutcome::Denied { .. }
        ) {
            tracing::debug!(
                workspace_id = ws,
                contact_inbox_id = self.contact_inbox.id,
                default_reply_frequency = self.workspace.frequency,
                "[default-reply] throttled: activation window still open for this contact"
            );
            return Ok(DefaultReply::Throttled);
        }
        let mut data = json!({
            "conversationId": self.conversation.id.to_string(),
            "contactInboxId": self.contact_inbox.id.to_string(),
            "flowId": flow_id.to_string(),
            "startedByContactMessage": true,
            "origin": "channel",
        });
        if let Some(id) = self.message_id {
            data["triggerMessageId"] = json!(id.to_string());
        }
        if let Some(at) = self.trigger_created_at() {
            data["triggerMessageCreatedAt"] = json!(at);
        }
        if let Some(tracking) = tracking {
            data["trackingContext"] = tracking;
        }
        let added = self
            .deps
            .jobs
            .add(
                QueueName::Integration,
                "sendFlow",
                json!({ "type": "sendFlow", "data": data }),
                JobOptions::default(),
            )
            .await;
        if let Err(err) = added {
            // Roll back only the claim this run owns.
            let _ = chatbotx_db::repo::write(&self.deps.db, move |tx| {
                chatbotx_db::repo::automation_throttle::release_automation_throttle(
                    tx, &subject, &claim_id,
                )
            })
            .await;
            return Err(fail(err.to_string()));
        }
        Ok(DefaultReply::Triggered)
    }

    /// The default AI agent, else the default reply flow.
    async fn agent_or_default_reply(&self) -> Result<(), JobError> {
        let conversation = self.conversation;
        let ws = conversation.workspace_id;
        let agent = chatbotx_business::ai_agent::find_default(&self.deps.db, ws)
            .await
            .map_err(|e| fail(e.to_string()))?;
        let Some(agent) = agent else {
            let tracking = tracking(
                conversation,
                self.message_id,
                "none",
                "flow",
                now_ms(),
                "bot_response_default_reply_flow_no_ai_agent",
            );
            if self.default_reply_flow(tracking).await? != DefaultReply::Triggered {
                self.bot_received(json!({
                    "hasResponse": false, "responseType": "none", "routeType": "fallback",
                    "result": "fallback", "aiProvider": "none",
                    "metadata": { "latency": 0, "fallbackReason": "no_ai_agent", "triggerContext": {
                        "triggerSource": "worker", "triggerHandler": "triggerAutomatedResponse",
                        "triggerType": "bot_response_fallback_no_ai_agent" } },
                }))
                .await;
            }
            return Ok(());
        };
        let attachments: &[Attachment] = match self.trigger {
            Some(t)
                if !chatbotx_business::message_actions::is_whatsapp_flow_response(
                    t.message.content_attributes.as_ref().map(|a| &a.0),
                ) =>
            {
                &t.attachments
            }
            _ => &[],
        };
        let file_only = self.trigger.is_some_and(|t| {
            t.message.sender_type == "contact"
                && t.message.text.as_deref().is_none_or(str::is_empty)
                && !attachments.is_empty()
        });
        let has_image = attachments
            .iter()
            .any(|a| is_image(a) || a.file_type == "image" || a.file_type == "gif");
        let has_document = attachments.iter().any(is_document);
        let mut messages = self.history(&agent).await?;
        let summary = messages.1;
        if file_only {
            messages
                .0
                .push(Turn::user(file_only_prompt(has_document, has_image)));
        }
        let allowed: Option<Vec<&str>> = file_only.then(|| {
            let mut ids = Vec::new();
            if has_image {
                ids.push("image_reader");
            }
            if has_document {
                ids.push("document_reader");
            }
            ids
        });
        // The typing indicator every 4 s while the model runs (best effort).
        let typing = {
            let deps = self.deps.clone();
            let data = json!({
                "conversationId": self.conversation.id.to_string(),
                "contactInboxId": self.contact_inbox.id.to_string(),
                "typing": true,
                "seconds": 5,
            });
            tokio::spawn(async move {
                loop {
                    if let Err(err) = crate::chat::send_typing_to_channel(&deps, &data).await {
                        tracing::debug!(err = %err, "[automated-response] typing indicator failed");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                }
            })
        };
        let start = now_ms();
        let result = self
            .reply_by_ai(
                &agent,
                messages.0,
                &summary,
                file_only,
                allowed.as_deref(),
                None,
            )
            .await;
        typing.abort();
        match result {
            Some(r) if !r.used_fallback_text => {}
            Some(r) => {
                self.bot_received(json!({
                    "hasResponse": true, "responseType": "ai_agent", "routeType": "agent",
                    "result": "fallback", "aiProvider": r.provider,
                    "metadata": { "latency": now_ms() - start, "fallbackReason": "no_intent_match",
                        "triggerContext": { "triggerSource": "worker", "triggerHandler": "triggerAutomatedResponse",
                            "triggerType": "bot_response_ai_agent_fallback_text" } },
                }))
                .await;
            }
            None => {
                let tracking = tracking(
                    conversation,
                    self.message_id,
                    "none",
                    "flow",
                    start,
                    "bot_response_default_reply_flow_ai_failed",
                );
                if self.default_reply_flow(tracking).await? != DefaultReply::Triggered {
                    self.bot_received(json!({
                        "hasResponse": false, "responseType": "ai_agent", "routeType": "agent",
                        "result": "fallback", "aiProvider": "none",
                        "metadata": { "latency": now_ms() - start, "fallbackReason": "no_intent_match",
                            "triggerContext": { "triggerSource": "worker", "triggerHandler": "triggerAutomatedResponse",
                                "triggerType": "bot_response_ai_agent_failed" } },
                    }))
                    .await;
                }
            }
        }
        Ok(())
    }

    /// The model turns and the summary: the cached AI context refreshed with
    /// the messages since its marker, else the messages since the marker.
    async fn history(&self, agent: &AiAgent) -> Result<(Vec<Turn>, String), JobError> {
        let (ws, conv) = (self.conversation.workspace_id, self.conversation.id);
        let cached = context::get_or_init(self.deps, ws, conv, &agent.models.0)
            .await
            .map_err(fail)?;
        let state = self
            .deps
            .db
            .read(move |conn| {
                chatbotx_db::repo::conversation_ai_context::find_conversation_ai_context_state(
                    conn, conv, ws,
                )
                .map_err(crate::repo_err)
            })
            .await
            .map_err(db_fail)?;
        let marker = cached
            .as_ref()
            .and_then(|c| c.marker_message_id.as_deref().and_then(|m| m.parse().ok()))
            .or_else(|| state.and_then(|s| s.ai_context_last_message_id));
        let since = get_safe_since_time(
            Some(SinceTimeInput::Ms(
                self.contact_inbox
                    .last_message_at
                    .unwrap_or(self.contact_inbox.created_at),
            )),
            Some(ai::AI_MESSAGE_HISTORY_LOOKBACK_MS),
        )
        .unwrap_or(0);
        let options = FindAiContextMessagesOptions {
            conversation_id: conv,
            limit: ai::MAX_CONVERSATION_HISTORY,
            marker_message_id: marker,
            message_types: Vec::new(),
            since_time: Some(since),
            text_not_null: false,
            workspace_id: ws,
        };
        let rows = self
            .deps
            .db
            .read(move |conn| find_ai_context_messages(conn, &options).map_err(crate::repo_err))
            .await
            .map_err(db_fail)?;
        let (mut turns, mut summary) = (Vec::new(), String::new());
        if let Some(cached) = cached {
            let new_messages: Vec<AiMessage> = rows
                .iter()
                .filter_map(|m| {
                    let text = m.text.as_deref().filter(|t| !t.is_empty())?;
                    let role = match m.sender_type.as_str() {
                        "contact" => "user",
                        "user" | "bot" => "assistant",
                        _ => return None,
                    };
                    Some(AiMessage::text(
                        role,
                        text,
                        Some(m.id.to_string()),
                        Some(m.created_at),
                    ))
                })
                .collect();
            let refreshed = if new_messages.is_empty() {
                Some(cached)
            } else {
                context::append_history(self.deps, conv, new_messages).await
            };
            if let Some(refreshed) = refreshed {
                turns = context::turns(&refreshed.history);
                summary = refreshed.summary;
            }
        }
        if turns.is_empty() {
            turns = context::turns(&context::map_db_messages(rows));
        }
        Ok((turns, summary))
    }

    /// `replyByAI`: each configured model in order until one responds.
    async fn reply_by_ai(
        &self,
        agent: &AiAgent,
        messages: Vec<Turn>,
        summary: &str,
        file_only: bool,
        allowed_system_ids: Option<&[&str]>,
        tracking_trigger_type: Option<&str>,
    ) -> Option<AiResult> {
        let conversation = self.conversation;
        let configs = agent.models.0.as_array().cloned().unwrap_or_default();
        let tools: Vec<String> = agent
            .tools
            .0
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        let prompt = match agent.prompt.as_deref().filter(|p| !p.is_empty()) {
            None => String::new(),
            Some(prompt) => match &self.deps.variables {
                Some(vars) => vars
                    .resolve_step(conversation, self.contact_inbox, &json!(prompt), None, None)
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| prompt.to_owned()),
                None => prompt.to_owned(),
            },
        };
        let complete = if summary.is_empty() {
            prompt
        } else {
            format!("Conversation Context: {summary}\n\n{prompt}")
        };
        let web_search_selected = tools.iter().any(|t| t == "sys:web_search")
            && allowed_system_ids.is_none_or(|ids| ids.contains(&"web_search"));
        let executed_rule_ids = Arc::new(Mutex::new(std::collections::HashSet::new()));
        for (index, config) in configs.iter().enumerate() {
            let provider = if config.get("kind").and_then(Value::as_str) == Some("openaiCompatible")
            {
                "openaiCompatible".to_owned()
            } else {
                config
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let started = std::time::Instant::now();
            let result = self
                .run_ai_reply(
                    agent,
                    config,
                    &provider,
                    &tools,
                    &messages,
                    &complete,
                    web_search_selected,
                    file_only,
                    allowed_system_ids,
                    tracking_trigger_type,
                    &executed_rule_ids,
                )
                .await;
            tracing::info!(
                conversation_id = conversation.id,
                workspace_id = conversation.workspace_id,
                agent_id = agent.id,
                provider = %provider,
                provider_index = index,
                provider_count = configs.len(),
                duration_ms = started.elapsed().as_millis() as u64,
                responded = result.is_some(),
                "[automated-response] fallback chain provider attempt"
            );
            if result.is_some() {
                return result;
            }
        }
        tracing::warn!(
            conversation_id = conversation.id,
            workspace_id = conversation.workspace_id,
            agent_id = agent.id,
            provider_count = configs.len(),
            "[automated-response] all fallback providers exhausted without a response"
        );
        None
    }

    /// `runAIReply`: one provider attempt.
    #[allow(clippy::too_many_arguments)]
    async fn run_ai_reply(
        &self,
        agent: &AiAgent,
        config: &Value,
        provider: &str,
        tools: &[String],
        messages: &[Turn],
        prompt: &str,
        web_search_selected: bool,
        file_only: bool,
        allowed_system_ids: Option<&[&str]>,
        tracking_trigger_type: Option<&str>,
        executed_rule_ids: &Arc<Mutex<std::collections::HashSet<String>>>,
    ) -> Option<AiResult> {
        let conversation = self.conversation;
        let ws = conversation.workspace_id;
        let model = match ai::reply_model(&self.deps.db, ws, config).await {
            Ok(Some(model)) => model,
            Ok(None) => {
                tracing::debug!(
                    workspace_id = ws,
                    provider,
                    "[automated-response] provider skipped: no auto-reply-enabled integration found"
                );
                return None;
            }
            Err(err) => {
                tracing::error!(err = %err, provider, conversation_id = conversation.id, workspace_id = ws, "[automated-response] runAIReply failed");
                return None;
            }
        };
        let start = now_ms();
        let reply = Arc::new(ReplyContext {
            deps: self.deps.clone(),
            workspace_id: ws,
            conversation_id: conversation.id,
            contact_id: conversation.contact_id,
            contact_inbox_id: self.contact_inbox.id,
            inbox_id: self.contact_inbox.inbox_id,
            channel: self.contact_inbox.channel.clone(),
            trigger_message_id: self.message_id,
            trigger_created_at: self.trigger_created_at(),
            file_only,
            provider_info: config.clone(),
            sent: Mutex::new(None),
            tracking: Mutex::new(tracking(
                conversation,
                self.message_id,
                provider,
                "ai_agent",
                start,
                tracking_trigger_type.unwrap_or("bot_response_ai_agent_success"),
            )),
        });
        let toolset = toolset::load(&self.deps.db, &self.deps.ai.embedder, ws, tools)
            .await
            .with_reply(Arc::clone(&reply), tools, allowed_system_ids);
        let action_tool = self.action_tool(agent, executed_rule_ids).await;
        let prompt = match &action_tool {
            Some(_) => format!(
                "{prompt}\n\n{}\n\n{}",
                crate::ai_actions::actions_prompt(agent.action_prompt.as_deref()),
                crate::ai_actions::ACTION_TOOL_PROTOCOL
            )
            .trim()
            .to_owned(),
            None => prompt.to_owned(),
        };
        let mut system = text::guarded_prompt(&prompt, &toolset);
        if toolset.is_empty() && action_tool.is_some() {
            system = format!("{system}\n\n{}", text::FABRICATION_GUARD)
                .trim()
                .to_owned();
        }
        if toolset.has("connect_user_to_human") {
            system = format!("{system}\n\n{HANDOFF_POLICY}").trim().to_owned();
        }
        if web_search_selected {
            system = format!("{system}\n\n{WEB_SEARCH_UNAVAILABLE}")
                .trim()
                .to_owned();
        }
        let rich_mode = agent.is_rich_response && self.message_id.is_some();
        if agent.is_rich_response && self.message_id.is_none() {
            tracing::warn!(
                workspace_id = ws,
                conversation_id = conversation.id,
                reason = "missing_rich_response_execution_id",
                "[rich-response] rich mode disabled for AI reply"
            );
        }
        if rich_mode {
            system = format!("{system}\n\n{}", crate::rich_message::RICH_RESPONSE_FORMAT)
                .trim()
                .to_owned();
        }
        let mut request = Request::new(Some(system), messages.to_vec());
        request.max_output_tokens = Some(agent.max_output_tokens);
        request.temperature = Some(agent.temperature);
        request.max_steps = 5;
        let tools = crate::ai_actions::WithActions {
            base: &toolset,
            actions: action_tool,
        };
        request.tool_choice = if toolset.is_empty() && tools.actions.is_none() {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let generated = match llm::generate(&*self.deps.ai.http, &model, &request, &tools).await {
            Ok(generated) => generated,
            Err(err) => {
                tracing::error!(err = %err, provider, conversation_id = conversation.id, workspace_id = ws, "[automated-response] runAIReply failed");
                return None;
            }
        };
        let responded = |used_fallback_text| {
            Some(AiResult {
                provider: provider.to_owned(),
                used_fallback_text,
            })
        };
        let sent = reply.sent.lock().map(|s| s.clone()).unwrap_or_default();
        if let Some(sent) = sent {
            if !sent.is_empty() {
                self.append_assistant(&sent).await;
            }
            return responded(false);
        }
        if rich_mode {
            // One JSON document: the final step's text only.
            return self
                .rich_reply(
                    &generated.final_step_text(),
                    provider,
                    &model.model_id,
                    start,
                    tracking_trigger_type,
                )
                .await;
        }
        let full_text = generated.step_separated_text();
        let parts = stream::message_parts(&full_text);
        for part in &parts {
            let tracking = reply.take_tracking();
            send_message(self.deps, conversation.id, part, tracking, true).await;
        }
        if !parts.is_empty() {
            self.append_assistant(&full_text).await;
            return responded(false);
        }
        if generated.tool_calls > 0 {
            // No assistant text after tool activity: the default reply flow,
            // else the canned clarifying question; never the raw tool output.
            let trigger_type =
                tracking_trigger_type.unwrap_or("bot_response_ai_agent_default_reply_flow");
            let flow_tracking = tracking(
                conversation,
                self.message_id,
                provider,
                "flow",
                start,
                trigger_type,
            );
            match self.default_reply_flow(flow_tracking).await {
                Ok(DefaultReply::Skipped) => {
                    let tracking = tracking(
                        conversation,
                        self.message_id,
                        provider,
                        "ai_agent",
                        start,
                        trigger_type,
                    );
                    send_message(
                        self.deps,
                        conversation.id,
                        text::FALLBACK_LOOKUP,
                        tracking,
                        false,
                    )
                    .await;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::error!(err = %err, provider, conversation_id = conversation.id, "[automated-response] runAIReply failed");
                    return None;
                }
            }
            return responded(true);
        }
        tracing::warn!(
            provider,
            conversation_id = conversation.id,
            workspace_id = ws,
            history_message_count = messages.len(),
            "[automated-response] AI produced an empty completion (no text, no tool calls)"
        );
        None
    }

    /// The `apply_ai_agent_actions` tool of the agent's action rules: none
    /// without a trigger message or with malformed rules.
    async fn action_tool(
        &self,
        agent: &AiAgent,
        executed_rule_ids: &Arc<Mutex<std::collections::HashSet<String>>>,
    ) -> Option<crate::ai_actions::ActionTool> {
        use chatbotx_db::repo::partials::ai::parse_ai_agent_action_rules;
        let conversation = self.conversation;
        let rules = match parse_ai_agent_action_rules(&agent.action_rules.0) {
            Ok(rules) => rules,
            Err(err) => {
                tracing::warn!(err = %err, workspace_id = conversation.workspace_id, conversation_id = conversation.id, reason = "malformed_action_rules", "[ai-agent-actions] action tool disabled");
                return None;
            }
        };
        if rules.is_empty() {
            return None;
        }
        let Some(trigger_message_id) = self.message_id else {
            tracing::warn!(
                workspace_id = conversation.workspace_id,
                conversation_id = conversation.id,
                reason = "missing_trigger_message_id",
                "[ai-agent-actions] action tool disabled"
            );
            return None;
        };
        let field_names =
            crate::ai_actions::field_names(self.deps, conversation.workspace_id, &rules).await;
        let workspace_timezone = self.workspace_timezone().await;
        Some(crate::ai_actions::ActionTool {
            rules,
            field_names,
            context: crate::ai_actions::ActionContext {
                deps: self.deps.clone(),
                conversation: conversation.clone(),
                contact_inbox: self.contact_inbox.clone(),
                trigger_message_id,
                workspace_timezone,
            },
            executed_rule_ids: Arc::clone(executed_rule_ids),
        })
    }

    async fn workspace_timezone(&self) -> Option<String> {
        let ws = self.conversation.workspace_id;
        self.deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                        [ws],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten())
            })
            .await
            .ok()
            .flatten()
    }

    /// `handleRichAIReply` after the stream: the parsed messages and actions.
    async fn rich_reply(
        &self,
        full_text: &str,
        provider: &str,
        model_id: &str,
        start: i64,
        tracking_trigger_type: Option<&str>,
    ) -> Option<AiResult> {
        use crate::rich_message::{RichParse, parse_rich_response, send_rich_messages};
        let conversation = self.conversation;
        let execution_id = self.message_id?.to_string();
        let tracking = tracking(
            conversation,
            self.message_id,
            provider,
            "ai_agent",
            start,
            tracking_trigger_type.unwrap_or("rich_response"),
        );
        let responded = Some(AiResult {
            provider: provider.to_owned(),
            used_fallback_text: false,
        });
        let parsed = match parse_rich_response(full_text) {
            RichParse::Ok(parsed) => parsed,
            other => {
                tracing::warn!(
                    workspace_id = conversation.workspace_id,
                    conversation_id = conversation.id,
                    reason = other.reason(),
                    "[rich-response] invalid AI rich response"
                );
                let trimmed = full_text.trim();
                if other == RichParse::PlainText && !trimmed.is_empty() {
                    send_message(self.deps, conversation.id, trimmed, tracking, true).await;
                    self.append_assistant(trimmed).await;
                    return responded;
                }
                return None;
            }
        };
        let flow_context_id = chatbotx_core::utils::id::create_id();
        let (enqueued, skipped) = if parsed.messages.is_empty() {
            (0, 0)
        } else {
            send_rich_messages(
                self.deps,
                &parsed.messages,
                conversation.id,
                &flow_context_id,
                &execution_id,
                tracking,
            )
            .await
        };
        let ctx = crate::rich_response::RichContext {
            conversation,
            contact_inbox: self.contact_inbox,
            execution_id: execution_id.clone(),
        };
        let (executed, failed) =
            crate::rich_response::execute_rich_actions(self.deps, &parsed.actions, &ctx).await;
        let failed: Vec<Value> = failed
            .into_iter()
            .map(|(action, reason)| json!({ "action": action, "reason": reason }))
            .collect();
        if enqueued == 0 && executed == 0 {
            tracing::warn!(
                workspace_id = conversation.workspace_id,
                conversation_id = conversation.id,
                execution_id,
                failed_actions = %serde_json::to_string(&failed).unwrap_or_default(),
                enqueued,
                skipped,
                "[rich-response] action-only response failed without sending messages"
            );
            return None;
        }
        if enqueued == 0 {
            self.bot_received(json!({
                "hasResponse": true, "responseType": "ai_agent", "routeType": "agent",
                "result": "success", "aiProvider": provider,
                "metadata": {
                    "latency": now_ms() - start, "modelId": model_id, "richMode": true,
                    "actionResult": { "executed": executed, "failed": failed },
                    "triggerContext": {
                        "triggerSource": "worker", "triggerHandler": "replyByAI",
                        "triggerType": tracking_trigger_type.unwrap_or("rich_response_action_only"),
                    },
                },
            }))
            .await;
        }
        self.append_assistant(full_text).await;
        responded
    }

    async fn append_assistant(&self, content: &str) {
        let message = AiMessage::text("assistant", content, None, Some(now_ms()));
        context::append_history(self.deps, self.conversation.id, vec![message]).await;
    }

    /// `runGlobalRouter`.
    async fn global_router(&self) -> RouterResult {
        let Some(trigger) = self.trigger else {
            return RouterResult::Other;
        };
        let text = trigger
            .message
            .text
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_owned();
        if trigger.message.sender_type != "contact"
            || text.is_empty()
            || !text.chars().any(char::is_alphanumeric)
        {
            return RouterResult::Other;
        }
        let Some(message_id) = self.message_id else {
            return RouterResult::Other;
        };
        let ws = self.conversation.workspace_id;
        let router = match decision::find_enabled_router(&self.deps.db, ws).await {
            Ok(router) => router,
            Err(err) => {
                tracing::error!(err = %err, conversation_id = self.conversation.id, message_id, workspace_id = ws, "Global Router dispatch failed");
                self.router_analytics(Some("global_router_fallback_failed"), "fallback", now_ms())
                    .await;
                return RouterResult::RetryableFailure;
            }
        };
        let (config, connection, profile) = match router {
            Router::Absent => return RouterResult::Other,
            Router::Invalid(reason) => {
                tracing::error!(
                    conversation_id = self.conversation.id,
                    message_id,
                    reason,
                    workspace_id = ws,
                    "Global Router is configured but unavailable"
                );
                crate::events::log_failure(
                    self.deps,
                    &self.contact_inbox.channel,
                    ws,
                    Some(self.conversation.contact_id),
                    None,
                    json!({ "message": reason }),
                )
                .await;
                self.router_analytics(Some("global_router_config_invalid"), "fallback", now_ms())
                    .await;
                return RouterResult::Other;
            }
            Router::Usable {
                config,
                connection,
                profile,
            } => (config, connection, profile),
        };
        let lock_key = format!("global-router:{ws}:{}:{message_id}", self.conversation.id);
        let Ok(_guard) = ROUTER_LOCKS.try_acquire(&lock_key) else {
            return RouterResult::Contention;
        };
        let start = now_ms();
        let execution_key = format!(
            "global-router-{}",
            sha256_hex(&format!("{ws}:{}:{message_id}", profile.id))
        );
        let eval_deps = EvalDeps {
            db: &self.deps.db,
            encryption: self.deps.variables.as_ref().map(|v| &v.encryption),
            locks: Some(&EVAL_LOCKS),
            doh: &HttpDoh,
        };
        let evaluation = decision::evaluate_inbound_message(
            &eval_deps,
            &connection,
            &profile,
            &execution_key,
            &text,
        )
        .await;
        let mut destination = config.fallback.clone();
        let mut primary = false;
        let mut fallback_reason: Option<&'static str> = None;
        match evaluation {
            InboundEval::Contention => return RouterResult::Contention,
            InboundEval::Answer(answer) => {
                let route = decision::resolve_route(&answer, &config, &profile.contract.0);
                match route.destination {
                    Some(d) => {
                        destination = d;
                        primary = true;
                    }
                    None => {
                        fallback_reason = Some(if route.threshold_unmet {
                            "global_router_threshold_unmet"
                        } else {
                            "global_router_no_route"
                        });
                    }
                }
                let written = decision::write_router_metadata(
                    &self.deps.db,
                    ws,
                    self.conversation.contact_id,
                    &config,
                    &answer,
                )
                .await;
                match written {
                    Some(Ok(changes)) => self.emit_field_changes(changes).await,
                    Some(Err(_)) => {
                        destination = config.fallback.clone();
                        primary = false;
                        fallback_reason = Some("global_router_destination_failed");
                    }
                    None => {}
                }
            }
            InboundEval::ThresholdUnmet => fallback_reason = Some("global_router_threshold_unmet"),
            InboundEval::Error(_) => fallback_reason = Some("global_router_provider_failed"),
        }
        let result = self
            .dispatch(&destination, primary, profile.id, &text)
            .await;
        let is_human = |d: &GlobalRouterDestination| {
            matches!(
                d,
                GlobalRouterDestination::User { .. } | GlobalRouterDestination::Team { .. }
            )
        };
        if matches!(result, Dispatch::Success | Dispatch::Terminal) {
            if is_human(&destination) {
                self.router_analytics(
                    if primary { None } else { fallback_reason },
                    if primary { "success" } else { "fallback" },
                    start,
                )
                .await;
            }
            return result.into();
        }
        if primary {
            let reason = if result == Dispatch::Unavailable {
                "global_router_destination_unavailable"
            } else {
                "global_router_destination_failed"
            };
            let fallback = self
                .dispatch(&config.fallback, false, profile.id, &text)
                .await;
            if matches!(fallback, Dispatch::Success | Dispatch::Terminal) {
                if is_human(&config.fallback) {
                    self.router_analytics(Some(reason), "fallback", start).await;
                }
            } else {
                tracing::error!(
                    conversation_id = self.conversation.id,
                    message_id,
                    profile_id = profile.id,
                    workspace_id = ws,
                    "Global Router fallback destination failed"
                );
                self.router_analytics(
                    Some(if fallback == Dispatch::Unavailable {
                        "global_router_destination_unavailable"
                    } else {
                        "global_router_fallback_failed"
                    }),
                    "fallback",
                    start,
                )
                .await;
            }
            return fallback.into();
        }
        tracing::error!(
            conversation_id = self.conversation.id,
            message_id,
            profile_id = profile.id,
            workspace_id = ws,
            "Global Router fallback destination failed"
        );
        self.router_analytics(Some("global_router_fallback_failed"), "fallback", start)
            .await;
        result.into()
    }

    async fn emit_field_changes(
        &self,
        changes: Vec<chatbotx_business::contact_custom_field::FieldChange>,
    ) {
        let ws = self.conversation.workspace_id.to_string();
        let inbox = self.contact_inbox.id.to_string();
        let bus = crate::order_payment::bus(self.deps);
        for change in changes {
            let event = chatbotx_business::events::trigger::custom_field_changed(
                &ws,
                &change.contact_id.to_string(),
                &change.custom_field_id.to_string(),
                &change.custom_field_name,
                change.old_value.map_or(Value::Null, Value::String),
                change.new_value.map_or(Value::Null, Value::String),
                Some(&inbox),
            );
            bus.publish(Event::Trigger(event)).await;
        }
    }

    /// `emitGlobalRouterAnalytics`.
    async fn router_analytics(&self, fallback_reason: Option<&str>, result: &str, start: i64) {
        let mut metadata = json!({
            "latency": now_ms() - start,
            "triggerContext": {
                "triggerHandler": "globalRouter",
                "triggerSource": "worker",
                "triggerType": if result == "success" { "bot_response_global_router_route" } else { "bot_response_global_router_failed" },
            },
        });
        if let Some(reason) = fallback_reason {
            metadata["fallbackReason"] = json!(reason);
        }
        self.bot_received(json!({
            "aiProvider": "none", "hasResponse": false, "metadata": metadata,
            "responseType": "none", "result": result, "routeType": "fallback",
        }))
        .await;
    }

    /// `dispatchDestination`.
    async fn dispatch(
        &self,
        destination: &GlobalRouterDestination,
        primary: bool,
        profile_id: i64,
        text: &str,
    ) -> Dispatch {
        match self
            .try_dispatch(destination, primary, profile_id, text)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::error!(err = %err, conversation_id = self.conversation.id, message_id = self.message_id, primary, profile_id, workspace_id = self.conversation.workspace_id, "Global Router destination dispatch failed");
                Dispatch::RetryableFailure
            }
        }
    }

    async fn try_dispatch(
        &self,
        destination: &GlobalRouterDestination,
        primary: bool,
        profile_id: i64,
        text: &str,
    ) -> Result<Dispatch, String> {
        let ws = self.conversation.workspace_id;
        let trigger_type = if primary {
            "bot_response_global_router_route"
        } else {
            "bot_response_global_router_fallback"
        };
        let parse = |id: &str| id.parse::<i64>().ok();
        match destination {
            GlobalRouterDestination::Flow { id } => {
                let Some(flow_id) = parse(id) else {
                    return Ok(Dispatch::Unavailable);
                };
                let published = self
                    .deps
                    .db
                    .read(move |conn| {
                        Ok(conn
                            .query_row(
                                r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1 AND "currentVersionId" IS NOT NULL"#,
                                params![flow_id, ws],
                                |_| Ok(()),
                            )
                            .optional()?
                            .is_some())
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                if !published {
                    return Ok(Dispatch::Unavailable);
                }
                let message_id = self.message_id.unwrap_or_default();
                let mut data = json!({
                    "contactInboxId": self.contact_inbox.id.to_string(),
                    "conversationId": self.conversation.id.to_string(),
                    "flowId": flow_id.to_string(),
                    "origin": "channel",
                    "trackingContext": tracking(self.conversation, self.message_id, "none", "flow", now_ms(), trigger_type),
                    "triggerMessageId": message_id.to_string(),
                    "startedByContactMessage": true,
                });
                if let Some(at) = self.trigger_created_at() {
                    data["triggerMessageCreatedAt"] = json!(at);
                }
                self.deps
                    .jobs
                    .add(
                        QueueName::Integration,
                        "sendFlow",
                        json!({ "type": "sendFlow", "data": data }),
                        JobOptions {
                            job_id: Some(format!(
                                "global-router-flow-{}-{message_id}{}",
                                self.contact_inbox.id,
                                if primary { "" } else { "-fallback" }
                            )),
                            ..JobOptions::default()
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(Dispatch::Success)
            }
            GlobalRouterDestination::User { id } | GlobalRouterDestination::Team { id } => {
                let is_user = matches!(destination, GlobalRouterDestination::User { .. });
                let Some(target) = parse(id) else {
                    return Ok(Dispatch::Unavailable);
                };
                let sql = if is_user {
                    r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#
                } else {
                    r#"SELECT 1 FROM "InboxTeam" WHERE "workspaceId" = ?1 AND "id" = ?2"#
                };
                let available = self
                    .deps
                    .db
                    .read(move |conn| {
                        Ok(conn
                            .query_row(sql, params![ws, target], |_| Ok(()))
                            .optional()?
                            .is_some())
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                if !available {
                    return Ok(Dispatch::Unavailable);
                }
                // Before the handoff: it is irreversible.
                if is_user {
                    let on_shift = crate::steps::working_hours::list_on_shift_user_ids(
                        &self.deps.db,
                        ws,
                        vec![target],
                        now_ms(),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    if !on_shift.contains(&target) {
                        return Ok(Dispatch::Unavailable);
                    }
                }
                let handed_off = chatbotx_business::conversation_actions::hand_off_if_bot_enabled(
                    &self.deps.db,
                    ws,
                    self.conversation.id,
                    true,
                    "global_router_route",
                    "global_router",
                    json!({ "profileId": profile_id.to_string() }),
                )
                .await
                .map_err(|e| e.to_string())?;
                if !handed_off {
                    return Ok(Dispatch::Terminal);
                }
                // A successful handoff is irreversible: an assignment failure
                // is logged, never followed by a bot fallback.
                if let Err(err) = self.claim_assignment(is_user, target).await {
                    tracing::error!(err = %err, conversation_id = self.conversation.id, message_id = self.message_id, profile_id, workspace_id = ws, "Global Router assignment failed after human handoff");
                }
                Ok(Dispatch::Terminal)
            }
            GlobalRouterDestination::AiAgent { id } => {
                let Some(agent_id) = parse(id) else {
                    return Ok(Dispatch::Unavailable);
                };
                let agent: Option<AiAgent> = self
                    .deps
                    .db
                    .read(move |conn| {
                        find_one(
                            conn,
                            r#""id" = ?1 AND "workspaceId" = ?2"#,
                            &[&agent_id, &ws],
                        )
                        .map_err(crate::repo_err)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                let Some(agent) = agent else {
                    return Ok(Dispatch::Unavailable);
                };
                let routed = Reply {
                    workspace: &WorkspaceReply {
                        default_reply: None,
                        frequency: self.workspace.frequency,
                        timezone: self.workspace.timezone.clone(),
                    },
                    ..*self
                };
                let result = routed
                    .reply_by_ai(
                        &agent,
                        vec![Turn::user(text)],
                        "",
                        false,
                        None,
                        Some(trigger_type),
                    )
                    .await;
                Ok(if result.is_some() {
                    Dispatch::Success
                } else {
                    Dispatch::RetryableFailure
                })
            }
        }
    }

    /// `claimAssignmentIfUnassigned`.
    // ponytail: read-then-assign, not one guarded update; a racing manual
    // assignment between the two wins only when it lands first.
    async fn claim_assignment(&self, is_user: bool, target: i64) -> Result<(), String> {
        let (ws, id) = (self.conversation.workspace_id, self.conversation.id);
        let unassigned = self
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "assignedUserId" IS NULL AND "assignedInboxTeamId" IS NULL FROM "Conversation" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                        params![ws, id],
                        |r| r.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(false))
            })
            .await
            .map_err(|e| e.to_string())?;
        if !unassigned {
            return Ok(());
        }
        let assigned_id = format!("{}_{target}", if is_user { "u" } else { "t" });
        let assigned = chatbotx_business::conversation_actions::assign_one(
            &self.deps.db,
            ws,
            id,
            Some(assigned_id),
        )
        .await
        .map_err(|e| e.to_string())?;
        if let (Some(realtime), Some(event)) = (&self.deps.realtime, assigned.event) {
            realtime.queue_workspace_event(&ws.to_string(), event).await;
        }
        if let Some(job) = assigned.job {
            self.deps
                .jobs
                .add(
                    job.queue,
                    job.name,
                    job.data,
                    JobOptions {
                        job_id: job.job_id,
                        ..JobOptions::default()
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dispatch {
    Success,
    Terminal,
    Unavailable,
    RetryableFailure,
}

impl From<Dispatch> for RouterResult {
    fn from(value: Dispatch) -> RouterResult {
        match value {
            Dispatch::Success => RouterResult::Success,
            Dispatch::Terminal => RouterResult::Terminal,
            Dispatch::Unavailable => RouterResult::Other,
            Dispatch::RetryableFailure => RouterResult::RetryableFailure,
        }
    }
}

fn file_only_prompt(has_document: bool, has_image: bool) -> &'static str {
    match (has_image, has_document) {
        (true, false) => {
            "I uploaded an image. Please analyze it, provide a short summary, then ask what specific detail I want to know more about."
        }
        (false, true) => {
            "I uploaded a document. Please read it, provide a short summary, then ask what specific part I want to know more about."
        }
        _ => {
            "I uploaded one or more files. Please inspect the supported attachment, provide a short summary, then ask what specific detail I want to know more about."
        }
    }
}

/// `sendMessageAndWait` / `sendMessageWithRender` with a `trackingContext`:
/// an image URL goes as `url`. The wait never fails the caller.
async fn send_message(
    deps: &Deps,
    conversation_id: i64,
    text: &str,
    tracking: Option<Value>,
    wait: bool,
) {
    let lower = text.trim().to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or("");
    let is_image = (lower.starts_with("http://") || lower.starts_with("https://"))
        && [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"]
            .iter()
            .any(|ext| path.ends_with(ext));
    let mut data = json!({ "conversationId": conversation_id.to_string() });
    data[if is_image { "url" } else { "text" }] = json!(text);
    if let Some(tracking) = tracking {
        data["trackingContext"] = tracking;
    }
    let added = deps
        .jobs
        .add(
            QueueName::Chat,
            chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION,
            json!({ "type": chatbotx_jobs::chat_wait::SEND_CHAT_MESSAGE_ACTION, "data": data }),
            JobOptions::default(),
        )
        .await;
    let added = match added {
        Ok(added) => added,
        Err(err) => {
            tracing::error!(err = %err, conversation_id, "[automated-response] sendChatMessage enqueue failed");
            return;
        }
    };
    if !wait {
        return;
    }
    let target = chatbotx_jobs::WaitTarget {
        queue: Some(QueueName::Chat),
        job_id: added.job_id,
        ..chatbotx_jobs::WaitTarget::default()
    };
    let context = chatbotx_jobs::chat_wait::ChatWaitContext {
        conversation_id: Some(conversation_id.to_string()),
        step_id: None,
    };
    chatbotx_jobs::chat_wait::wait_for_chat_job_completion(
        deps.waiter.as_ref(),
        Some(&target),
        &context,
    )
    .await;
}

/// `systemFunctionContextGetter` + the system tool executors of one reply
/// attempt: what the tools send, start and read.
pub struct ReplyContext {
    pub deps: Deps,
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub inbox_id: i64,
    pub channel: String,
    pub trigger_message_id: Option<i64>,
    /// ISO text.
    pub trigger_created_at: Option<String>,
    pub file_only: bool,
    /// The model config of this attempt (`providerInfo`, the image reader's model).
    pub provider_info: Value,
    /// `directSendTracker`: the text the `sendMessage` hook sent.
    pub sent: Mutex<Option<String>>,
    /// `trackingContextRef`: attached to the first outbound message only.
    pub tracking: Mutex<Option<Value>>,
}

impl ReplyContext {
    fn take_tracking(&self) -> Option<Value> {
        self.tracking.lock().ok().and_then(|mut t| t.take())
    }

    /// `sendMessage`: marks the reply as sent even for an empty text.
    async fn send_message(&self, text: &str) {
        if let Ok(mut sent) = self.sent.lock() {
            *sent = Some(text.to_owned());
        }
        if !text.is_empty() {
            send_message(
                &self.deps,
                self.conversation_id,
                text,
                self.take_tracking(),
                true,
            )
            .await;
        }
    }

    /// `triggerFlow`.
    async fn trigger_flow(&self, flow_id: i64) -> Result<(), String> {
        let mut data = json!({
            "conversationId": self.conversation_id.to_string(),
            "contactInboxId": self.contact_inbox_id.to_string(),
            "flowId": flow_id.to_string(),
            "startedByContactMessage": true,
            "origin": "channel",
        });
        if let Some(id) = self.trigger_message_id {
            data["triggerMessageId"] = json!(id.to_string());
        }
        if let Some(at) = &self.trigger_created_at {
            data["triggerMessageCreatedAt"] = json!(at);
        }
        self.deps
            .jobs
            .add(
                QueueName::Integration,
                "sendFlow",
                json!({ "type": "sendFlow", "data": data }),
                JobOptions::default(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// An AI function's side effects (`getAIFunctionTools` with a context):
    /// the collected fields, then the linked flow plus the output message, or
    /// the output message alone. `true` = sent (the tool returns "").
    pub(crate) async fn function_effects(
        &self,
        fields: Vec<(i64, String)>,
        trigger_flow_id: Option<i64>,
        output_message: &str,
    ) -> Result<bool, String> {
        if !fields.is_empty() {
            chatbotx_business::contact_custom_field::set_values(
                &self.deps.db,
                chatbotx_business::contact_custom_field::SetValuesInput {
                    workspace_id: self.workspace_id,
                    contact_id: self.contact_id,
                    fields,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        if let Some(flow_id) = trigger_flow_id {
            self.trigger_flow(flow_id).await?;
            self.send_message(output_message).await;
            return Ok(true);
        }
        if !output_message.is_empty() {
            self.send_message(output_message).await;
            return Ok(true);
        }
        Ok(false)
    }

    /// The `sys:` tool executors of `replies.ts`.
    pub(crate) async fn system_tool(&self, id: &str, args: &Value) -> Result<String, String> {
        let arg = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        match id {
            "connect_user_to_human" => {
                let mut metadata = serde_json::Map::new();
                for key in ["userRequestExcerpt", "requestedBy"] {
                    if let Some(value) = args.get(key).filter(|v| !v.is_null()) {
                        metadata.insert(key.into(), value.clone());
                    }
                }
                metadata.entry("requestedBy").or_insert(json!("user"));
                chatbotx_business::conversation_actions::hand_off_if_bot_enabled(
                    &self.deps.db,
                    self.workspace_id,
                    self.conversation_id,
                    true,
                    arg("reason").unwrap_or("user_requested_human"),
                    "ai_system_tool",
                    Value::Object(metadata),
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok(HANDOFF_REPLY.to_owned())
            }
            "document_reader" => Ok(self
                .read_document(arg("query").unwrap_or(""), arg("documentContext"))
                .await),
            "url_context" => Ok(self.read_url(arg("query").unwrap_or(""), arg("url")).await),
            "image_reader" => Ok(self
                .read_image(arg("query").unwrap_or(""), arg("imageContext"))
                .await),
            other => Err(format!("Model tried to call unavailable tool '{other}'")),
        }
    }

    /// `createDocumentReaderExecutor`.
    async fn read_document(&self, query: &str, hint: Option<&str>) -> String {
        let result: Result<String, String> = async {
            let source = ai::conversation_source::resolve_document_source(
                &self.deps,
                self.workspace_id,
                self.conversation_id,
                self.trigger_message_id,
                hint,
            )
            .await
            .map_err(|e| e.to_string())?;
            let Some(source) = source else {
                return Ok("I couldn't find a supported PDF or DOCX document in this conversation yet.".into());
            };
            let mut snippets: Vec<String> =
                ai::conversation_source::retrieve_chunks(&self.deps, &source, query, Some(5))
                    .await?
                    .into_iter()
                    .map(|s| s.content)
                    .collect();
            let attachment = match source.attachment_id {
                Some(id) => self.attachment(id).await?,
                None => None,
            };
            if snippets.is_empty()
                && let Some(attachment) = &attachment
            {
                let data = json!({
                    "attachmentId": attachment.id.to_string(),
                    "conversationId": self.conversation_id.to_string(),
                    "mimeType": attachment.mime_type,
                    "originPath": attachment.origin_path,
                    "query": query,
                    "workspaceId": self.workspace_id.to_string(),
                });
                let job_id = format!(
                    "heavy-document-reader-{}-{}-{}",
                    self.conversation_id,
                    attachment.id,
                    &sha256_hex(query)[..32]
                );
                let result = self
                    .heavy(chatbotx_jobs::queues::heavy::HeavyJobAction::ExtractTextFromFile, data, job_id)
                    .await?;
                snippets = result["snippets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect();
            }
            let title = source
                .title
                .clone()
                .or_else(|| attachment.as_ref().and_then(|a| a.name.clone()))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "User uploaded document (PDF/DOCX)".into());
            let summary = summarize(
                source.summary.as_deref(),
                &snippets,
                "I found the document, but I need a more specific question to extract relevant details.",
            );
            Ok(format_output(
                &format!("Document: {}", title.trim()),
                &summary,
                &snippets,
                self.file_only.then_some(
                    "Follow-up: Ask the user what specific section or detail they want to explore next.",
                ),
            ))
        }
        .await;
        result.unwrap_or_else(|err| {
            tracing::error!(error = %err, conversation_id = self.conversation_id, workspace_id = self.workspace_id, "[document-reader] document tool execution failed");
            "I found your document, but I couldn't read it completely. Please ask a more specific question or try another file.".into()
        })
    }

    /// `createUrlReaderExecutor`.
    async fn read_url(&self, query: &str, hint: Option<&str>) -> String {
        let result: Result<String, String> = async {
            let source = ai::conversation_source::resolve_url_source(
                &self.deps,
                self.workspace_id,
                self.conversation_id,
                self.trigger_message_id,
                hint,
            )
            .await
            .map_err(|e| e.to_string())?;
            let Some(source) = source else {
                return Ok("I couldn't find a URL in this conversation to read context from. Please share a URL and ask your question.".into());
            };
            match source.status.as_str() {
                "pending" | "processing" => {
                    return Ok("The URL is still being processed. Please wait a moment and ask again.".into());
                }
                "error" => {
                    return Ok("I wasn't able to read the content of that URL. It may be inaccessible or unsupported. Please try a different URL.".into());
                }
                _ => {}
            }
            let snippets: Vec<String> =
                ai::conversation_source::retrieve_chunks(&self.deps, &source, query, Some(5))
                    .await?
                    .into_iter()
                    .map(|s| s.content)
                    .collect();
            let url = source
                .metadata
                .as_ref()
                .and_then(|m| m.0.get("url").and_then(Value::as_str).map(str::to_owned))
                .or_else(|| source.title.clone())
                .or_else(|| hint.map(str::to_owned))
                .unwrap_or_else(|| "User-provided URL".into());
            let summary = summarize(
                source.summary.as_deref(),
                &snippets,
                "I found the URL, but I need a more specific question to extract relevant details.",
            );
            Ok(format_output(
                &format!("URL: {url}"),
                &summary,
                &snippets,
                self.file_only.then_some(
                    "Follow-up: Ask the user what specific section or detail from the URL they want to explore next.",
                ),
            ))
        }
        .await;
        result.unwrap_or_else(|err| {
            tracing::error!(error = %err, conversation_id = self.conversation_id, workspace_id = self.workspace_id, "[url-reader] url context tool execution failed");
            "I found the URL, but I couldn't read it completely. Please ask a more specific question or try another URL.".into()
        })
    }

    /// `createImageReaderExecutor`: the trigger message's image, else the
    /// conversation's newest, through the heavy `analyzeImage` job.
    async fn read_image(&self, query: &str, image_context: Option<&str>) -> String {
        let result: Result<String, String> = async {
            let (ws, conv) = (self.workspace_id, self.conversation_id);
            let attachments: Vec<Attachment> = self
                .deps
                .db
                .read(move |conn| {
                    chatbotx_db::repo::find_many(
                        conn,
                        r#""workspaceId" = ?1 AND "conversationId" = ?2 ORDER BY "createdAt" DESC LIMIT 50"#,
                        &[&ws, &conv],
                    )
                    .map_err(crate::repo_err)
                })
                .await
                .map_err(|e| e.to_string())?;
            let images: Vec<&Attachment> = attachments.iter().filter(|a| is_image(a)).collect();
            let Some(attachment) = self
                .trigger_message_id
                .and_then(|m| images.iter().find(|a| a.message_id == m))
                .or(images.first())
                .copied()
            else {
                return Ok("I couldn't find a supported image in this conversation yet.".into());
            };
            let title = attachment
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or("User uploaded image")
                .to_owned();
            let mut lines = vec![
                "Analyze the uploaded image for a customer support conversation.".to_owned(),
                "Answer only from visible image content. If a requested detail is not visible, say that it is not visible.".to_owned(),
                "Return concise natural language. Do not return JSON or markdown tables.".to_owned(),
                format!("User question: {}", if query.is_empty() { "Describe this image." } else { query }),
            ];
            if let Some(context) = image_context {
                lines.push(format!("Image selection context: {context}"));
            }
            lines.push(format!("Image title: {title}"));
            if self.file_only {
                lines.push("If the user did not ask a specific question, provide a short summary and suggest what detail they can ask about next.".to_owned());
            }
            let prompt = lines.join("\n");
            let job_id = format!(
                "heavy-image-reader-{}-{}-{}",
                self.conversation_id,
                attachment.id,
                &sha256_hex(&json!({ "prompt": prompt, "providerInfo": self.provider_info }).to_string())[..32]
            );
            let data = json!({
                "workspaceId": self.workspace_id.to_string(),
                "originPath": attachment.origin_path,
                "mimeType": attachment.mime_type,
                "sizeBytes": attachment.size,
                "prompt": prompt,
                "providerInfo": self.provider_info,
            });
            let result = self
                .heavy(chatbotx_jobs::queues::heavy::HeavyJobAction::AnalyzeImage, data, job_id)
                .await?;
            let analysis = result["analysis"].as_str().unwrap_or("").trim().to_owned();
            if analysis.is_empty() {
                return Ok("I found the image, but I couldn't extract a useful visual answer from it.".into());
            }
            let mut output = vec![format!("Image: {title}"), format!("Analysis: {analysis}")];
            if self.file_only {
                output.push("Follow-up: Ask the user what specific detail in the image they want to know more about.".into());
            }
            Ok(output.join("\n"))
        }
        .await;
        result.unwrap_or_else(|err| {
            tracing::error!(error = %err, conversation_id = self.conversation_id, workspace_id = self.workspace_id, "[image-reader] image tool execution failed");
            "I found your image, but I couldn't analyze it completely. Please ask a more specific question or try another image.".into()
        })
    }

    async fn attachment(&self, id: i64) -> Result<Option<Attachment>, String> {
        let ws = self.workspace_id;
        self.deps
            .db
            .read(move |conn| {
                find_one(conn, r#""id" = ?1 AND "workspaceId" = ?2"#, &[&id, &ws])
                    .map_err(crate::repo_err)
            })
            .await
            .map_err(|e| e.to_string())
    }

    /// A heavy job and its result (`waitForJobCompletionWithRetries`).
    async fn heavy(
        &self,
        action: chatbotx_jobs::queues::heavy::HeavyJobAction,
        data: Value,
        job_id: String,
    ) -> Result<Value, String> {
        let name = action.as_str();
        let options = JobOptions {
            job_id: Some(job_id),
            ..chatbotx_jobs::queues::heavy::heavy_job_options(action)
        };
        let added = self
            .deps
            .jobs
            .add(
                QueueName::Heavy,
                name,
                json!({ "type": name, "data": data }),
                options,
            )
            .await
            .map_err(|e| e.to_string())?;
        let timeout = chatbotx_jobs::queues::heavy::heavy_completion_wait_timeout_ms(
            action,
            crate::heavy::wait_timeout_ms(),
        );
        self.deps
            .waiter
            .wait_until_finished(QueueName::Heavy, &added.job_id, timeout)
            .await
            .map_err(|e| e.to_string())
    }
}

/// `summarizeSnippets`.
fn summarize(summary: Option<&str>, snippets: &[String], empty: &str) -> String {
    let cut = |s: &str| s.chars().take(500).collect::<String>();
    match summary.map(str::trim).filter(|s| !s.is_empty()) {
        Some(summary) => cut(summary),
        None => snippets
            .first()
            .map_or_else(|| empty.to_owned(), |s| cut(s)),
    }
}

/// The reader tools' `formatToolOutput`.
fn format_output(
    head: &str,
    summary: &str,
    snippets: &[String],
    follow_up: Option<&str>,
) -> String {
    let mut out = vec![head.to_owned(), format!("Summary: {summary}")];
    if snippets.is_empty() {
        out.push("Relevant snippets: No matching snippets were found yet.".into());
    } else {
        out.push("Relevant snippets:".into());
        out.extend(
            snippets
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {s}", i + 1)),
        );
    }
    out.extend(follow_up.map(str::to_owned));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_output_and_frequency() {
        assert_eq!(
            format_output("URL: u", "s", &["a".into(), "b".into()], None),
            "URL: u\nSummary: s\nRelevant snippets:\n1. a\n2. b"
        );
        assert_eq!(summarize(Some("  "), &[], "none"), "none");
        assert_eq!(summarize(None, &["first".into()], "none"), "first");
        assert_eq!(frequency_window_seconds(frequency_of("oncePerDay")), 86_400);
        assert_eq!(frequency_window_seconds(frequency_of("bogus")), 0);
        assert_eq!(file_only_prompt(false, true).split(' ').nth(2), Some("an"));
    }
}
