//! `chat/handlers/send-flow-step.ts` (`sendFlowStep`, `sendChatMessage`,
//! `convertButtonsToTemplate`) and `chat/utils/stored-step-content.ts`:
//! a flow step becomes a stored outgoing message, then the channel sends it.

use chatbotx_business::message::message_json;
use chatbotx_business::product_message::service::prepare_outgoing;
use chatbotx_core::flow_config::{
    ButtonPayload, CHANNEL_DELIVERABLE_STEP_TYPES, append_code_to_magic_link,
    encode_button_payload, extract_metadata, get_channel_flow_policy, is_native_button_type,
    is_proactive_outbound_metadata, sends_as_native_copy_button,
};
use chatbotx_core::utils::spintax::apply_spintax;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::message::{
    self as repo_message, CreateMessageInput, MessageWithAttachments,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Map, Value, json};

use crate::channel::{OutgoingContact, SendFailure, SendFlowStep};
use crate::chat::{
    is_bulk_outbound, mark_read_after_delivery, persist_send_error, record_outbound_message_sent,
    send_message_to_channel,
};
use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

/// How a delivered flow step is recorded on its Message row.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredStepContent {
    pub content_type: &'static str,
    pub text: Option<String>,
    pub content_attributes: Option<Value>,
}

fn text_field(step: &Value, key: &str) -> Option<String> {
    step.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// `String(x)` of a JSON number or string.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "undefined".to_owned(),
        other => other.to_string(),
    }
}

/// `resolveStoredStepContent`.
pub fn resolve_stored_step_content(step: &Value) -> StoredStepContent {
    let text_only = |text: Option<String>| StoredStepContent {
        content_type: "text",
        text,
        content_attributes: None,
    };
    match str_of(step, "stepType").unwrap_or("") {
        "sendText" | "whatsappCallButton" => text_only(text_field(step, "text")),
        "whatsappLocation" => {
            let name = text_field(step, "name").filter(|s| !s.is_empty());
            let address = text_field(step, "address").filter(|s| !s.is_empty());
            let latitude = js_string(&step["latitude"]);
            let longitude = js_string(&step["longitude"]);
            let label = [name.clone(), address.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(": ");
            let mut attributes = json!({ "latitude": latitude, "longitude": longitude });
            if let Some(name) = name {
                attributes["name"] = json!(name);
            }
            if let Some(address) = address {
                attributes["address"] = json!(address);
            }
            StoredStepContent {
                content_type: "location",
                text: Some(if label.is_empty() {
                    format!("{latitude}, {longitude}")
                } else {
                    label
                }),
                content_attributes: Some(attributes),
            }
        }
        "sendViewCatalog" => StoredStepContent {
            content_type: "text",
            text: text_field(step, "text"),
            content_attributes: Some(json!({ "viewCatalog": { "footer": step.get("footer") } })),
        },
        _ => text_only(None),
    }
}

/// `isBlankTextCarrierStep`.
fn is_blank_text_carrier_step(step: &Value) -> bool {
    match str_of(step, "stepType").unwrap_or("") {
        "sendText" | "whatsappCallButton" | "sendViewCatalog" => {
            str_of(step, "text").unwrap_or("").trim().is_empty()
        }
        "sendQuickReply" => str_of(step, "message").unwrap_or("").trim().is_empty(),
        _ => false,
    }
}

/// `convertButtonsToTemplate`: each authored button as the stored /
/// channel-neutral `MessageButtonTemplate`.
pub fn convert_buttons_to_template(
    channel: &str,
    flow_id: &str,
    flow_version_id: Option<&str>,
    buttons: &[Value],
    metadata: Option<&Value>,
    contact_inbox_id: Option<&str>,
) -> Vec<Value> {
    let dials = is_native_button_type(Some(channel), "callPhoneNumber");
    let broadcast_id = extract_metadata("broadcastId", metadata);
    let sequence_step_id = extract_metadata("sequenceStepId", metadata);
    let comment_automation_id = extract_metadata("commentAutomationId", metadata);
    buttons
        .iter()
        .map(|button| {
            let payload = encode_button_payload(&ButtonPayload {
                flow_id: flow_id.to_owned(),
                flow_version_id: flow_version_id.map(str::to_owned),
                button_id: text_field(button, "id"),
                broadcast_id: broadcast_id.clone(),
                sequence_step_id: sequence_step_id.clone(),
                contact_inbox_id: contact_inbox_id.map(str::to_owned),
                comment_automation_id: comment_automation_id.clone(),
                node_id: None,
            });
            let before = &button["beforeStep"];
            let base = |button_type: &str| {
                let mut out = Map::new();
                out.insert("id".into(), button["id"].clone());
                out.insert("label".into(), button["label"].clone());
                out.insert("buttonType".into(), json!(button_type));
                out
            };
            let mut out = match str_of(button, "buttonType") {
                Some("openWebsite") => {
                    let mut out = base("url");
                    let url = str_of(before, "url").unwrap_or_default();
                    out.insert(
                        "url".into(),
                        json!(append_code_to_magic_link(url, &payload)),
                    );
                    out
                }
                Some("callPhoneNumber") if dials => {
                    let mut out = base("phoneNumber");
                    out.insert("phoneNumber".into(), before["phoneNumber"].clone());
                    out
                }
                _ if sends_as_native_copy_button(Some(channel), button) => {
                    let mut out = base("copyText");
                    out.insert("copyText".into(), before["text"].clone());
                    out
                }
                _ => base("postback"),
            };
            out.insert("postback".into(), json!(payload));
            Value::Object(out)
        })
        .collect()
}

/// `findTargetContactInbox`: the job's contact inbox, else the contact's
/// most recent one in the workspace.
async fn find_target_contact_inbox(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox_id: Option<i64>,
) -> Result<Option<ContactInbox>, JobError> {
    if let Some(id) = contact_inbox_id {
        return load_contact_inbox(&deps.db, id, conversation.contact_id).await;
    }
    let (contact_id, workspace_id) = (conversation.contact_id, conversation.workspace_id);
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        r#"SELECT {} FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
                           WHERE ci."contactId" = ?1 AND i."workspaceId" = ?2
                           ORDER BY coalesce(ci."lastMessageAt", 0) DESC, ci."id" ASC LIMIT 1"#,
                        ContactInbox::columns_sql(Some("ci"))
                    ),
                    params![contact_id, workspace_id],
                    ContactInbox::from_row,
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// Spintax in every string leaf (`resolveContactVariablesDeep(..., {
/// spintax: true })` without the `{{variable}}` pass, see the spec).
fn spin(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(apply_spintax(text)),
        Value::Array(items) => Value::Array(items.iter().map(spin).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), spin(v))).collect())
        }
        other => other.clone(),
    }
}

