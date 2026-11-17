//! `apps/worker/src/integration/handlers/thread-control.ts`: the
//! `threadControlAction` job (archive auto-release) and the `threadControlEvent`
//! routing webhook job.

use chatbotx_business::thread_control as tc;
use chatbotx_db::repo::parse_iso;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

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

fn message_text(message: &Value) -> Option<String> {
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
    })
}

/// `receiveThreadControlEvent` for a WhatsApp handover: resolves (or, for
/// `controlPassed`, creates) the contact and records the event.
///
/// ponytail: standby messages / echoes, the Messenger routing items, the hand-back
/// response (resume flow / AI settings, `startHandoverResponse`) and the AI
/// take-back are not ported; they are logged and acknowledged.
pub async fn receive_event(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("");
    let payload = &data["payload"];
    let kind = str_of(payload, "kind").unwrap_or("");
    if integration_type != "whatsapp" || kind != "handover" {
        tracing::info!(
            integration_type,
            kind,
            "Thread-control routing item acknowledged (not ported)"
        );
        return Ok(());
    }
    let Some(event) = parse_handover_event(&payload["body"], chatbotx_db::repo::now_ms()) else {
        return Ok(());
    };
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
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
    let Some((_, _, inbox)) = resolved else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
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
    let (ws, inbox_id) = (inbox.workspace_id, inbox.inbox_id);
    let seen_at = deps
        .db
        .read(move |conn| {
            Ok(chatbotx_db::repo::inbox::find_by_id(conn, inbox_id)
                .map_err(crate::repo_err)?
                .and_then(|i| i.thread_control_seen_at))
        })
        .await
        .map_err(|e| JobError::retryable(e.to_string()))?;
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
    if recorded.event_applied && event.event == "controlPassed" && !recorded.is_redelivery {
        tracing::info!(
            conversation_id = conversation.id,
            "Handover to this app recorded; the resume flow (startHandoverResponse) is not ported"
        );
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
}
