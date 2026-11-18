//! `apps/worker/src/integration/handlers/thread-control.ts`: the
//! `threadControlAction` job (archive auto-release) and the `threadControlEvent`
//! routing webhook job.

use chatbotx_business::thread_control as tc;
use chatbotx_db::repo::parse_iso;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use chatbotx_db::rusqlite::{OptionalExtension as _, params};

use crate::{Deps, str_of};

/// ponytail: the worker has no Graph base override; channel tests use the
/// business crate's own local-server tests.
const GRAPH: &str = chatbotx_business::whatsapp::GRAPH_API_URL;

fn id_of(data: &Value, key: &str) -> Option<i64> {
    str_of(data, key).and_then(|v| v.parse().ok())
}

/// `releaseOwnedThread`: re-checks owned + version + still archived, then releases;
/// a permanent rejection is logged and the job completes.
pub async fn release_owned_thread(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (Some(ws), Some(contact_inbox), Some(conversation_id)) = (
        id_of(data, "workspaceId"),
        id_of(data, "contactInboxId"),
        id_of(data, "conversationId"),
    ) else {
        return Err(JobError::unrecoverable(
            "invalid threadControlAction payload",
        ));
    };
    let current = tc::resolve_current_state(&deps.db, ws, contact_inbox)
        .await
        .map_err(|e| JobError::retryable(e.to_string()))?;
    let Some((_, updated_at)) = current.filter(|(state, _)| state == "owned") else {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: the thread is no longer owned"
        );
        return Ok(());
    };
    // A job queued before the version was carried keeps the owned-only check.
    if let Some(expected) = data.get("threadControlUpdatedAt")
        && expected.as_str().and_then(parse_iso) != updated_at
    {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: ownership changed since the archive"
        );
        return Ok(());
    }
    let conversation =
        match chatbotx_business::conversation_list::find_or_fail(&deps.db, ws, conversation_id)
            .await
        {
            Ok(conversation) => Some(conversation),
            Err(error) if error.code() == Some("notFound") => None,
            Err(error) => return Err(JobError::retryable(error.message())),
        };
    let Some(conversation) = conversation.filter(|c| c.archived_at.is_some()) else {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: the conversation is no longer archived"
        );
        return Ok(());
    };
    let action = str_of(data, "action").unwrap_or("release");
    let result = tc::request_action(
        &deps.db,
        deps.realtime.as_ref(),
        GRAPH,
        tc::ActionRequest {
            workspace_id: ws,
            contact_inbox_id: contact_inbox,
            conversation_id: conversation.id,
            action,
            target_role: None,
            metadata: None,
            expected_updated_at: Some(updated_at),
        },
    )
    .await;
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.is_permanent() => {
            tracing::warn!(%error, contact_inbox, "Thread release after archive was rejected; not retrying");
            Ok(())
        }
        Err(error) => Err(JobError::retryable(error.to_string())),
    }
}

// ---- inbound deliveries -------------------------------------------------------

/// `inbox.threadControlSeenAt` (one read; `None` = the inbox never saw routing).
async fn inbox_seen_at(deps: &Deps, inbox_id: i64) -> Result<Option<i64>, JobError> {
    deps.db
        .read(move |conn| {
            Ok(chatbotx_db::repo::inbox::find_by_id(conn, inbox_id)
                .map_err(crate::repo_err)?
                .and_then(|i| i.thread_control_seen_at))
        })
        .await
        .map_err(|e| JobError::retryable(e.to_string()))
}

/// `recordInboundThreadControl`: records the routing effect of one inbound delivery
/// (`info` = the channel's `threadControl`). A failure rethrows so the job retries.
pub async fn record_inbound(
    deps: &Deps,
    inbox: &crate::channel::InboundInbox,
    contact_inbox: &chatbotx_db::repo::models::ContactInbox,
    conversation_id: i64,
    info: &Value,
    fallback_occurred_at: i64,
    supersedes_standby_copy: bool,
) -> Result<Option<tc::RecordEventResult>, JobError> {
    let delivery = str_of(info, "delivery").unwrap_or("owner");
    let occurred_at = str_of(info, "occurredAt")
        .and_then(parse_iso)
        .unwrap_or(fallback_occurred_at);
    let input = tc::RecordEventInput {
        workspace_id: inbox.workspace_id,
        inbox_id: inbox.inbox_id,
        inbox_seen_at: inbox_seen_at(deps, inbox.inbox_id).await?,
        contact_inbox: Some(contact_inbox.clone()),
        conversation_id,
        owner_role: str_of(info, "ownerRole").map(str::to_owned),
        owner_app_id: str_of(info, "ownerAppId").map(str::to_owned),
        occurred_at,
        supersedes_standby_at: supersedes_standby_copy.then_some(occurred_at),
        context: info.get("context").filter(|c| !c.is_null()).cloned(),
        ..tc::RecordEventInput::default()
    };
    tc::record_inbound_delivery(&deps.db, deps.realtime.as_ref(), input, delivery)
        .await
        .map_err(|error| {
            tracing::error!(%error, contact_inbox_id = contact_inbox.id, delivery,
                "Unable to record the conversation routing state of an inbound delivery");
            JobError::retryable(error.to_string())
        })
}

// ---- threadControlEvent (WhatsApp `messaging_handovers`) --------------------

/// `THREAD_CONTROL_EVENT_JOB_NAME`.
pub const EVENT_JOB: &str = "threadControlEvent";

/// A parsed `ThreadControlWebhookEvent` of a handover.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HandoverEvent {
    pub source_id: String,
    pub source_user_id: Option<String>,
    /// `controlPassed` | `controlTaken`.
    pub event: &'static str,
    pub previous_owner_role: Option<&'static str>,
    pub new_owner_role: Option<&'static str>,
    pub previous_owner_app_id: Option<String>,
    pub new_owner_app_id: Option<String>,
    pub handover_note: Option<String>,
    pub context: Option<Value>,
    pub occurred_at: i64,
    /// `onlyIfOwnedByAppId`: an inferred hand-back applies only to a thread that app holds.
    pub only_if_owned_by_app_id: Option<String>,
    /// `resumeEligible`: a handover TO us that may start the hand-back response.
    pub resume_eligible: bool,
    /// `aiAgentAppId`: the channel's AI-agent app id (Messenger only).
    pub ai_agent_app_id: Option<String>,
}

