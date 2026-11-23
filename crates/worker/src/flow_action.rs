//! `runFlowPostback` / `runFlowQuickReply` (`integration/handlers/flow.ts`
//! `runFlowAction`): a tapped button or quick reply runs the flow target it
//! names.
//! A tap on a rich-response button (AI reply, poll message) runs its stored
//! payload first (`tryRunRichButtonFallback`). A version-pinned payload
//! keeps its version for the continuations.

use chatbotx_business::conversation_challenge::{ClearQuickReply, clear_quick_reply_challenge};
use chatbotx_core::flow_config::resolve_flow_action_target;
use chatbotx_core::flow_config::steps::find_whatsapp_flow_step_by_button_id;
use chatbotx_core::flow_config::util::decode_button_payload;
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use crate::{Deps, fail, id_of, load_contact_inbox, load_conversation, str_of};

pub const RUN_FLOW_POSTBACK: &str = "runFlowPostback";
pub const RUN_FLOW_QUICK_REPLY: &str = "runFlowQuickReply";

/// `flowActionHandlers[kind]`: the trigger types of the analytics rows.
fn trigger_types(name: &str) -> (&'static str, &'static str) {
    if name == RUN_FLOW_POSTBACK {
        ("contact_postback", "contact_postback_failed")
    } else {
        ("contact_quick_reply", "contact_quick_reply_failed")
    }
}

