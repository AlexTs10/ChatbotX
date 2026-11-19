//! The `whatsappCallEvent` integration job (`handlers/whatsapp-call.ts`) and
//! the terminal call card (`shared/whatsapp-call-finalize.ts`
//! `finalizeCallSideEffects`): the call row, its activity message, the
//! contact / conversation activity and the `incomingCall` / `callEnded` /
//! `missedAudioCall` events.
//!
//! The VoIP hooks (`markOutboundRinging` / `markOutboundAccepted` +
//! `whatsappCallOutboundStatus`, `emitCallEndedToAgent`) live in
//! [`crate::whatsapp_voip`].
use chatbotx_business::events::{Event, TriggerEvent};
use chatbotx_business::message::message_json;
use chatbotx_core::utils::whatsapp_call::{WhatsappCallMediaToggles, transcribes_calls};
use chatbotx_db::repo::message::{
    self as repo_message, ContentBySourcePatch, CreateMessageInput, MessageWithAttachments,
};
use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::partials::whatsapp_call::{
    WhatsappCallTerminalStatus, resolve_whatsapp_call_outcome,
};
use chatbotx_db::repo::whatsapp_call::{self as calls, FinalizeInput, WhatsappCallUpsertInput};
use chatbotx_db::repo::{integration_whatsapp, iso, new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use chatbotx_rt::protocol::EventType;
use serde_json::{Map, Value, json};

use crate::channel::IncomingContact;
use crate::{Deps, db_fail, fail, str_of};

/// `CALL_CANCELED_BY_BUSINESS_LAST_ERROR`.
pub const CALL_CANCELED_BY_BUSINESS_LAST_ERROR: &str = "canceled_by_business";

async fn read<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::read(&deps.db, f).await.map_err(db_fail)
}

async fn write<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::write(&deps.db, move |tx| f(tx))
        .await
        .map_err(db_fail)
}

async fn by_wacid(deps: &Deps, wacid: &str) -> Result<Option<WhatsappCall>, JobError> {
    let wacid = wacid.to_owned();
    read(deps, move |c| calls::find_by_wacid(c, &wacid)).await
}

async fn by_attempt(deps: &Deps, attempt: Option<&str>) -> Result<Option<WhatsappCall>, JobError> {
    let Some(attempt) = attempt.filter(|a| !a.is_empty()).map(str::to_owned) else {
        return Ok(None);
    };
    read(deps, move |c| calls::find_by_attempt_id(c, &attempt)).await
}

/// Meta call timestamps are unix seconds.
fn unix_seconds_ms(value: Option<&str>) -> Option<i64> {
    let seconds = value?.trim().parse::<f64>().ok()?;
    seconds
        .is_finite()
        .then(|| (seconds * 1000.0).round() as i64)
}

/// The `whatsappCallEvent` processor (inside the webhook context).
pub async fn handle_event(deps: &Deps, data: &Value) -> Result<(), JobError> {
    use chatbotx_business::events::context::{
        WebhookExecutionContext, with_webhook_execution_context,
    };
    with_webhook_execution_context(WebhookExecutionContext::webhook(), dispatch(deps, data)).await
}

async fn dispatch(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let event = &data["payload"]["event"];
    match event["kind"].as_str() {
        Some("connect") => handle_connect(deps, data, event).await,
        Some("status") => handle_status(deps, event).await,
        Some("terminate") => handle_terminate(deps, data, event).await,
        kind => {
            tracing::warn!(?kind, "Unhandled whatsapp call event kind");
            Ok(())
        }
    }
}