fn as_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `parseRole`: case-insensitive, `meta_ai` = `ai_agent`, unknown = `None`.
pub fn parse_role(value: Option<&Value>) -> Option<&'static str> {
    let normalised = value?.as_str()?.trim().to_lowercase();
    let role = if normalised == "meta_ai" {
        "ai_agent"
    } else {
        normalised.as_str()
    };
    tc::roles::parse(Some(role))
}

fn timestamp_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `metaSecondsToDate` (ms; `now` for a missing / bad value).
fn meta_seconds(value: Option<&Value>, now: i64) -> i64 {
    match timestamp_text(value).and_then(|t| t.parse::<f64>().ok()) {
        Some(s) if s.is_finite() && s > 0.0 => (s * 1000.0) as i64,
        _ => now,
    }
}

pub(crate) fn message_text(message: &Value) -> Option<String> {
    if let Some(body) = message.pointer("/text/body").and_then(Value::as_str) {
        return Some(body.to_owned());
    }
    let kind = as_text(message.get("type"))?;
    let caption = message
        .get(&kind)
        .and_then(|m| m.get("caption"))
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    Some(caption.map_or_else(|| format!("[{kind}]"), str::to_owned))
}

/// `parseConversationContext` (summary / history; template echoes keep `[template]`).
///
/// ponytail: `renderTemplateEcho` (filled template text of a business history item)
/// is not ported; the item reads `[template]`.
pub fn parse_conversation_context(value: Option<&Value>) -> Option<Value> {
    let value = value.filter(|v| !v.is_null())?;
    match value.get("type").and_then(Value::as_str) {
        Some("summary") => {
            let text = value.pointer("/summary/text").and_then(Value::as_str)?;
            (!text.trim().is_empty()).then(|| json!({ "type": "summary", "text": text }))
        }
        Some("history") => {
            let items = value.pointer("/history/items").and_then(Value::as_array)?;
            let parsed: Vec<Value> = items
                .iter()
                .filter(|item| item.is_object())
                .filter_map(|item| {
                    let sender =
                        if item.get("sender_type").and_then(Value::as_str) == Some("business") {
                            "business"
                        } else {
                            "user"
                        };
                    let echo = item.pointer("/message_echo/message");
                    let message = item.get("message");
                    let candidates = if sender == "business" {
                        [echo, message]
                    } else {
                        [message, echo]
                    };
                    let text = candidates.into_iter().flatten().find_map(message_text)?;
                    let mut out = json!({ "sender": sender, "text": text });
                    if let Some(t) = timestamp_text(item.get("timestamp")) {
                        out["timestamp"] = json!(t);
                    }
                    Some(out)
                })
                .collect();
            (!parsed.is_empty()).then(|| json!({ "type": "history", "items": parsed }))
        }
        other => {
            tracing::warn!(context_type = ?other, "Whatsapp conversation_context ignored: unrecognised shape");
            None
        }
    }
}

// ---- WhatsApp deliveries (`readThreadControlReceiveInfo`, standby echoes) -------

/// `readThreadControlReceiveInfo` of one `messages` / `standby` single-item raw body.
pub fn whatsapp_receive_info(raw: &Value) -> Value {
    let change = raw.pointer("/entry/0/changes/0");
    let value = change.and_then(|c| c.get("value"));
    let delivery = if change.and_then(|c| c.get("field")).and_then(Value::as_str) == Some("standby")
    {
        "standby"
    } else {
        "owner"
    };
    let mut info = json!({ "delivery": delivery });
    let raw_context = value.and_then(|v| {
        v.get("conversation_context")
            .or_else(|| v.pointer("/messages/0/conversation_context"))
    });
    if let Some(context) = parse_conversation_context(raw_context) {
        info["context"] = context;
    }
    let item = value.and_then(|v| {
        v.pointer("/standby/messages/0")
            .or_else(|| v.pointer("/standby/message_echoes/0"))
            .or_else(|| v.pointer("/messages/0"))
    });
    if let Some(seconds) = item
        .and_then(|i| timestamp_text(i.get("timestamp")))
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
    {
        info["occurredAt"] = json!(chatbotx_db::repo::iso((seconds * 1000.0) as i64));
    }
    info
}

/// `isStandbyEchoItem`.
pub fn is_standby_echo_item(item: &Value) -> bool {
    item.get("id").is_some_and(Value::is_string)
        && item.get("message").is_some_and(Value::is_object)
}

/// `fillPlaceholders`: `{{n}}` = the n-th sent parameter, `{{name}}` = that name's.
fn fill_placeholders(text: &str, parameters: &[Value]) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{\{\s*(\w+)\s*\}\}").expect("static regex")
    });
    RE.replace_all(text, |caps: &regex::Captures| {
        let key = &caps[1];
        let parameter = if key.chars().all(|c| c.is_ascii_digit()) {
            key.parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| parameters.get(i))
        } else {
            parameters
                .iter()
                .find(|p| p.get("parameter_name").and_then(Value::as_str) == Some(key))
        };
        parameter
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
            .map_or_else(|| caps[0].to_owned(), str::to_owned)
    })
    .into_owned()
}

/// `renderTemplateEcho`: the definition's HEADER / BODY text with the sent
/// parameters, `[image]` for a media header, the name when the body is missing.
pub fn render_template_echo(definition: Option<&Value>, sent: Option<&Value>) -> String {
    let name = sent
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or("template");
    let components = |v: Option<&Value>| -> Vec<Value> {
        v.and_then(|v| v.get("components"))
            .and_then(Value::as_array)
            .map(|c| c.iter().filter(|x| x.is_object()).cloned().collect())
            .unwrap_or_default()
    };
    let (definition, sent) = (components(definition), components(sent));
    let find = |list: &[Value], kind: &str| {
        list.iter()
            .find(|c| {
                c.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.eq_ignore_ascii_case(kind))
            })
            .cloned()
    };
    let params = |kind: &str| -> Vec<Value> {
        find(&sent, kind)
            .and_then(|c| c.get("parameters").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    };
    let header = find(&definition, "HEADER");
    let Some(body) = find(&definition, "BODY").and_then(|b| {
        b.get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    }) else {
        return format!("[template] {name}");
    };
    let mut lines = Vec::new();
    if let Some(header) = &header {
        let format = header
            .get("format")
            .and_then(Value::as_str)
            .map(str::to_uppercase);
        let text = header
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty());
        match (format.as_deref(), text) {
            (Some("TEXT"), Some(text)) => lines.push(fill_placeholders(text, &params("header"))),
            (Some(f), _) if f != "TEXT" && !f.is_empty() => {
                lines.push(format!("[{}]", f.to_lowercase()))
            }
            _ => {}
        }
    }
    lines.push(fill_placeholders(&body, &params("body")));
    lines.join("\n")
}