pub(crate) async fn store_message(
    deps: &Deps,
    input: CreateMessageInput,
) -> Result<MessageWithAttachments, JobError> {
    deps.db
        .write(move |tx| {
            Ok(MessageWithAttachments {
                message: repo_message::create(tx, &input).map_err(crate::repo_err)?,
                attachment_count: None,
                attachments: Vec::new(),
            })
        })
        .await
        .map_err(db_fail)
}

/// `conversationService.recordOutboundFlowStep` (`flow_step` = the step
/// ids) / `recordOutboundMessageActivity` (`None`): the contact inbox
/// tracking, the conversation activity and, when `close_window`, the bot
/// close of the DM waiting window (`humanReplyWindowSet({ close, actor:
/// "bot" })`: a DM only, the bot enabled, a window opened at or before `at`;
/// the stale-event marker `awaitingHumanReplyClosedAt` stays).
/// The bot-close condition (`?5` = at, `?6` = close requested).
const CLOSES: &str = r#"(?6 AND "sourceId" IS NULL AND "botEnabled" AND "awaitingHumanReplySince" IS NOT NULL AND "awaitingHumanReplySince" <= ?5)"#;

pub(crate) async fn record_outbound(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    at: i64,
    bump_activity: bool,
    close_window: bool,
    flow_step: Option<Option<String>>,
) {
    record_outbound_message_sent(deps, contact_inbox, conversation.workspace_id, at).await;
    let (id, workspace_id, last_step) = (
        conversation.id,
        conversation.workspace_id,
        conversation.current_step.clone(),
    );
    let (is_flow_step, step_id) = (flow_step.is_some(), flow_step.flatten());
    let result = deps
        .db
        .write(move |tx| {
            tx.execute(
                &format!(
                    r#"UPDATE "Conversation"
                       SET "lastStep" = CASE WHEN ?1 THEN ?2 ELSE "lastStep" END,
                           "currentStep" = CASE WHEN ?1 THEN ?3 ELSE "currentStep" END,
                           "lastActivityAt" = CASE WHEN ?4 THEN ?5 ELSE "lastActivityAt" END,
                           "awaitingHumanReplySince" = CASE WHEN {CLOSES} THEN NULL ELSE "awaitingHumanReplySince" END,
                           "responseDueAt" = CASE WHEN {CLOSES} THEN NULL ELSE "responseDueAt" END,
                           "responseSlaBreachingAt" = CASE WHEN {CLOSES} THEN NULL ELSE "responseSlaBreachingAt" END,
                           "responseSlaTargetSeconds" = CASE WHEN {CLOSES} THEN NULL ELSE "responseSlaTargetSeconds" END,
                           "updatedAt" = ?7
                       WHERE "id" = ?8 AND "workspaceId" = ?9"#
                ),
                params![
                    is_flow_step,
                    last_step,
                    step_id,
                    bump_activity,
                    at,
                    close_window,
                    now_ms(),
                    id,
                    workspace_id
                ],
            )?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, conversation_id = id, "recordOutboundFlowStep failed");
    }
}