/// `resolveCallerIdentity`: the item's own identity wins; the payload
/// contact only fills in when it is the same party.
fn caller_identity(
    payload: &Value,
    event: &Value,
) -> (Option<String>, Option<String>, Option<Value>) {
    let outbound = event["direction"] == "businessInitiated";
    let (wa_key, user_key) = if outbound {
        ("to", "toUserId")
    } else {
        ("from", "fromUserId")
    };
    let item_wa = str_of(event, wa_key).map(str::to_owned);
    let item_user = str_of(event, user_key).map(str::to_owned);
    let contact = payload.get("contact").filter(|c| c.is_object()).cloned();
    if item_wa.is_none() && item_user.is_none() {
        let wa = contact
            .as_ref()
            .and_then(|c| str_of(c, "waId"))
            .map(str::to_owned);
        let user = contact
            .as_ref()
            .and_then(|c| str_of(c, "userId"))
            .map(str::to_owned);
        return (wa, user, contact);
    }
    let matched = contact.filter(|c| {
        (item_wa.is_some() && str_of(c, "waId") == item_wa.as_deref())
            || (item_user.is_some() && str_of(c, "userId") == item_user.as_deref())
    });
    let wa = item_wa.or_else(|| {
        matched
            .as_ref()
            .and_then(|c| str_of(c, "waId"))
            .map(str::to_owned)
    });
    let user = item_user.or_else(|| {
        matched
            .as_ref()
            .and_then(|c| str_of(c, "userId"))
            .map(str::to_owned)
    });
    (wa, user, matched)
}

/// `resolveCallParticipants`: the inbox, then the caller's contact inbox
/// and conversation (`None` = no identity).
async fn resolve_participants(
    deps: &Deps,
    data: &Value,
    event: &Value,
) -> Result<Option<(crate::channel::InboundInbox, i64, i64, i64)>, JobError> {
    let phone = str_of(data, "integrationIdentifier").unwrap_or_default();
    let Some((_, _, inbox)) = deps
        .channels
        .resolve_by_identifier(&deps.db, deps.realtime.as_ref(), "whatsapp", phone)
        .await
        .map_err(|e| fail(e.to_string()))?
    else {
        return Err(fail(format!("Integration not found for whatsapp: {phone}")));
    };
    let (wa, user, contact) = caller_identity(&data["payload"], event);
    let Some(source_id) = wa.or_else(|| user.clone()) else {
        return Ok(None);
    };
    let field = |k: &str| {
        contact
            .as_ref()
            .and_then(|c| str_of(c, k))
            .map(str::to_owned)
    };
    let incoming = IncomingContact {
        source_id,
        source_user_id: user,
        source_parent_user_id: field("parentUserId"),
        source_username: field("username"),
        first_name: field("name"),
        ..IncomingContact::default()
    };
    let (contact_inbox, conversation, _) =
        crate::inbound::detect_contact_and_conversation(deps, &inbox, &incoming, "inboundMessage")
            .await?;
    Ok(Some((
        inbox,
        contact_inbox.id,
        contact_inbox.contact_id,
        conversation.id,
    )))
}

/// `recordIncomingCall` (`createIfAbsent`).
async fn record_incoming(
    deps: &Deps,
    data: &Value,
    event: &Value,
) -> Result<Option<(WhatsappCall, bool, i64)>, JobError> {
    let Some((inbox, contact_inbox_id, contact_id, conversation_id)) =
        resolve_participants(deps, data, event).await?
    else {
        return Ok(None);
    };
    let input = WhatsappCallUpsertInput {
        wacid: str_of(event, "wacid").unwrap_or_default().to_owned(),
        direction: str_of(event, "direction")
            .unwrap_or("userInitiated")
            .to_owned(),
        status: "ringing".into(),
        workspace_id: inbox.workspace_id,
        inbox_id: inbox.inbox_id,
        contact_inbox_id,
        conversation_id,
        ..WhatsappCallUpsertInput::default()
    };
    let result = write(deps, move |c| calls::create_if_absent(c, &input)).await?;
    Ok(Some((result.call, result.is_new, contact_id)))
}

/// `attachBusinessInitiatedToPendingOutbound`: exact wacid, then attemptId.
async fn attach_outbound(deps: &Deps, event: &Value) -> Result<Option<WhatsappCall>, JobError> {
    let wacid = str_of(event, "wacid").unwrap_or_default().to_owned();
    if let Some(call) = by_wacid(deps, &wacid).await? {
        return Ok(Some(call));
    }
    let attempt = str_of(event, "bizOpaqueCallbackData");
    if let Some(row) = by_attempt(deps, attempt).await? {
        return write(deps, move |c| calls::attach_wacid(c, row.id, &wacid)).await;
    }
    tracing::warn!(
        wacid,
        ?attempt,
        event = "outbound-correlation-unmatched",
        "Whatsapp call (businessInitiated): no exact wacid/attemptId match to attach to"
    );
    Ok(None)
}