/// `parseStandbyEcho`: a partner's outgoing message as an outgoing third-party
/// message on the contact it was sent to (media labelled, never downloaded).
pub fn parse_standby_echo(echo: &Value, info: Value) -> crate::channel::Received {
    let sent = echo
        .get("message")
        .filter(|m| m.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let text = match sent.get("type").and_then(Value::as_str) {
        Some("template") => render_template_echo(echo.get("template"), sent.get("template")),
        Some("interactive") if sent.get("interactive").is_some_and(Value::is_object) => {
            let i = &sent["interactive"];
            as_text(i.pointer("/body/text"))
                .or_else(|| as_text(i.pointer("/header/text")))
                .unwrap_or_else(|| "[interactive]".into())
        }
        _ => message_text(&sent).unwrap_or_else(|| "[message]".into()),
    };
    let to = as_text(sent.get("to"));
    let recipient = as_text(sent.get("recipient"));
    let mut message = crate::channel::IncomingMessage::incoming(as_text(echo.get("id")));
    message.message_type = "outgoing".into();
    message.text = Some(text);
    message.content_attributes = Some(json!({ "threadControlEcho": true }));
    let mut info = info;
    // A template echo says nothing about ownership: any partner may send one.
    if sent.get("type").and_then(Value::as_str) == Some("template") {
        info["signalsOwnership"] = json!(false);
    }
    crate::channel::Received {
        contact: crate::channel::IncomingContact {
            source_id: to.clone().or_else(|| recipient.clone()).unwrap_or_default(),
            source_user_id: if to.is_none() { recipient } else { None },
            ..Default::default()
        },
        message: Some(message),
        echo_origin: Some("thirdParty".into()),
        thread_control: Some(info),
        ..Default::default()
    }
}

fn app_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `parseHandoverEvent`.
pub fn parse_handover_event(value: &Value, now: i64) -> Option<HandoverEvent> {
    let event = match value.get("type").and_then(Value::as_str) {
        Some("control_passed") => "controlPassed",
        Some("control_taken") => "controlTaken",
        _ => {
            tracing::warn!("Whatsapp messaging_handovers skipped: malformed payload");
            return None;
        }
    };
    let sender = value.get("sender");
    let phone =
        sender.and_then(|s| as_text(s.get("phone_number")).or_else(|| as_text(s.get("wa_id"))));
    let user_id = sender.and_then(|s| as_text(s.get("user_id")));
    let Some(source_id) = phone.or_else(|| user_id.clone()) else {
        tracing::warn!("Whatsapp messaging_handovers skipped: no sender identity");
        return None;
    };
    let passed = event == "controlPassed";
    let payload = value.get(if passed {
        "control_passed"
    } else {
        "control_taken"
    });
    let field = |k: &str| payload.and_then(|p| p.get(k));
    Some(HandoverEvent {
        source_id,
        source_user_id: user_id,
        event,
        previous_owner_role: parse_role(
            field("previous_owner_role").or_else(|| field("previous_owner_app_role")),
        ),
        new_owner_role: parse_role(field("new_owner_role").or_else(|| field("new_owner_app_role"))),
        previous_owner_app_id: app_id(field("previous_owner_app_id")),
        new_owner_app_id: app_id(field("new_owner_app_id")),
        handover_note: as_text(field("metadata")),
        context: if passed {
            parse_conversation_context(field("conversation_context"))
        } else {
            None
        },
        occurred_at: meta_seconds(value.get("timestamp"), now),
        only_if_owned_by_app_id: None,
        resume_eligible: passed,
        ai_agent_app_id: None,
    })
}

/// `messengerTimestampToOccurredAt`: Meta milliseconds, floored to whole seconds.
fn messenger_ms(value: Option<&Value>, now: i64) -> i64 {
    let ms = match timestamp_text(value).and_then(|t| t.parse::<f64>().ok()) {
        Some(ms) if ms.is_finite() && ms > 0.0 => ms as i64,
        _ => now,
    };
    chatbotx_db::repo::partials::thread_control::to_thread_control_timestamp(ms)
}

/// `isAiHandbackNotice` of a `messaging[]` item.
fn is_ai_handback_notice(item: &Value) -> bool {
    crate::channels::messenger_webhook::is_ai_handback_notice(item)
        && item.get("pass_thread_control").is_none()
        && item.get("take_thread_control").is_none()
}

/// Messenger `parseHandoverEvent` (`pass_thread_control` / `take_thread_control` /
/// the Business-AI hand-back notice). The owner is an app id, never a role.
pub fn parse_messenger_handover_event(
    item: &Value,
    now: i64,
    own_app_id: Option<&str>,
) -> Option<HandoverEvent> {
    let Some(sender) = item
        .pointer("/sender/id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        tracing::warn!("Messenger handover skipped: malformed payload");
        return None;
    };
    let occurred_at = messenger_ms(item.get("timestamp"), now);
    let passed = item.get("pass_thread_control").filter(|p| p.is_object());
    let Some(payload) =
        passed.or_else(|| item.get("take_thread_control").filter(|p| p.is_object()))
    else {
        if is_ai_handback_notice(item) {
            let ai = tc::channel::business_ai_app_id();
            return Some(HandoverEvent {
                source_id: sender.to_owned(),
                event: "controlPassed",
                previous_owner_app_id: Some(ai.clone()),
                only_if_owned_by_app_id: Some(ai),
                new_owner_app_id: own_app_id.map(str::to_owned),
                occurred_at,
                ..HandoverEvent::default()
            });
        }
        tracing::warn!("Messenger handover skipped: no pass/take payload");
        return None;
    };
    let note = payload
        .pointer("/thread_control_metadata/text")
        .and_then(Value::as_str)
        .or_else(|| payload.get("metadata").and_then(Value::as_str))
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned);
    Some(HandoverEvent {
        source_id: sender.to_owned(),
        event: if passed.is_some() {
            "controlPassed"
        } else {
            "controlTaken"
        },
        previous_owner_app_id: app_id(payload.get("previous_owner_app_id")),
        new_owner_app_id: app_id(payload.get("new_owner_app_id")),
        handover_note: note,
        occurred_at,
        ..HandoverEvent::default()
    })
}