pub(crate) fn queue_message_created(
    deps: &Deps,
    conversation: &Conversation,
    inbox_id: i64,
    message: &Value,
) {
    let Some(realtime) = deps.realtime.clone() else {
        return;
    };
    let route = crate::chat::route_of(conversation, inbox_id);
    let workspace = conversation.workspace_id.to_string();
    let event = Envelope::routed(EventType::MessageCreated, message.clone(), route);
    tokio::spawn(async move { realtime.queue_workspace_event(&workspace, event).await });
}

/// `ChatJobAction.sendFlowMessage` (`sendFlowStep`). A send failure is
/// recorded on the message and swallowed (the job completes).
pub async fn send_flow_step(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(conversation_id) = data.get("conversationId").and_then(id_of) else {
        return Err(JobError::unrecoverable("sendFlowStep: no conversationId"));
    };
    let Some(conversation) = load_conversation(&deps.db, conversation_id).await? else {
        return Ok(());
    };
    let contact_inbox_id = data.get("contactInboxId").and_then(id_of);
    let Some(contact_inbox) =
        find_target_contact_inbox(deps, &conversation, contact_inbox_id).await?
    else {
        return Ok(());
    };
    let step = data.get("step").cloned().unwrap_or(Value::Null);
    let step_type = str_of(&step, "stepType").unwrap_or("").to_owned();
    let flow_id = data.get("flowId").map(js_string).unwrap_or_default();
    let flow_version_id = str_of(data, "flowVersionId").map(str::to_owned);
    let metadata = data.get("metadata").filter(|m| !m.is_null()).cloned();
    let is_bulk = is_bulk_outbound(metadata.as_ref(), data["isBulkBroadcast"] == json!(true));

    if get_channel_flow_policy(Some(&contact_inbox.channel))
        .and_then(|p| p.steps.get(step_type.as_str()))
        .is_some_and(|support| *support == "unsupported")
    {
        tracing::debug!(conversation_id, step_type = %step_type, channel = %contact_inbox.channel, "Skipping a flow step the channel does not support");
        return Ok(());
    }
    // `processWhatsappTemplate`: the template path stores and sends itself
    // (variables, payment checks, flow buttons); its error is logged only.
    if step_type == "sendWaTemplateMessage" {
        if contact_inbox.channel != "whatsapp" {
            return Ok(());
        }
        let template = &step["template"];
        let input = crate::wa_template::ProcessTemplate {
            flow: Some(crate::wa_template::TemplateFlow {
                id: flow_id.clone(),
                version_id: flow_version_id.clone(),
                buttons: step["buttons"].as_array().cloned().unwrap_or_default(),
            }),
            step: Some((
                str_of(&step, "id").unwrap_or("").to_owned(),
                str_of(&step, "nodeId").unwrap_or("").to_owned(),
            )),
            metadata: metadata.clone(),
            is_bulk_broadcast: data["isBulkBroadcast"] == json!(true),
            sent_by_user_id: data
                .get("sentByUserId")
                .filter(|v| !v.is_null())
                .map(js_string),
            template: json!({ "id": template["id"], "name": template["name"],
                "language": template["language"], "params": template["params"] }),
            ..crate::wa_template::ProcessTemplate::default()
        };
        if let Err(error) =
            crate::wa_template::process_template(deps, &conversation, &contact_inbox, input).await
        {
            tracing::error!(err = %error.message(), "sendFlowStep WhatsApp template error for conversationId: {conversation_id}");
        }
        return Ok(());
    }
    if !CHANNEL_DELIVERABLE_STEP_TYPES.contains(&step_type.as_str()) {
        tracing::debug!(conversation_id, step_type = %step_type, "Skipping non-deliverable flow step");
        return Ok(());
    }
    let resolved = match &deps.variables {
        Some(variables) => {
            let resolved = variables
                .resolve_step(
                    &conversation,
                    &contact_inbox,
                    &step,
                    str_of(data, "appointmentId").map(str::to_owned),
                    data.get("sentByUserId")
                        .filter(|v| !v.is_null())
                        .map(js_string),
                )
                .await;
            match resolved {
                Ok(resolved) => resolved,
                Err(error) => {
                    // The original's variable pass throws before its try block.
                    return Err(fail(format!("sendFlowStep variables: {error}")));
                }
            }
        }
        None => spin(&step),
    };
    // `resolveCollectionStep`: the collection becomes its product cards for
    // this contact only now, after the variable pass (product names are the
    // store's data, never spun).
    let resolved = if step_type == "sendCollection" {
        match chatbotx_business::webshop::collection::resolve_collection_step(
            &deps.db,
            conversation.workspace_id,
            conversation.contact_id,
            Some(contact_inbox.id),
            &deps.app_url,
            &resolved,
        )
        .await
        .map_err(|e| fail(e.message()))?
        {
            Some(step) => step,
            None => {
                tracing::debug!(
                    conversation_id,
                    "Skipping a product collection that matched no product"
                );
                return Ok(());
            }
        }
    } else {
        resolved
    };
    if is_blank_text_carrier_step(&resolved) {
        tracing::warn!(conversation_id, step_type = %step_type, "Skipping blank text flow step");
        return Ok(());
    }
    let stored = resolve_stored_step_content(&resolved);
    // `resolveProductMessageForStep`: before any Message row, so a step
    // that cannot be sent leaves no phantom message. A failure is the
    // terminal catch (logged, the job completes).
    let product = if step_type == "sendProducts" {
        match prepare_outgoing(
            &deps.db,
            conversation.workspace_id,
            &contact_inbox.channel,
            &resolved,
            false,
        )
        .await
        {
            Ok(prepared) => {
                if !prepared.missing.is_empty() {
                    // ponytail: the provider ErrorLog row (`logProviderErrorForChannel`) comes with Q.W2.
                    tracing::warn!(conversation_id, flow_id = %flow_id, product_ids = ?prepared.missing, "Product message sent without products that are no longer sendable");
                }
                Some(prepared)
            }
            Err(error) => {
                tracing::error!(err = %error, "sendFlowStep error for conversationId: {conversation_id}");
                return Ok(());
            }
        }
    } else {
        None
    };
    // Open Mini App buttons become link buttons first, so everything below
    // only ever sees `openWebsite` (`resolveMiniAppLinks`).
    let mut resolved = resolved;
    let mut raw_quick_replies = data.get("quickReplies").cloned();
    let link_context = crate::mini_app_buttons::LinkContext {
        workspace_id: conversation.workspace_id,
        contact_id: conversation.contact_id,
        conversation_id: conversation.id,
        contact_inbox_id: contact_inbox.id,
        flow_id: &flow_id,
        flow_version_id: flow_version_id.as_deref(),
        metadata: metadata.as_ref(),
        app_url: &deps.app_url,
    };
    if step_type == "sendMessengerTemplateMessage" {
        // `processMessengerTemplate` takes the step's raw buttons: only the
        // template's URL suffix changes (`applyMiniAppResumeToTemplateParams`).
        crate::mini_app_buttons::resolve_template_mini_app_resume(
            deps,
            &link_context,
            contact_inbox.inbox_id,
            &mut resolved,
        )
        .await;
    } else {
        crate::mini_app_buttons::resolve_mini_app_links(
            deps,
            &link_context,
            &mut resolved,
            raw_quick_replies.as_mut(),
        )
        .await;
    }
    let channel = contact_inbox.channel.clone();
    let contact_inbox_text = contact_inbox.id.to_string();
    let convert = |buttons: &[Value]| {
        convert_buttons_to_template(
            &channel,
            &flow_id,
            flow_version_id.as_deref(),
            buttons,
            metadata.as_ref(),
            Some(&contact_inbox_text),
        )
    };
    let quick_replies: Vec<Value> = raw_quick_replies
        .as_ref()
        .and_then(Value::as_array)
        .map(|b| convert(b))
        .unwrap_or_default()
        .into_iter()
        .chain(
            data.get("presetQuickReplies")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        )
        .collect();
    let step_buttons = resolved
        .get("buttons")
        .and_then(Value::as_array)
        .map(|b| convert(b))
        .unwrap_or_default();

    let mut attributes = match stored.content_attributes.clone() {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in [
        ("metadata", metadata.clone().unwrap_or(Value::Null)),
        (
            "richResponse",
            data.get("richResponse").cloned().unwrap_or(Value::Null),
        ),
        ("stepId", resolved["id"].clone()),
        ("nodeId", resolved["nodeId"].clone()),
        ("flowId", json!(flow_id)),
        ("flowVersionId", json!(flow_version_id)),
    ] {
        if !value.is_null() {
            attributes.insert(key.into(), value);
        }
    }
    if let Some(product) = &product
        && let Value::Object(base) = product.content_attributes.clone()
    {
        // `{ ...prepared.contentAttributes, ...contentAttributes }`.
        let mut merged = base;
        merged.extend(attributes);
        attributes = merged;
    }
    let display_buttons: Vec<Value> = step_buttons.iter().chain(&quick_replies).cloned().collect();
    // `{ type, payload, ...contentAttributes }`: the step's own keys win.
    let with_template = |attributes: Map<String, Value>, payload: Value| {
        let mut out = Map::new();
        out.insert("type".into(), json!("template"));
        out.insert("payload".into(), payload);
        out.extend(attributes);
        out
    };
    if !display_buttons.is_empty() {
        attributes = with_template(
            attributes,
            json!({ "templateType": "button", "buttons": display_buttons }),
        );
    }
    if let Some(cards) = resolved
        .get("cards")
        .and_then(Value::as_array)
        .filter(|c| !c.is_empty())
    {
        let cards: Vec<Value> = cards
            .iter()
            .map(|card| {
                let mut card = card.clone();
                if let Some(buttons) = card.get("buttons").and_then(Value::as_array) {
                    card["buttons"] = Value::Array(convert(buttons));
                }
                card
            })
            .collect();
        attributes = with_template(
            attributes,
            json!({ "templateType": "carousel", "cards": cards }),
        );
    }
    let comment_anchor = data.get("commentAnchor").filter(|a| a.is_object()).cloned();
    let is_public_reply = comment_anchor
        .as_ref()
        .and_then(|a| str_of(a, "replyChannel"))
        == Some("public");
    if let Some(anchor) = &comment_anchor
        && (is_public_reply || anchor.get("automationId").is_some_and(|v| !v.is_null()))
    {
        attributes.insert("replyToCommentId".into(), anchor["commentId"].clone());
        if !anchor["automationId"].is_null() {
            attributes.insert(
                "commentAutomation".into(),
                json!({ "automationId": anchor["automationId"], "replyChannel": anchor["replyChannel"] }),
            );
        }
    }

    let created_at = now_ms();
    // ponytail: media steps keep the authored URL; the copy into storage
    // (`uploadFileFromUrl`) and attachment rows come with the media unit.
    let message = store_message(
        deps,
        CreateMessageInput {
            contact_inbox_id: contact_inbox.id,
            conversation_id: conversation.id,
            workspace_id: conversation.workspace_id,
            content_attributes: Some(Value::Object(attributes)),
            content_type: stored.content_type.to_owned(),
            message_type: "outgoing".into(),
            sender_type: "bot".into(),
            text: stored
                .text
                .clone()
                .or_else(|| product.as_ref().and_then(|p| p.text.clone())),
            r#type: Some(
                if is_public_reply {
                    "comment"
                } else {
                    "message"
                }
                .into(),
            ),
            created_at: Some(created_at),
            ..CreateMessageInput::default()
        },
    )
    .await?;
    let message_value = message_json(&message);
    // An inbox-originated flow send is an agent reply: the producer claim
    // owns the close, so the worker must not consume the window first.
    let close_window = !(str_of(data, "sendFrom") == Some("inbox")
        || metadata
            .as_ref()
            .is_some_and(is_proactive_outbound_metadata));
    record_outbound(
        deps,
        &conversation,
        &contact_inbox,
        created_at,
        !is_bulk,
        close_window,
        Some(text_field(&resolved, "id")),
    )
    .await;

    let quick_replies = (!quick_replies.is_empty()).then_some(Value::Array(quick_replies));
    let send = if is_public_reply {
        let send_data = json!({
            "conversation": { "id": conversation.id.to_string() },
            "contactInbox": { "id": contact_inbox.id.to_string() },
            "message": message_value,
            "quickReplies": quick_replies,
            "metadata": metadata,
            "isBulkBroadcast": data.get("isBulkBroadcast"),
            "sendFrom": data.get("sendFrom"),
        });
        send_message_to_channel(deps, &send_data, 0, true)
            .await
            .map_err(|e| SendFailure::Other(e.to_string()))
    } else {
        send_flow_step_to_channel(
            deps,
            &conversation,
            &contact_inbox,
            SendFlowStep {
                contact: OutgoingContact::new(&contact_inbox, &conversation),
                flow_id: flow_id.clone(),
                flow_version_id: flow_version_id.clone(),
                step: resolved.clone(),
                product_message: product.map(|p| p.message),
                quick_replies,
                metadata: metadata.clone(),
                rich_response: data.get("richResponse").filter(|v| !v.is_null()).cloned(),
                send_from: str_of(data, "sendFrom").map(str::to_owned),
                comment_anchor: comment_anchor.filter(|a| {
                    matches!(contact_inbox.channel.as_str(), "messenger" | "instagram")
                        && str_of(a, "replyChannel") == Some("private")
                }),
                message: message_value.clone(),
            },
            &message,
        )
        .await
    };
    if !is_bulk {
        queue_message_created(deps, &conversation, contact_inbox.inbox_id, &message_value);
    }
    if contact_inbox.channel == "webchat"
        && let Some(realtime) = &deps.realtime
        && let Err(err) = realtime.publish_guest_event(
            &conversation.workspace_id.to_string(),
            &contact_inbox.source_id,
            Envelope::new(EventType::MessageCreated, message_value.clone()),
        )
    {
        tracing::error!(err = %err, workspace_id = conversation.workspace_id, "Failed to publish guest realtime event for flow step");
    }
    let ctx = crate::events::MessageContext::of(&conversation, &contact_inbox);
    let step_ids: Map<String, Value> = [("stepId", "id"), ("nodeId", "nodeId")]
        .into_iter()
        .filter_map(|(key, field)| Some((key.to_owned(), json!(text_field(&step, field)?))))
        .collect();
    let step_ids = Value::Object(step_ids);
    match send {
        Ok(result) => {
            let mut action = json!({
                "flowId": flow_id,
                "messageId": message.message.id.to_string(),
            });
            if let Some(version) = &flow_version_id {
                action["flowVersionId"] = json!(version);
            }
            if let Some(source) = result.message_ids.first() {
                action["sourceId"] = json!(source);
            }
            crate::events::emit_message_event(
                deps.events.as_ref(),
                "message:sent",
                &ctx,
                action,
                metadata.as_ref(),
                step_ids,
            )
            .await;
            // A public reply went through `sendMessageToChannel`, which counts it.
            if !is_public_reply {
                let trigger = if text_field(&resolved, "stepType").as_deref()
                    == Some("sendMessengerTemplateMessage")
                {
                    (
                        "processMessengerTemplate",
                        "message_bot_sent_messenger_template",
                    )
                } else {
                    ("sendFlowStep", "message_bot_sent_flow")
                };
                crate::events::emit_message_sent(
                    deps.events.as_ref(),
                    conversation.workspace_id,
                    &contact_inbox,
                    &result,
                    trigger,
                    None,
                )
                .await;
            }
            // A `flow` comment reply reports delivery as a `text` one does
            // (only the first step settles: `deliveredAt IS NULL`).
            crate::low::settle_anchor_delivered(
                deps,
                message.message.content_attributes.as_ref().map(|a| &a.0),
            )
            .await;
            if !is_public_reply && result.sent_count > 0 {
                mark_read_after_delivery(
                    deps,
                    &conversation,
                    contact_inbox.inbox_id,
                    message.message.created_at,
                    is_bulk,
                )
                .await;
            }
        }
        Err(error) => {
            tracing::error!(err = %error, "sendFlowStep error for conversationId: {conversation_id}");
            // Always terminal here: nothing re-attempts the step's send.
            crate::low::settle_anchor_failed(
                deps,
                message.message.content_attributes.as_ref().map(|a| &a.0),
                error.message(),
            )
            .await;
            if !is_public_reply {
                // The public path's `sendMessageToChannel` emitted it already.
                let mut extra = step_ids;
                extra["errorData"] = crate::events::parsed_error(&error);
                extra["willRetry"] = json!(false);
                crate::events::emit_message_event(
                    deps.events.as_ref(),
                    "message:failed",
                    &ctx,
                    json!({ "messageId": message.message.id.to_string(), "flowId": flow_id }),
                    metadata.as_ref(),
                    extra,
                )
                .await;
                persist_send_error(
                    deps,
                    &conversation,
                    contact_inbox.inbox_id,
                    (message.message.id, message.message.created_at, None),
                    Some(error.message().chars().take(500).collect()),
                    is_bulk,
                )
                .await;
            }
        }
    }
    Ok(())
}