/// `applyWhatsappFlowResponseSideEffects`: a WhatsApp Flow form answer
/// fills the mapped contact fields, then scores the "survey" rule.
async fn apply_whatsapp_flow_response(
    deps: &Deps,
    conversation: &chatbotx_db::repo::models::Conversation,
    contact_inbox: &chatbotx_db::repo::models::ContactInbox,
    step: &Value,
    response: &serde_json::Map<String, Value>,
    data: &Value,
) -> Result<(), JobError> {
    let Some(flow_source_id) = str_of(&step["flow"], "sourceId").filter(|s| !s.is_empty()) else {
        tracing::warn!(flow_id = %step["flow"]["id"], "[whatsapp-flow-response] step.flow.sourceId missing");
        return Ok(());
    };
    chatbotx_business::whatsapp_flow_response::apply_response(
        &deps.db,
        chatbotx_business::whatsapp_flow_response::ApplyInput {
            workspace_id: conversation.workspace_id,
            contact_id: conversation.contact_id,
            inbox_id: Some(contact_inbox.inbox_id),
            integration_whatsapp_id: None,
            flow_source_id: flow_source_id.to_owned(),
            field_mappings: step["flow"]["fieldMappings"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            flow_response: response.clone(),
            flow_files: data["payload"]["flowFiles"].as_object().cloned(),
        },
    )
    .await
    .map_err(|e| fail(e.to_string()))?;
    crate::template_flow_response::award_survey(
        deps,
        conversation.workspace_id,
        conversation.contact_id,
        contact_inbox,
        data.get("messageId").and_then(id_of),
    )
    .await;
    Ok(())
}

/// `runFlowAction`.
pub async fn run_flow_action(
    deps: &Deps,
    name: &str,
    data: &Value,
    flow_execution_key: String,
) -> Result<(), JobError> {
    let conversation_id = data
        .get("conversationId")
        .and_then(id_of)
        .ok_or_else(|| fail("Conversation not found"))?;
    let conversation = load_conversation(&deps.db, conversation_id)
        .await?
        .ok_or_else(|| fail("Conversation not found"))?;
    let contact_inbox_id = data
        .get("contactInboxId")
        .and_then(id_of)
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let contact_inbox = load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id)
        .await?
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let ws = conversation.workspace_id;
    // Every tap ends a pending quick reply retry, whatever it resolves to.
    if let Err(err) =
        clear_quick_reply_challenge(&deps.db, ws, conversation.id, ClearQuickReply::default()).await
    {
        tracing::warn!(err = %err, conversation_id = conversation.id, "clearQuickReplyChallengeOnTap: failed to clear challenge");
    }
    let action = str_of(data, "action").unwrap_or("");
    // Bare flow ids (Messenger ad payloads) only on Messenger, read from the
    // stored contact inbox.
    let Some(payload) = decode_button_payload(action, contact_inbox.channel == "messenger") else {
        tracing::warn!(
            conversation_id = conversation.id,
            action,
            "{name}: could not decode action payload, skipping"
        );
        return Ok(());
    };
    if let Some(button_id) = payload.button_id.as_deref() {
        let message_id = data.get("messageId").and_then(id_of);
        let fallback = crate::rich_response::try_run_rich_button_fallback(
            deps,
            button_id,
            &conversation,
            &contact_inbox,
            &payload.flow_id,
            message_id,
        )
        .await?;
        if let crate::rich_response::Fallback::Handled {
            enqueue_automated_response,
        } = fallback
        {
            if enqueue_automated_response && let Some(message_id) = message_id {
                // `automatedResponseService.enqueue`: no keyword gate.
                crate::inbound::enqueue_automated_response_for(
                    deps,
                    (conversation.id, contact_inbox.id, ws),
                    message_id,
                    Some(String::new()),
                    Vec::new(),
                )
                .await?;
            }
            return Ok(());
        }
    }
    let Some(flow_id) = payload.flow_id.parse::<i64>().ok() else {
        tracing::warn!(
            conversation_id = conversation.id,
            "{name}: invalid flow id, skipping"
        );
        return Ok(());
    };
    let flow_version_id = payload
        .flow_version_id
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok());
    let Some((flow_version, _)) =
        crate::flow::find_flow_version(deps, flow_id, flow_version_id, ws).await?
    else {
        tracing::warn!(
            conversation_id = conversation.id,
            flow_id,
            "{name}: flow version could not be resolved, skipping"
        );
        return Ok(());
    };
    let mut job = json!({
        "conversationId": conversation.id.to_string(),
        "contactInboxId": contact_inbox.id.to_string(),
        "flowId": flow_id.to_string(),
    });
    if let Some(version) = flow_version_id {
        job["flowVersionId"] = json!(version.to_string());
    }
    let from_status = str_of(data, "webhookType") == Some(crate::inbound::MESSAGE_STATUS);
    let Some(button_id) = payload.button_id.clone() else {
        if let Some(node) = &payload.node_id {
            job["nodeId"] = json!(node);
        }
        return crate::flow::run_flow_node(deps, &job, flow_execution_key).await;
    };
    let nodes = flow_version.nodes.0.as_array().cloned().unwrap_or_default();
    // The channel cannot tell a step button from a node quick reply: the flow decides.
    let Some(target) = resolve_flow_action_target(&nodes, &button_id) else {
        tracing::warn!(
            conversation_id = conversation.id,
            flow_id,
            button_id,
            "{name}: action matches no button or quick reply, skipping"
        );
        return Ok(());
    };
    let target_node_id = target.node_id.clone().unwrap_or_default();
    if let Some(response) = data["payload"]["waFlowResponse"].as_object()
        && let Some(step) = find_whatsapp_flow_step_by_button_id(&nodes, &button_id)
    {
        apply_whatsapp_flow_response(deps, &conversation, &contact_inbox, step, response, data)
            .await?;
    }
    if !from_status && let Some(buses) = deps.events.as_ref() {
        let event = json!({
            "nodeId": target_node_id,
            "context": {
                "workspaceId": ws.to_string(),
                "contactId": conversation.contact_id.to_string(),
                "conversationId": conversation.id.to_string(),
                "channel": contact_inbox.channel,
                "contactInboxId": contact_inbox.id.to_string(),
            },
            "action": {
                "flowId": flow_id.to_string(),
                "buttonId": button_id,
                "broadcastId": payload.broadcast_id,
                "sequenceStepId": payload.sequence_step_id.clone().unwrap_or_default(),
                "commentAutomationId": payload.comment_automation_id,
                "clickType": if target.target_type == "button" { "button" } else { "quick_reply" },
            },
            "occurredAt": iso(now_ms()),
        });
        if let Some(sent) = chatbotx_jobs::events::emit(buses, "flow:clicked", event)
            && let Err(err) = sent.await
        {
            tracing::warn!(err = %err, conversation_id = conversation.id, "flow:clicked emit failed");
        }
    }
    job["targetType"] = json!(target.target_type);
    job["targetId"] = json!(button_id);
    job["nodeId"] = json!(target_node_id);
    // A tap is the contact's message; a delivery-status webhook is not.
    job["startedByContactMessage"] = json!(!from_status);
    let start = now_ms();
    let result = crate::flow::run_flow_node(deps, &job, flow_execution_key).await;
    if let Some(message_id) = str_of(data, "messageId") {
        let (ok_type, failed_type) = trigger_types(name);
        let mut metadata = json!({
            "latency": now_ms() - start,
            "flowId": flow_id.to_string(),
            "triggerContext": { "triggerSource": "worker", "triggerHandler": name,
                "triggerType": if result.is_ok() { ok_type } else { failed_type } },
        });
        if result.is_err() {
            metadata["fallbackReason"] = json!("handler_error_to_fallback");
        }
        crate::events::emit_bot_received(
            deps.events.as_ref(),
            ws,
            conversation.id,
            &contact_inbox,
            message_id,
            json!({
                "hasResponse": result.is_ok(),
                "responseType": "flow",
                "routeType": "flow",
                "result": if result.is_ok() { "success" } else { "fallback" },
                "aiProvider": "none",
                "metadata": metadata,
            }),
        )
        .await;
    }
    result
}