/// `withHandoverDecisions`: the direction follows the app ids (a pass to another
/// app is a take from us; a take naming us is ours), and the Business-AI app is
/// the `ai_agent` role on either side.
pub fn with_messenger_decisions(
    mut event: HandoverEvent,
    own_app_id: Option<&str>,
) -> HandoverEvent {
    // `isResumeEligibleHandover`, judged on the event as delivered.
    event.resume_eligible = event.event == "controlPassed"
        && own_app_id.is_some()
        && event.new_owner_app_id.as_deref() == own_app_id;
    if let (Some(own), Some(new_owner)) = (own_app_id, event.new_owner_app_id.as_deref()) {
        let is_us = new_owner == own;
        if event.event == "controlPassed" && !is_us {
            event.event = "controlTaken";
        } else if event.event == "controlTaken" && is_us {
            event.event = "controlPassed";
        }
    }
    let ai = tc::channel::business_ai_app_id();
    if event.new_owner_app_id.as_deref() == Some(ai.as_str()) {
        event.new_owner_role = Some("ai_agent");
    }
    if event.previous_owner_app_id.as_deref() == Some(ai.as_str()) {
        event.previous_owner_role = Some("ai_agent");
    }
    event.ai_agent_app_id = Some(ai);
    event
}

/// How the job runner is running a `threadControlEvent` job.
#[derive(Debug, Clone, Default)]
pub struct EventJob {
    /// `isThreadControlJobReprocess`: a retry or a stalled-job recovery.
    pub is_retry: bool,
    /// The routing job's deterministic id.
    pub job_id: Option<String>,
}

impl EventJob {
    pub fn of(job: &chatbotx_jobs::JobCtx) -> Self {
        Self {
            is_retry: job.attempts_made > 0 || job.attempts_started > 1,
            job_id: Some(job.job_id.clone()).filter(|id| !id.is_empty()),
        }
    }
}

/// `receiveThreadControlEvent`: a handover is recorded (and may start the hand-back
/// response), a standby message goes through the inbound pipeline (stored, no
/// automation; may queue the AI take-back), a request / app roles is logged.
pub async fn receive_event(deps: &Deps, data: &Value, job: &EventJob) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("");
    let payload = &data["payload"];
    let kind = str_of(payload, "kind").unwrap_or("");
    if !matches!(integration_type, "whatsapp" | "messenger") {
        tracing::debug!(
            integration_type,
            "Channel has no receiveThreadControlEvent handler; dropping"
        );
        return Ok(());
    }
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
    match kind {
        "handover" => {}
        "standbyMessage" | "standbyEcho" => {
            return receive_standby_message(deps, integration_type, identifier, &payload["body"])
                .await;
        }
        "handoverRequest" => {
            // A partner asking for the thread changes no ownership; ack only.
            tracing::debug!(
                integration_type,
                identifier,
                "Thread-control request ignored"
            );
            return Ok(());
        }
        "appRoles" => {
            tracing::info!(integration_type, identifier, roles = %payload["body"]["app_roles"],
                "Thread-control receiver roles received");
            return Ok(());
        }
        _ => {
            tracing::warn!(
                integration_type,
                kind,
                "threadControlEvent dropped: malformed job payload"
            );
            return Ok(());
        }
    }
    let resolved = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            integration_type,
            identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?;
    let Some((_, channel_ctx, inbox)) = resolved else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
    };
    let now = chatbotx_db::repo::now_ms();
    let event = if integration_type == "messenger" {
        let own = channel_ctx
            .auth
            .get("clientId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty());
        parse_messenger_handover_event(&payload["body"], now, own)
            .map(|e| with_messenger_decisions(e, own))
    } else {
        parse_handover_event(&payload["body"], now)
    };
    let Some(event) = event else {
        return Ok(());
    };
    let contact = crate::channel::IncomingContact {
        source_id: event.source_id.clone(),
        source_user_id: event.source_user_id.clone(),
        ..Default::default()
    };
    let (probe_inbox, probe_contact) = (inbox.clone(), contact.clone());
    let existing = deps
        .db
        .read(move |conn| {
            Ok(crate::inbound::find_existing_contact_inbox(
                conn,
                &probe_inbox,
                &probe_contact,
            )?)
        })
        .await
        .map_err(|e| JobError::retryable(e.to_string()))?;
    let (contact_inbox, conversation) = match existing {
        Some((row, _)) => {
            let (ws, contact_id) = (inbox.workspace_id, row.contact_id);
            let conversation = deps
                .db
                .write(move |tx| {
                    Ok(crate::inbound::find_or_create_conversation(
                        tx, ws, contact_id, None, None,
                    )?)
                })
                .await
                .map_err(|e| JobError::retryable(e.to_string()))?;
            (row, conversation)
        }
        // `UNKNOWN_CONTACT_POLICY`: nothing to attach a take to.
        None if event.event == "controlTaken" => {
            tracing::debug!(
                inbox_id = inbox.inbox_id,
                "Dropping a thread-control handover for an unknown contact"
            );
            return Ok(());
        }
        None => {
            let (ci, conversation, _) = crate::inbound::detect_contact_and_conversation(
                deps,
                &inbox,
                &contact,
                "inboundMessage",
            )
            .await?;
            (ci, conversation)
        }
    };
    if let Some(only) = event.only_if_owned_by_app_id.as_deref()
        && contact_inbox.thread_owner_app_id.as_deref() != Some(only)
        && !is_own_retry(job, &event, &contact_inbox)
    {
        tracing::debug!(
            contact_inbox_id = contact_inbox.id,
            "Dropping an inferred hand-back: the thread is not held by the app it names"
        );
        return Ok(());
    }
    let (ws, inbox_id) = (inbox.workspace_id, inbox.inbox_id);
    let seen_at = inbox_seen_at(deps, inbox_id).await?;
    let thread_contact_inbox = contact_inbox.clone();
    let recorded = tc::record_event(
        &deps.db,
        deps.realtime.as_ref(),
        tc::RecordEventInput {
            workspace_id: ws,
            inbox_id,
            inbox_seen_at: seen_at,
            conversation_id: conversation.id,
            event: event.event.into(),
            owner_role: event.new_owner_role.map(str::to_owned),
            previous_owner_role: event.previous_owner_role.map(str::to_owned),
            owner_app_id: event.new_owner_app_id.clone(),
            previous_owner_app_id: event.previous_owner_app_id.clone(),
            occurred_at: event.occurred_at,
            context: event.context.clone(),
            handover_note: event.handover_note.clone(),
            contact_inbox: Some(contact_inbox),
            ..tc::RecordEventInput::default()
        },
    )
    .await
    .map_err(|e| JobError::retryable(e.to_string()))?;
    // ponytail: the channel's own owner expiry (`storeChannelThreadExpiry`, Messenger
    // thread_owner) is not fetched after a standby handover; the 24h rule applies.

    // Only a handover TO us starts the response, and only the first time it lands
    // (our own retry may have died before the enqueue; job ids dedupe it).
    let is_first_delivery = !recorded.is_redelivery || job.is_retry;
    if recorded.event_applied && is_first_delivery && event.resume_eligible {
        let previous = recorded
            .row
            .as_ref()
            .and_then(|r| r.thread_previous_owner_app_id.clone());
        let event_key = job
            .job_id
            .clone()
            .unwrap_or_else(|| event.occurred_at.to_string());
        start_handover_response(
            deps,
            HandoverResponse {
                workspace_id: ws,
                inbox_id,
                integration_type,
                integration_id: channel_ctx.integration_id,
                ai_agent_app_id: event.ai_agent_app_id.as_deref(),
                previous_owner_app_id: previous.as_deref(),
                event_key: &event_key,
                contact_inbox: &thread_contact_inbox,
                conversation: &conversation,
            },
        )
        .await?;
    }
    Ok(())
}