/// `sendFlowStepToChannel`: the channel's `sendFlowStep`, then the
/// provider id as the row's `sourceId`.
pub(crate) async fn send_flow_step_to_channel(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    data: SendFlowStep,
    message: &MessageWithAttachments,
) -> Result<crate::channel::SendResult, SendFailure> {
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            contact_inbox,
        )
        .await?;
    let result = channel.send_flow_step(&ctx, data).await?;
    if let Some(source) = result.message_ids.first().cloned() {
        let (id, workspace_id, created_at) = (
            message.message.id,
            conversation.workspace_id,
            message.message.created_at,
        );
        if let Err(err) = deps
            .db
            .write(move |tx| {
                repo_message::update_source_id(tx, id, &source, workspace_id, created_at)
                    .map_err(crate::repo_err)
            })
            .await
        {
            tracing::error!(err = %err, "Failed to update message sourceId with provider id");
        }
    }
    Ok(result)
}

/// `ChatJobAction.sendChatMessage` (`sendChatMessage`): a bot text (or
/// media URL) message outside a flow step.
pub async fn send_chat_message(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let conversation_id = data
        .get("conversation")
        .or_else(|| data.get("conversationId"))
        .and_then(id_of)
        .ok_or_else(|| JobError::unrecoverable("sendChatMessage: no conversation"))?;
    let Some(conversation) = load_conversation(&deps.db, conversation_id).await? else {
        return Err(fail("Conversation not found"));
    };
    let contact_inbox_id = data.get("contactInbox").and_then(id_of);
    let Some(contact_inbox) =
        find_target_contact_inbox(deps, &conversation, contact_inbox_id).await?
    else {
        return Err(JobError::unrecoverable(format!(
            "sendChatMessage: contact inbox not found for conversation {conversation_id}"
        )));
    };
    let text = str_of(data, "text").filter(|t| !t.is_empty());
    let url = str_of(data, "url").filter(|u| !u.is_empty());
    if text.is_none() && url.is_none() {
        return Ok(());
    }
    let metadata = data.get("metadata").filter(|m| !m.is_null()).cloned();
    let is_bulk = is_bulk_outbound(metadata.as_ref(), data["isBulkBroadcast"] == json!(true));
    // ponytail: a media URL is sent as text after the message text (the
    // original's download-failure fallback) until the media copy is ported.
    let message_text = [text, url]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let quick_replies = data
        .get("quickReplies")
        .and_then(Value::as_array)
        .filter(|q| !q.is_empty())
        .cloned();
    let mut attributes = Map::new();
    if let Some(metadata) = &metadata {
        attributes.insert("metadata".into(), metadata.clone());
    }
    if let Some(buttons) = &quick_replies {
        attributes.insert("type".into(), json!("template"));
        attributes.insert(
            "payload".into(),
            json!({ "templateType": "button", "buttons": buttons }),
        );
    }
    let created_at = now_ms();
    let message = store_message(
        deps,
        CreateMessageInput {
            contact_inbox_id: contact_inbox.id,
            conversation_id: conversation.id,
            workspace_id: conversation.workspace_id,
            content_attributes: Some(Value::Object(attributes)),
            content_type: "text".into(),
            message_type: "outgoing".into(),
            sender_type: "bot".into(),
            text: Some(message_text),
            created_at: Some(created_at),
            ..CreateMessageInput::default()
        },
    )
    .await?;
    let message_value = message_json(&message);
    record_outbound(
        deps,
        &conversation,
        &contact_inbox,
        created_at,
        !is_bulk,
        !metadata
            .as_ref()
            .is_some_and(is_proactive_outbound_metadata),
        None,
    )
    .await;
    let send_data = json!({
        "conversation": { "id": conversation.id.to_string() },
        "contactInbox": { "id": contact_inbox.id.to_string() },
        "message": message_value,
        "quickReplies": quick_replies,
        "metadata": metadata,
        "isBulkBroadcast": data.get("isBulkBroadcast"),
    });
    if !is_bulk {
        queue_message_created(deps, &conversation, contact_inbox.inbox_id, &message_value);
    }
    if let Err(err) = send_message_to_channel(deps, &send_data, 0, false).await {
        tracing::error!(err = %err, "sendChatMessage error for conversationId: {conversation_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! `apps/worker/__tests__/stored-step-content.test.ts`.
    use super::*;

    #[test]
    fn stores_a_location_as_its_label_plus_the_coordinates() {
        let stored = resolve_stored_step_content(&json!({
            "stepType": "whatsappLocation", "latitude": 52.52, "longitude": 13.405,
            "name": "Office", "address": "Main St 1",
        }));
        assert_eq!(stored.content_type, "location");
        assert_eq!(stored.text.as_deref(), Some("Office: Main St 1"));
        assert_eq!(
            stored.content_attributes,
            Some(
                json!({ "latitude": "52.52", "longitude": "13.405", "name": "Office", "address": "Main St 1" })
            )
        );
    }

    #[test]
    fn falls_back_to_the_coordinates_when_the_location_has_no_name_or_address() {
        let stored = resolve_stored_step_content(&json!({
            "stepType": "whatsappLocation", "latitude": 1.5, "longitude": -2,
            "name": "", "address": "",
        }));
        assert_eq!(stored.text.as_deref(), Some("1.5, -2"));
        assert_eq!(
            stored.content_attributes,
            Some(json!({ "latitude": "1.5", "longitude": "-2" }))
        );
    }

    #[test]
    fn stores_text_steps_as_text() {
        let stored = resolve_stored_step_content(&json!({ "stepType": "sendText", "text": "hi" }));
        assert_eq!(stored.content_type, "text");
        assert_eq!(stored.text.as_deref(), Some("hi"));
        assert_eq!(stored.content_attributes, None);
    }

    #[test]
    fn marks_a_view_catalog_message() {
        let stored = resolve_stored_step_content(&json!({
            "stepType": "sendViewCatalog", "text": "Browse", "footer": "Tap below",
        }));
        assert_eq!(stored.text.as_deref(), Some("Browse"));
        assert_eq!(
            stored.content_attributes,
            Some(json!({ "viewCatalog": { "footer": "Tap below" } }))
        );
    }

    #[test]
    fn stores_media_steps_without_text() {
        let stored = resolve_stored_step_content(
            &json!({ "stepType": "sendImage", "url": "https://x/y.png" }),
        );
        assert_eq!(stored.text, None);
    }

    #[test]
    fn blank_text_carriers_are_skipped() {
        assert!(is_blank_text_carrier_step(
            &json!({ "stepType": "sendText", "text": "  " })
        ));
        assert!(is_blank_text_carrier_step(
            &json!({ "stepType": "sendQuickReply", "message": "" })
        ));
        assert!(!is_blank_text_carrier_step(
            &json!({ "stepType": "sendImage" })
        ));
    }
}