async fn handle_connect(deps: &Deps, data: &Value, event: &Value) -> Result<(), JobError> {
    if event["direction"] == "businessInitiated" {
        attach_outbound(deps, event).await?;
        return Ok(());
    }
    let Some((call, is_new, contact_id)) = record_incoming(deps, data, event).await? else {
        tracing::warn!(wacid = ?str_of(event, "wacid"), "Whatsapp call connect skipped: unable to resolve caller");
        return Ok(());
    };
    if is_new {
        let metadata =
            json!({ "callId": call.wacid, "conversationId": call.conversation_id.to_string() });
        emit(
            deps,
            "incomingCall",
            call.workspace_id,
            contact_id,
            metadata,
        )
        .await;
    }
    Ok(())
}

pub(crate) async fn emit(
    deps: &Deps,
    event_type: &str,
    workspace_id: i64,
    contact_id: i64,
    metadata: Value,
) {
    let event = TriggerEvent {
        event_type: event_type.to_owned(),
        workspace_id: workspace_id.to_string(),
        contact_id: contact_id.to_string(),
        metadata: metadata.as_object().cloned().unwrap_or_default(),
    };
    crate::order_payment::bus(deps)
        .publish(Event::Trigger(event))
        .await;
}

async fn handle_status(deps: &Deps, event: &Value) -> Result<(), JobError> {
    let status = match str_of(event, "status") {
        Some("RINGING") => "ringing",
        Some("ACCEPTED") => "accepted",
        Some("REJECTED") => "rejected",
        _ => return Ok(()),
    };
    let wacid = str_of(event, "wacid").unwrap_or_default().to_owned();
    let existing = match by_wacid(deps, &wacid).await? {
        Some(call) => Some(call),
        None => by_attempt(deps, str_of(event, "bizOpaqueCallbackData")).await?,
    };
    let Some(existing) = existing else {
        if status == "accepted" {
            return Err(fail(format!("whatsapp-call-status-row-not-ready: {wacid}")));
        }
        tracing::warn!(
            wacid,
            status,
            "Whatsapp call status skipped: call row not found"
        );
        return Ok(());
    };
    if existing.direction == "businessInitiated" {
        return outbound_status(deps, existing, status).await;
    }
    let current = existing.clone();
    let key = wacid.clone();
    let previous = write(deps, move |c| {
        calls::update_interim_status(c, &key, status, Some(&current))
    })
    .await?;
    // A REJECTED that lost the race against terminate: repair the card too.
    if status == "rejected" && previous.as_deref() == Some("failed") {
        let entity = json!({ "type": "whatsapp_call", "direction": existing.direction, "status": "rejected" });
        let patch = ContentBySourcePatch {
            text: Some(Some(activity_text(&entity))),
            content_attributes: Some(Some(entity)),
        };
        let (source, ws) = (activity_source_id(existing.id), existing.workspace_id);
        write(deps, move |c| {
            repo_message::update_content_by_source_id(c, &source, ws, &patch)
        })
        .await?;
    }
    Ok(())
}