/// `isOwnRetry`: our own retry of an inferred hand-back whose event was already
/// recorded (the row no longer reads as the AI's because this very event moved it).
pub fn is_own_retry(
    job: &EventJob,
    event: &HandoverEvent,
    row: &chatbotx_db::repo::models::ContactInbox,
) -> bool {
    own_retry_of(
        job,
        event,
        row.thread_control_last_event.as_deref(),
        row.thread_previous_owner_app_id.as_deref(),
        row.thread_control_updated_at,
    )
}

fn own_retry_of(
    job: &EventJob,
    event: &HandoverEvent,
    last_event: Option<&str>,
    previous_owner_app_id: Option<&str>,
    updated_at: Option<i64>,
) -> bool {
    job.is_retry
        && last_event == Some(event.event)
        && previous_owner_app_id == event.only_if_owned_by_app_id.as_deref()
        && updated_at == Some(event.occurred_at)
}

// ---- hand-back response (`startHandoverResponse`) ------------------------------

pub struct HandoverResponse<'a> {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub integration_type: &'a str,
    /// The `Integration<Channel>` row (its `handoverResumeFlowId`).
    pub integration_id: i64,
    pub ai_agent_app_id: Option<&'a str>,
    pub previous_owner_app_id: Option<&'a str>,
    /// Identifies the hand-back event: a redelivery or retry never answers twice.
    pub event_key: &'a str,
    pub contact_inbox: &'a chatbotx_db::repo::models::ContactInbox,
    pub conversation: &'a chatbotx_db::repo::models::Conversation,
}

/// `startHandoverResponse`: a hand-back FROM the AI agent while the Page's AI
/// hand-over automation is active runs its flow (else its return message) and may
/// pause the bot; any other hand-back runs the Page / number's resume flow.
pub async fn start_handover_response(
    deps: &Deps,
    props: HandoverResponse<'_>,
) -> Result<(), JobError> {
    let (ws, inbox_id, integration_id) = (props.workspace_id, props.inbox_id, props.integration_id);
    let table = if props.integration_type == "messenger" {
        "IntegrationMessenger"
    } else {
        "IntegrationWhatsapp"
    };
    let from_ai = props
        .ai_agent_app_id
        .is_some_and(|ai| props.previous_owner_app_id == Some(ai))
        && chatbotx_core::utils::channel::parse_ai_handover_channel(Some(props.integration_type))
            .is_some();
    let now = chatbotx_db::repo::now_ms();
    let (settings, page_flow) = deps
        .db
        .read(move |conn| {
            let settings = if from_ai {
                chatbotx_business::ai_handover::find_active_in(conn, ws, inbox_id, now)
                    .map_err(|e| chatbotx_db::DbError::other(e.message()))?
            } else {
                None
            };
            let page_flow: Option<i64> = conn
                .query_row(
                    &format!(r#"SELECT "handoverResumeFlowId" FROM "{table}" WHERE "id" = ?1"#),
                    params![integration_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            Ok((settings, page_flow))
        })
        .await
        .map_err(|e| JobError::retryable(e.to_string()))?;
    // One flow decides: the AI hand-over card's while it applies, else the Page's.
    let flow_id = match &settings {
        Some(s) => s.goto_flow_id,
        None => page_flow,
    };
    let ci = props.contact_inbox;
    let conversation = props.conversation;
    let mut flow_started = false;
    if let Some(flow_id) = flow_id {
        let startable = deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                        params![flow_id, ws],
                        |r| r.get::<_, Option<i64>>(0),
                    )
                    .optional()?
                    .flatten()
                    .is_some())
            })
            .await
            .map_err(|e| JobError::retryable(e.to_string()))?;
        if startable {
            crate::inbound::add_job(
                deps,
                chatbotx_jobs::QueueName::Integration,
                crate::flow::SEND_FLOW,
                json!({ "type": crate::flow::SEND_FLOW, "data": {
                    "conversationId": conversation.id.to_string(),
                    "contactInboxId": ci.id.to_string(),
                    "flowId": flow_id.to_string(),
                    "origin": "channel",
                }}),
                chatbotx_jobs::JobOptions {
                    job_id: Some(format!("thread-resume-{}-{}", ci.id, props.event_key)),
                    ..Default::default()
                },
            )
            .await?;
            flow_started = true;
        } else {
            tracing::warn!(
                flow_id,
                workspace_id = ws,
                "Handover resume flow is missing or inactive; skipping"
            );
        }
    }
    let Some(settings) = settings else {
        return Ok(());
    };
    if let Some(text) = settings.return_message.as_deref().filter(|_| !flow_started) {
        match render_return_message(deps, conversation, ci, text).await {
            Some(text) => {
                crate::inbound::add_job(
                    deps,
                    chatbotx_jobs::QueueName::Chat,
                    crate::chat::SEND_CHAT_MESSAGE,
                    json!({ "type": crate::chat::SEND_CHAT_MESSAGE, "data": {
                        "conversation": { "id": conversation.id.to_string(), "workspaceId": ws.to_string() },
                        "contactInbox": { "id": ci.id.to_string() },
                        "text": text,
                    }}),
                    chatbotx_jobs::JobOptions {
                        job_id: Some(format!("thread-resume-message-{}-{}", ci.id, props.event_key)),
                        ..Default::default()
                    },
                )
                .await?;
            }
            None => tracing::warn!(
                contact_inbox_id = ci.id,
                "Could not resolve the hand-back return message variables; skipping it"
            ),
        }
    }
    if settings.pause_bot_waiting_for_staff && conversation.bot_enabled {
        // ponytail: the guarded handoff pauses the bot only; the transferred-to-human
        // trigger / webhook / analytics of `handoffExecutorService` come with Q.D6.
        chatbotx_business::conversation_actions::set_bot_enabled(
            &deps.db,
            ws,
            conversation.id,
            false,
        )
        .await
        .map_err(|e| JobError::retryable(e.message()))?;
    }
    Ok(())
}

/// `resolveContactVariablesDeep` of the return message; `None` = resolution failed
/// (the chat path would otherwise send a raw `{{placeholder}}`).
async fn render_return_message(
    deps: &Deps,
    conversation: &chatbotx_db::repo::models::Conversation,
    ci: &chatbotx_db::repo::models::ContactInbox,
    text: &str,
) -> Option<String> {
    let Some(vars) = &deps.variables else {
        return Some(text.to_owned());
    };
    match vars
        .resolve(
            conversation,
            ci,
            &Value::String(text.to_owned()),
            None,
            None,
            false,
        )
        .await
    {
        Ok(Value::String(text)) => Some(text),
        _ => None,
    }
}

// ---- standby messages + AI take-back -------------------------------------------

/// `ai-takeback-` job of `IntegrationJobAction.aiHandoverTakeBack`.
pub const TAKE_BACK_JOB: &str = "aiHandoverTakeBack";

/// `receiveStandbyMessage`: the normal inbound pipeline (stores, suppresses
/// automation), then the AI take-back when it is due.
///
/// ponytail: `recordWhatsappCallPermissionReply` for a standby copy comes with the
/// WhatsApp calls slice.
async fn receive_standby_message(
    deps: &Deps,
    integration_type: &str,
    identifier: &str,
    body: &Value,
) -> Result<(), JobError> {
    if integration_type == "messenger" && body.pointer("/entry/0/standby/0").is_none() {
        tracing::warn!("Messenger standby message dropped: no standby item");
        return Ok(());
    }
    let data = json!({
        "integrationType": integration_type,
        "integrationIdentifier": identifier,
        "payload": body,
    });
    let Some(stored) = crate::inbound::receive_message(deps, &data).await? else {
        return Ok(());
    };
    let Some(copy) = stored.standby_copy.as_ref() else {
        return Ok(());
    };
    // `toOwnerDeliveryPayload` (Messenger only): the standby item as an owner
    // `messaging[]` delivery, so the bot can answer it after the take-back.
    if integration_type != "messenger" || copy.message_type != "incoming" {
        return Ok(());
    }
    let first = &body["entry"][0];
    let replay = json!({ "object": body["object"], "entry": [{
        "id": first["id"], "time": first["time"], "messaging": [first["standby"][0]],
    }]});
    enqueue_take_back_if_due(deps, &stored, copy, identifier, replay).await
}

/// `resolveStage`: `take` while the AI agent holds the thread (standby), `replay`
/// once this app took it from the AI, else nothing.
fn take_back_stage(
    current: Option<&(String, chatbotx_db::repo::models::ContactInbox)>,
    ai_agent_app_id: &str,
) -> Option<&'static str> {
    let (state, row) = current?;
    stage_of(
        state,
        row.thread_owner_role.as_deref(),
        row.thread_control_last_event.as_deref(),
        row.thread_previous_owner_app_id.as_deref(),
        ai_agent_app_id,
    )
}

fn stage_of(
    state: &str,
    owner_role: Option<&str>,
    last_event: Option<&str>,
    previous_owner_app_id: Option<&str>,
    ai_agent_app_id: &str,
) -> Option<&'static str> {
    if state == "standby" && owner_role == Some("ai_agent") {
        return Some("take");
    }
    (state == "owned"
        && last_event == Some("taken")
        && previous_owner_app_id == Some(ai_agent_app_id))
    .then_some("replay")
}