/// `handleOutboundInterimStatus`.
async fn outbound_status(
    deps: &Deps,
    call: WhatsappCall,
    status: &'static str,
) -> Result<(), JobError> {
    match status {
        "ringing" => {
            let (wacid, current) = (call.wacid.clone().unwrap_or_default(), call.clone());
            write(deps, move |c| {
                calls::update_interim_status(c, &wacid, "ringing", Some(&current))
            })
            .await?;
            crate::whatsapp_voip::outbound_status_hook(deps, &call, "ringing").await?;
        }
        "accepted" => {
            let Some(agent) = call.answered_by_user_id else {
                tracing::warn!(
                    call_id = call.id,
                    "Whatsapp outbound call accepted: row has no answeredByUserId; cannot mark accepted"
                );
                return Ok(());
            };
            let id = call.id;
            write(deps, move |c| calls::mark_accepted_if_active(c, id, agent)).await?;
            crate::whatsapp_voip::outbound_status_hook(deps, &call, "accepted").await?;
        }
        _ => {
            let entity = json!({ "type": "whatsapp_call", "direction": "businessInitiated", "status": "rejected" });
            finalize(
                deps,
                Finalize {
                    call,
                    entity,
                    ended_at: None,
                    started_at: None,
                    last_error: None,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// `TERMINAL_STATUS_RULES` + `resolveTerminalEntity`.
fn terminal_entity(event: &Value, call: &WhatsappCall) -> Value {
    let duration = event["durationSeconds"].as_f64();
    let meta_answered = event["status"] == "COMPLETED"
        && (event.get("startTime").is_some_and(|v| !v.is_null())
            || duration.is_some_and(|d| d > 0.0));
    let row_answered = matches!(call.status.as_str(), "accepted" | "completed");
    let answered = meta_answered || row_answered;
    let status = if call.status == "rejected" {
        "rejected"
    } else if !answered && call.last_error.as_deref() == Some(CALL_CANCELED_BY_BUSINESS_LAST_ERROR)
    {
        "canceled"
    } else if answered {
        "completed"
    } else {
        "failed"
    };
    let duration = if meta_answered {
        Some(duration.unwrap_or(0.0))
    } else {
        duration
    };
    let mut entity =
        json!({ "type": "whatsapp_call", "direction": call.direction, "status": status });
    if let Some(d) = duration {
        entity["durationSeconds"] = json!(d.round() as i64);
    }
    entity
}

/// `formatTerminateErrors`.
fn terminate_errors(event: &Value) -> Option<String> {
    let errors = event["errors"].as_array().filter(|e| !e.is_empty())?;
    Some(
        errors
            .iter()
            .map(|e| {
                let code = e["code"].as_i64().map_or("?".to_owned(), |c| c.to_string());
                let title = str_of(e, "title")
                    .or_else(|| str_of(e, "message"))
                    .unwrap_or("");
                format!("{code}:{title}")
            })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

async fn handle_terminate(deps: &Deps, data: &Value, event: &Value) -> Result<(), JobError> {
    let wacid = str_of(event, "wacid").unwrap_or_default();
    let call = match by_wacid(deps, wacid).await? {
        Some(call) => call,
        None => match str_of(event, "direction") {
            Some("businessInitiated") => match attach_outbound(deps, event).await? {
                Some(call) => call,
                None => return Ok(()),
            },
            None => {
                tracing::warn!(wacid, "Whatsapp call terminate skipped: missing direction");
                return Ok(());
            }
            Some(_) => match record_incoming(deps, data, event).await? {
                Some((call, _, _)) => call,
                None => {
                    tracing::warn!(
                        wacid,
                        "Whatsapp call terminate skipped: unable to resolve caller"
                    );
                    return Ok(());
                }
            },
        },
    };
    let entity = terminal_entity(event, &call);
    let ended_at = unix_seconds_ms(
        str_of(event, "endTime")
            .or_else(|| str_of(event, "timestamp"))
            .or_else(|| str_of(event, "startTime")),
    );
    let started_at = Some(unix_seconds_ms(str_of(event, "startTime")));
    let last_error = terminate_errors(event);
    finalize(
        deps,
        Finalize {
            call,
            entity,
            ended_at,
            started_at,
            last_error,
        },
    )
    .await
}

// ---------------------------------------------------------------- finalize

/// `callActivitySourceId`.
pub fn activity_source_id(call_id: i64) -> String {
    format!("wacall-{call_id}")
}

/// `buildCallActivityText` (with `resolveWhatsappCallActivityLabelKey`).
pub fn activity_text(entity: &Value) -> String {
    match entity["status"].as_str() {
        Some("completed") => match entity["durationSeconds"].as_i64() {
            Some(d) => format!("Voice call · {}:{:02}", d / 60, d % 60),
            None => "Voice call".into(),
        },
        Some("canceled") => "Cancelled call".into(),
        Some("rejected") => "Declined voice call".into(),
        _ if entity["direction"] == "userInitiated" => "Missed voice call".into(),
        _ => "No answer".into(),
    }
}

/// `finalizeCallSideEffects({ call, entity })`.
pub(crate) async fn finalize_card(
    deps: &Deps,
    call: WhatsappCall,
    entity: Value,
) -> Result<(), JobError> {
    finalize(
        deps,
        Finalize {
            call,
            entity,
            ended_at: None,
            started_at: None,
            last_error: None,
        },
    )
    .await
}

struct Finalize {
    call: WhatsappCall,
    /// `{ type, direction, status, durationSeconds? }`.
    entity: Value,
    ended_at: Option<i64>,
    /// `None` = not given (never clears the column).
    started_at: Option<Option<i64>>,
    last_error: Option<String>,
}

/// `finalizeCallSideEffects`: the one progressive call card, the terminal
/// row, then (first delivery only) the activity bookkeeping and events.
async fn finalize(deps: &Deps, input: Finalize) -> Result<(), JobError> {
    let Finalize {
        call,
        mut entity,
        ended_at,
        started_at,
        last_error,
    } = input;
    let ended_at = ended_at.unwrap_or_else(now_ms);
    let (ws, inbox_id) = (call.workspace_id, call.inbox_id);
    let integration = read(deps, move |c| {
        integration_whatsapp::find_by_inbox_id_for_workspace(c, ws, inbox_id)
    })
    .await?;
    let records = integration
        .as_ref()
        .is_some_and(|i| i.call_recording_enabled);
    let transcribes = integration.as_ref().is_some_and(|i| {
        transcribes_calls(WhatsappCallMediaToggles {
            call_recording_enabled: i.call_recording_enabled,
            call_transcription_enabled: i.call_transcription_enabled,
        })
    });
    let agent_name = match call.answered_by_user_id {
        Some(user) => read(deps, move |c| {
            Ok(c.query_row(r#"SELECT "name" FROM "User" WHERE "id" = ?1"#, params![user], |r| r.get::<_, Option<String>>(0))
                .optional()?
                .flatten())
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, user, "Whatsapp call: unable to resolve agent name for call activity card");
            None
        }),
        None => None,
    };
    let status = entity["status"].as_str().unwrap_or("failed").to_owned();
    let answered_at = started_at.flatten();
    let window_opened_at = match call.direction.as_str() {
        "userInitiated" => Some(call.created_at),
        _ if status == "completed" => Some(answered_at.unwrap_or(call.created_at)),
        _ => None,
    };
    let fields = entity.as_object_mut().expect("entity is an object");
    let mut set = |k: &str, v: Value| {
        fields.insert(k.to_owned(), v);
    };
    set("callId", json!(call.id.to_string()));
    if let Some(answer) = answered_at
        .map(|a| ((a - call.created_at) as f64 / 1000.0).round() as i64)
        .filter(|s| *s >= 0)
    {
        set("answerSeconds", json!(answer));
    }
    set("hasRecording", json!(false));
    set(
        "recordingRequested",
        json!(call.recording_requested.unwrap_or(records)),
    );
    set("transcriptionRequested", json!(transcribes));
    set("hasTranscript", json!(false));
    set("hasSummary", json!(false));
    set("recordingExpired", json!(false));
    set(
        "recordingUnavailable",
        json!(call.recording_requested == Some(false) && records),
    );
    if let Some(agent) = call.answered_by_user_id {
        set("agentUserId", json!(agent.to_string()));
        if let Some(name) = agent_name.filter(|n| !n.is_empty()) {
            set("agentName", json!(name));
        }
    }
    if let Some(at) = window_opened_at {
        set("customerServiceWindowOpenedAt", json!(iso(at)));
    }
    if let Some(reason) = last_error.as_ref().filter(|r| !r.is_empty()) {
        set("failureReason", json!(reason));
    }
    tracing::info!(call_id = call.id, wacid = ?call.wacid, status, records, transcribes, "[wa-call-media] call card flags stamped");

    let message = CreateMessageInput {
        id: Some(new_id()),
        conversation_id: call.conversation_id,
        contact_inbox_id: call.contact_inbox_id,
        workspace_id: call.workspace_id,
        source_id: Some(activity_source_id(call.id)),
        sender_type: "system".into(),
        message_type: "activity".into(),
        text: Some(activity_text(&entity)),
        content_type: "text".into(),
        content_attributes: Some(entity.clone()),
        created_at: Some(ended_at),
        ..CreateMessageInput::default()
    };
    let saved = write(deps, move |c| repo_message::create_or_update(c, &message)).await?;
    let terminal = match status.as_str() {
        "completed" => WhatsappCallTerminalStatus::Completed,
        "rejected" => WhatsappCallTerminalStatus::Rejected,
        _ => WhatsappCallTerminalStatus::Failed,
    };
    let outcome = resolve_whatsapp_call_outcome(terminal, status == "canceled");
    let finalize = FinalizeInput {
        id: call.id,
        status: match terminal {
            WhatsappCallTerminalStatus::Completed => "completed",
            WhatsappCallTerminalStatus::Rejected => "rejected",
            WhatsappCallTerminalStatus::Failed => "failed",
        }
        .into(),
        outcome: outcome.as_str().into(),
        started_at,
        ended_at: Some(Some(ended_at)),
        duration_seconds: Some(entity["durationSeconds"].as_i64()),
        message_id: Some(Some(saved.message.id)),
        last_error: last_error.clone().map(Some),
        answered_by_user_id: None,
        current: Some(call.clone()),
    };
    write(deps, move |c| calls::finalize_by_id(c, &finalize)).await?;
    // Transport cleanup on every delivery (each step is idempotent).
    let persisted = match terminal {
        WhatsappCallTerminalStatus::Completed => "completed",
        WhatsappCallTerminalStatus::Rejected => "rejected",
        WhatsappCallTerminalStatus::Failed => "failed",
    };
    crate::whatsapp_voip::emit_call_ended_to_agent(deps, &call, persisted).await?;
    if !saved.is_new {
        return Ok(());
    }

    let at = saved.message.created_at;
    let inbound = call.direction == "userInitiated";
    let (conversation_id, contact_inbox_id) = (call.conversation_id, call.contact_inbox_id);
    let contact_id = write(deps, move |c| {
        c.execute(
            r#"UPDATE "Conversation" SET "lastActivityAt" = ?1,
                 "contactRepliedAt" = CASE WHEN ?2 THEN ?1 ELSE "contactRepliedAt" END, "updatedAt" = ?3
               WHERE "id" = ?4 AND "workspaceId" = ?5"#,
            params![at, inbound, now_ms(), conversation_id, ws],
        )?;
        c.execute(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = ?1,
                 "lastIncomingMessageAt" = COALESCE(?2, "lastIncomingMessageAt"), "updatedAt" = ?3
               WHERE "id" = ?4"#,
            params![at, window_opened_at, now_ms(), contact_inbox_id],
        )?;
        Ok(c.query_row(r#"SELECT "contactId" FROM "ContactInbox" WHERE "id" = ?1"#, params![contact_inbox_id], |r| r.get::<_, i64>(0))
            .optional()?)
    })
    .await?;
    if let Some(conversation) = crate::load_conversation(&deps.db, call.conversation_id).await? {
        let row = message_json(&MessageWithAttachments {
            message: saved.message.clone(),
            attachment_count: None,
            attachments: Vec::new(),
        });
        crate::chat::queue_event(
            deps,
            &conversation,
            call.inbox_id,
            EventType::MessageCreated,
            row,
        )
        .await;
    }
    let Some(contact_id) = contact_id else {
        return Ok(());
    };
    let correlation = call
        .wacid
        .clone()
        .or_else(|| call.attempt_id.clone())
        .unwrap_or_else(|| call.id.to_string());
    if status == "completed" {
        let mut metadata = Map::new();
        metadata.insert("callId".into(), json!(correlation));
        if let Some(d) = entity["durationSeconds"].as_i64() {
            metadata.insert("durationSeconds".into(), json!(d));
        }
        emit(deps, "callEnded", ws, contact_id, Value::Object(metadata)).await;
    } else if inbound {
        let metadata =
            json!({ "callId": correlation, "conversationId": call.conversation_id.to_string() });
        emit(deps, "missedAudioCall", ws, contact_id, metadata).await;
    }
    Ok(())
}

#[cfg(test)]
#[path = "whatsapp_call_tests.rs"]
mod tests;