async fn is_take_back_due(deps: &Deps, ws: i64, inbox_id: i64) -> Result<bool, JobError> {
    let now = chatbotx_db::repo::now_ms();
    deps.db
        .read(move |conn| {
            chatbotx_business::ai_handover::is_configured_and_inactive(conn, ws, inbox_id, now)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(|e| JobError::retryable(e.to_string()))
}

async fn current_thread(
    deps: &Deps,
    ws: i64,
    contact_inbox_id: i64,
) -> Result<Option<(String, chatbotx_db::repo::models::ContactInbox)>, JobError> {
    tc::resolve_current_row(&deps.db, ws, contact_inbox_id)
        .await
        .map_err(|e| JobError::retryable(e.to_string()))
}

/// `enqueueAiHandoverTakeBackIfDue`.
async fn enqueue_take_back_if_due(
    deps: &Deps,
    stored: &crate::inbound::Stored,
    copy: &chatbotx_db::repo::models::Message,
    identifier: &str,
    replay: Value,
) -> Result<(), JobError> {
    let ws = stored.conversation.workspace_id;
    let inbox_id = stored.inbox.inbox_id;
    let ai = tc::channel::business_ai_app_id();
    if !is_take_back_due(deps, ws, inbox_id).await? {
        return Ok(());
    }
    let current = current_thread(deps, ws, copy.contact_inbox_id).await?;
    if take_back_stage(current.as_ref(), &ai).is_none() {
        return Ok(());
    }
    let updated_at = current
        .as_ref()
        .and_then(|(_, row)| row.thread_control_updated_at)
        .map(chatbotx_db::repo::iso);
    crate::inbound::add_job(
        deps,
        chatbotx_jobs::QueueName::Integration,
        TAKE_BACK_JOB,
        json!({ "type": TAKE_BACK_JOB, "data": {
            "workspaceId": ws.to_string(),
            "inboxId": inbox_id.to_string(),
            "integrationType": "messenger",
            "integrationIdentifier": identifier,
            "contactInboxId": copy.contact_inbox_id.to_string(),
            "conversationId": copy.conversation_id.to_string(),
            "messageId": copy.id.to_string(),
            "aiAgentAppId": ai,
            "threadControlUpdatedAt": updated_at,
            "ownerReplayPayload": replay,
        }}),
        take_back_options(format!("ai-takeback-{}", copy.id)),
    )
    .await
}

/// `TAKE_BACK_JOB_OPTIONS`: 5 attempts, 10 s exponential, `removeOnFail`.
fn take_back_options(job_id: String) -> chatbotx_jobs::JobOptions {
    chatbotx_jobs::JobOptions {
        job_id: Some(job_id),
        attempts: Some(5),
        backoff: Some(chatbotx_jobs::options::Backoff::Exponential(10_000)),
        remove_on_fail: Some(chatbotx_jobs::options::Retention::Remove),
        ..Default::default()
    }
}

async fn replay_as_owner_delivery(deps: &Deps, data: &Value) -> Result<(), JobError> {
    crate::inbound::add_job(
        deps,
        chatbotx_jobs::QueueName::Integration,
        crate::inbound::INCOMING_MESSAGE,
        json!({ "type": crate::inbound::INCOMING_MESSAGE, "data": {
            "integrationType": data["integrationType"],
            "integrationIdentifier": data["integrationIdentifier"],
            "payload": data["ownerReplayPayload"],
        }}),
        chatbotx_jobs::JobOptions {
            job_id: Some(format!(
                "ai-takeback-replay-{}",
                str_of(data, "messageId").unwrap_or("")
            )),
            ..Default::default()
        },
    )
    .await
}

/// `runAiHandoverTakeBack`: re-reads the thread; replays when already taken from
/// the AI, takes then replays while the AI holds it and the automation is still
/// not running, else nothing. A thread another app holds is never taken.
pub async fn run_take_back(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType");
    if chatbotx_core::utils::channel::parse_ai_handover_channel(integration_type).is_none() {
        return Ok(());
    }
    let (Some(ws), Some(inbox_id), Some(contact_inbox), Some(conversation_id)) = (
        id_of(data, "workspaceId"),
        id_of(data, "inboxId"),
        id_of(data, "contactInboxId"),
        id_of(data, "conversationId"),
    ) else {
        return Err(JobError::unrecoverable(
            "invalid aiHandoverTakeBack payload",
        ));
    };
    let ai = str_of(data, "aiAgentAppId").unwrap_or("");
    let stage = take_back_stage(current_thread(deps, ws, contact_inbox).await?.as_ref(), ai);
    match stage {
        Some("replay") => return replay_as_owner_delivery(deps, data).await,
        Some("take") => {}
        _ => {
            tracing::info!(
                contact_inbox,
                "AI take-back skipped: the AI agent no longer holds the thread"
            );
            return Ok(());
        }
    }
    if !is_take_back_due(deps, ws, inbox_id).await? {
        tracing::info!(
            contact_inbox,
            "AI take-back skipped: the AI automation is running or not configured"
        );
        return Ok(());
    }
    let expected = str_of(data, "threadControlUpdatedAt").and_then(parse_iso);
    let result = tc::request_action(
        &deps.db,
        deps.realtime.as_ref(),
        GRAPH,
        tc::ActionRequest {
            workspace_id: ws,
            contact_inbox_id: contact_inbox,
            conversation_id,
            action: "take",
            target_role: None,
            metadata: None,
            expected_updated_at: Some(expected),
        },
    )
    .await;
    let taken = match result {
        // Owned through OUR take; an owned snapshot from anything else (the AI
        // handing back meanwhile) is a newer event that wins.
        Ok(row) => {
            row.thread_control_state.as_deref() == Some("owned")
                && row.thread_control_last_event.as_deref() == Some("taken")
        }
        Err(error) if error.is_permanent() => {
            let current = current_thread(deps, ws, contact_inbox).await?;
            let replay = take_back_stage(current.as_ref(), ai) == Some("replay");
            if !replay {
                tracing::warn!(%error, contact_inbox,
                    "AI take-back was refused; the AI keeps the thread and the bot stays silent");
            }
            replay
        }
        Err(error) => return Err(JobError::retryable(error.to_string())),
    };
    if taken {
        replay_as_owner_delivery(deps, data).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! `integrations/whatsapp/__tests__/conversation-routing-parsers.test.ts` and the
    //! handover cases of `conversation-routing-extractor.test.ts`.
    use super::*;
    use crate::channels::whatsapp_webhook::webhook_jobs;

    const USER: &str = "84900000001";
    const NOW: i64 = 1_767_225_600_000;

    fn handover_value(kind: &str, payload: Value) -> Value {
        let mut value = json!({
            "messaging_product": "whatsapp",
            "sender": { "phone_number": USER },
            "recipient": { "phone_number_id": "phone-1", "display_phone_number": "1555" },
            "type": kind,
            "timestamp": "1755700100",
        });
        value[kind] = payload;
        value
    }

    fn summary() -> Value {
        json!({ "type": "summary", "summary": { "text": "Customer wants to change the delivery address." } })
    }

    #[test]
    fn parses_the_official_summary_shape() {
        assert_eq!(
            parse_conversation_context(Some(&summary())),
            Some(
                json!({ "type": "summary", "text": "Customer wants to change the delivery address." })
            )
        );
    }

    #[test]
    fn parses_the_pdf_history_shape_mapping_sender_type_and_skipping_empty_items() {
        let history = json!({ "type": "history", "history": { "items": [
            { "sender_type": "user", "timestamp": "1755690000", "message": { "type": "text", "text": { "body": "Where is my order?" } } },
            { "sender_type": "business", "timestamp": "1755690060", "message_echo": { "message": { "type": "text", "text": { "body": "It ships today." } } } },
            { "sender_type": "user", "message": { "type": "image" } },
            { "sender_type": "user", "message": {} },
        ] } });
        assert_eq!(
            parse_conversation_context(Some(&history)),
            Some(json!({ "type": "history", "items": [
                { "sender": "user", "text": "Where is my order?", "timestamp": "1755690000" },
                { "sender": "business", "text": "It ships today.", "timestamp": "1755690060" },
                { "sender": "user", "text": "[image]" },
            ] }))
        );
    }

    #[test]
    fn parses_control_passed_with_summary_context_and_note() {
        let event = parse_handover_event(
            &handover_value(
                "control_passed",
                json!({ "previous_owner_role": "ai_agent", "new_owner_role": "escalation",
                        "metadata": "user requested human", "conversation_context": summary() }),
            ),
            NOW,
        )
        .expect("event");
        assert_eq!(event.source_id, USER);
        assert_eq!(event.event, "controlPassed");
        assert_eq!(event.previous_owner_role, Some("ai_agent"));
        assert_eq!(event.new_owner_role, Some("escalation"));
        assert_eq!(event.handover_note.as_deref(), Some("user requested human"));
        assert_eq!(event.context.as_ref().unwrap()["type"], "summary");
        assert_eq!(event.occurred_at, 1_755_700_100_000);
    }

    #[test]
    fn keeps_the_handover_when_metadata_is_not_a_string() {
        let event = parse_handover_event(
            &handover_value(
                "control_passed",
                json!({ "new_owner_role": "escalation", "metadata": { "unexpected": "shape" } }),
            ),
            NOW,
        )
        .expect("event");
        assert_eq!(event.handover_note, None);
        assert_eq!(event.new_owner_role, Some("escalation"));
    }

    #[test]
    fn parses_control_taken_without_context_and_ignores_a_context_on_it() {
        let event = parse_handover_event(
            &handover_value(
                "control_taken",
                json!({ "previous_owner_role": "customer_service", "new_owner_role": "escalation",
                        "conversation_context": summary() }),
            ),
            NOW,
        )
        .expect("event");
        assert_eq!(event.event, "controlTaken");
        assert_eq!(event.context, None);
    }

    #[test]
    fn maps_an_unknown_role_to_null_and_coerces_numeric_app_ids() {
        let event = parse_handover_event(
            &handover_value(
                "control_passed",
                json!({ "previous_owner_role": "brand_new_role", "previous_owner_app_id": 1_234_567_890_123_456_i64,
                        "new_owner_app_id": "9876543210" }),
            ),
            NOW,
        )
        .expect("event");
        assert_eq!(event.previous_owner_role, None);
        assert_eq!(
            event.previous_owner_app_id.as_deref(),
            Some("1234567890123456")
        );
        assert_eq!(event.new_owner_app_id.as_deref(), Some("9876543210"));
    }

    #[test]
    fn accepts_the_pdf_app_role_keys_and_normalises_meta_ai_to_ai_agent() {
        let event = parse_handover_event(
            &handover_value(
                "control_passed",
                json!({ "previous_owner_app_role": "META_AI", "new_owner_app_role": "ESCALATION" }),
            ),
            NOW,
        )
        .expect("event");
        assert_eq!(event.previous_owner_role, Some("ai_agent"));
        assert_eq!(event.new_owner_role, Some("escalation"));
    }

    #[test]
    fn keys_a_bsuid_only_sender_by_its_user_id_and_falls_back_to_the_clock() {
        let mut value = handover_value("control_taken", json!({}));
        value["sender"] = json!({ "user_id": "US.1" });
        value.as_object_mut().unwrap().remove("timestamp");
        let event = parse_handover_event(&value, NOW).expect("event");
        assert_eq!(event.source_id, "US.1");
        assert_eq!(event.source_user_id.as_deref(), Some("US.1"));
        assert_eq!(event.occurred_at, NOW);
    }

    fn envelope(value: Value) -> Value {
        json!({ "object": "whatsapp_business_account",
                "entry": [{ "id": "waba", "changes": [{ "field": "messaging_handovers", "value": value }] }] })
    }

    #[test]
    fn emits_a_handover_job_for_control_taken_with_a_stable_id() {
        let body = envelope(handover_value("control_taken", json!({})));
        let jobs = webhook_jobs(&body, None);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "threadControlEvent");
        assert_eq!(jobs[0].data["data"]["payload"]["kind"], "handover");
        assert!(jobs[0].job_id.starts_with("wa-tc-phone-1-"));
        assert_eq!(webhook_jobs(&body, None)[0].job_id, jobs[0].job_id);
    }

    #[test]
    fn drops_a_handover_without_recipient_phone_number_id_or_for_another_number() {
        let mut value = handover_value("control_taken", json!({}));
        assert!(webhook_jobs(&envelope(value.clone()), Some("phone-2")).is_empty());
        value.as_object_mut().unwrap().remove("recipient");
        assert!(webhook_jobs(&envelope(value), None).is_empty());
    }

    // integrations/messenger/__tests__/conversation-routing-parsers.test.ts

    #[test]
    fn messenger_pass_is_controlpassed_with_app_ids_note_and_floored_seconds() {
        let item = json!({ "sender": { "id": "psid" }, "recipient": { "id": "page" },
            "timestamp": 1_700_000_000_123_i64,
            "pass_thread_control": { "new_owner_app_id": 42, "previous_owner_app_id": "9",
                "thread_control_metadata": { "text": "note" }, "metadata": "flat" } });
        let event = parse_messenger_handover_event(&item, 1, None).unwrap();
        assert_eq!(event.event, "controlPassed");
        assert_eq!(event.new_owner_app_id.as_deref(), Some("42"));
        assert_eq!(event.previous_owner_app_id.as_deref(), Some("9"));
        assert_eq!(event.handover_note.as_deref(), Some("note"));
        assert_eq!(event.occurred_at, 1_700_000_000_000);
        assert!(
            parse_messenger_handover_event(&json!({ "pass_thread_control": {} }), 1, None)
                .is_none()
        );
    }

    #[test]
    fn messenger_direction_follows_the_app_ids_and_the_ai_app_is_the_ai_agent() {
        let ai = tc::channel::business_ai_app_id();
        let pass_to_ai = json!({ "sender": { "id": "psid" },
            "pass_thread_control": { "new_owner_app_id": ai.clone() } });
        let event = with_messenger_decisions(
            parse_messenger_handover_event(&pass_to_ai, 1, Some("own")).unwrap(),
            Some("own"),
        );
        assert_eq!(
            (event.event, event.new_owner_role),
            ("controlTaken", Some("ai_agent"))
        );
        let take_by_us = json!({ "sender": { "id": "psid" },
            "take_thread_control": { "new_owner_app_id": "own", "previous_owner_app_id": ai } });
        let event = with_messenger_decisions(
            parse_messenger_handover_event(&take_by_us, 1, Some("own")).unwrap(),
            Some("own"),
        );
        assert_eq!(
            (event.event, event.previous_owner_role),
            ("controlPassed", Some("ai_agent"))
        );
    }

    #[test]
    fn messenger_ai_handback_notice_is_a_pass_only_if_the_ai_holds_the_thread() {
        let notice = json!({ "sender": { "id": "psid" }, "timestamp": 5000,
            "message": { "admin_text": "The AI handed the chat back" } });
        let event = parse_messenger_handover_event(&notice, 1, Some("own")).unwrap();
        let ai = tc::channel::business_ai_app_id();
        assert_eq!(event.event, "controlPassed");
        assert_eq!(event.only_if_owned_by_app_id.as_deref(), Some(ai.as_str()));
        assert_eq!(event.new_owner_app_id.as_deref(), Some("own"));
        let vietnamese =
            json!({ "sender": { "id": "psid" }, "message": { "admin_text": "ai đó" } });
        assert!(parse_messenger_handover_event(&vietnamese, 1, None).is_none());
    }
}

#[cfg(test)]
#[path = "thread_control_flow_tests.rs"]
mod flow_tests;
