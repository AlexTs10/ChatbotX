//! `chat/handlers/send-whatsapp-template.ts` (`processWhatsappTemplate`,
//! `sendWhatsappTemplateToConversation`), `integration/handlers/wa-template-handler.ts`
//! (`validateWhatsappTemplate`, `replaceWhatsappTemplateVariables`) and
//! `wa-payment-request-guard.ts` (`assertPaymentRequestsSendable`).
//!
//! ponytail: `message:sent` / `message:failed` events and the sent
//! analytics wait for Q.W2 (no worker event bus yet).

use chatbotx_business::message::message_json;
use chatbotx_core::flow_config::steps::{
    CollectPaymentRequestIssuesOptions, bind_wa_template_quick_reply_buttons,
    collect_payment_request_issues, extract_template_params, find_payment_request_mismatches,
    payment_request_field_names, payment_request_schema, read_payment_request_field,
};
use chatbotx_core::utils::id::create_id;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::message::CreateMessageInput;
use chatbotx_db::repo::models::{ContactInbox, Conversation, WhatsappMessageTemplate};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use serde_json::{Map, Value, json};

use crate::channel::{
    Category, ChannelError, OutgoingContact, SendFailure, SendFlowStep,
    should_suppress_retryable_channel_error,
};
use crate::send_flow_step::{
    convert_buttons_to_template, queue_message_created, record_outbound, send_flow_step_to_channel,
    store_message,
};
use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

/// `ChatJobAction.sendWhatsappTemplateToConversation`.
pub const SEND_TEMPLATE_TO_CONVERSATION: &str = "sendWhatsappTemplateToConversation";

/// Template categories Meta refuses for a BSUID recipient (error 131062).
const BSUID_RESTRICTED_TEMPLATE_CATEGORIES: &[&str] = &["AUTHENTICATION"];

/// `validateWhatsappTemplate(templateId, inboxId)`: the approved template of
/// the inbox's WhatsApp number.
pub async fn validate_template(
    deps: &Deps,
    template_id: i64,
    inbox_id: i64,
) -> Result<Option<WhatsappMessageTemplate>, JobError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        r#"SELECT {} FROM "WhatsappMessageTemplate" t
                           JOIN "IntegrationWhatsapp" w ON w."id" = t."integrationWhatsappId"
                           JOIN "Inbox" i ON i."id" = w."inboxId"
                           WHERE t."id" = ?1 AND w."inboxId" = ?2 AND w."workspaceId" = i."workspaceId"
                             AND t."status" = 'APPROVED'"#,
                        WhatsappMessageTemplate::columns_sql(Some("t"))
                    ),
                    params![template_id, inbox_id],
                    WhatsappMessageTemplate::from_row,
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// The text resolver of `replaceWhatsappTemplateVariables`
/// (`contactVariableService.replaceAll`, no spintax).
async fn replace_all(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    sent_by_user_id: Option<String>,
    texts: Vec<String>,
) -> Result<Vec<String>, SendFailure> {
    let Some(variables) = &deps.variables else {
        return Ok(texts);
    };
    let resolved = variables
        .resolve(
            conversation,
            contact_inbox,
            &json!(texts),
            None,
            sent_by_user_id,
            false,
        )
        .await
        .map_err(SendFailure::Other)?;
    Ok(resolved
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or(texts))
}

/// `replaceWhatsappTemplateVariables`: `{{variables}}` in the header text,
/// body and button texts and the payment request values; NAMED parameter
/// names re-derived from the template components.
pub async fn replace_template_variables(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    sent_by_user_id: Option<String>,
    params: &Value,
    components: &[Value],
) -> Result<Value, SendFailure> {
    let named = extract_template_params(components);
    let mut out = params.clone();
    // Collect every text to resolve in one pass, then write them back.
    let mut texts: Vec<String> = Vec::new();
    let header = params["header"].as_array().cloned().unwrap_or_default();
    let body = params["body"].as_array().cloned().unwrap_or_default();
    let buttons = params["button"].as_array().cloned().unwrap_or_default();
    for h in &header {
        if h["type"] == "text"
            && let Some(t) = h["text"].as_str().filter(|t| !t.is_empty())
        {
            texts.push(t.to_owned());
        }
    }
    for b in &body {
        texts.push(b["text"].as_str().unwrap_or_default().to_owned());
    }
    for b in &buttons {
        texts.push(b["text"].as_str().unwrap_or_default().to_owned());
        if let Some(request) = b.get("payment_request").filter(|p| p.is_object()) {
            let method = request["method"].as_str().unwrap_or_default();
            for field in payment_request_field_names(method) {
                if let Some(t) =
                    read_payment_request_field(request, &field).filter(|t| !t.is_empty())
                {
                    texts.push(t.to_owned());
                }
            }
        }
    }
    let mut resolved = replace_all(deps, conversation, contact_inbox, sent_by_user_id, texts)
        .await?
        .into_iter();
    let mut next = || resolved.next().unwrap_or_default();
    if params.get("header").is_some_and(Value::is_array) {
        let replaced: Vec<Value> = header
            .iter()
            .enumerate()
            .map(|(index, h)| {
                if !(h["type"] == "text" && h["text"].as_str().is_some_and(|t| !t.is_empty())) {
                    return h.clone();
                }
                let mut h = h.clone();
                if let Some(name) = named["header"][index]["parameter_name"].as_str() {
                    h["parameter_name"] = json!(name);
                }
                h["text"] = json!(next());
                h
            })
            .collect();
        out["header"] = Value::Array(replaced);
    }
    if params.get("body").is_some_and(Value::is_array) {
        let replaced: Vec<Value> = body
            .iter()
            .enumerate()
            .map(|(index, b)| {
                let mut b = b.clone();
                if let Some(name) = named["body"][index]["parameter_name"].as_str() {
                    b["parameter_name"] = json!(name);
                }
                b["text"] = json!(next());
                b
            })
            .collect();
        out["body"] = Value::Array(replaced);
    }
    if params.get("button").is_some_and(Value::is_array) {
        let mut replaced = Vec::with_capacity(buttons.len());
        for b in &buttons {
            let mut b = b.clone();
            b["text"] = json!(next());
            if let Some(request) = b.get("payment_request").filter(|p| p.is_object()).cloned() {
                let method = request["method"].as_str().unwrap_or_default().to_owned();
                let mut request = request;
                for field in payment_request_field_names(&method) {
                    if read_payment_request_field(&request, &field).is_some_and(|t| !t.is_empty()) {
                        // Trimmed once here: the validated value is the sent one.
                        request[&field] = json!(next().trim());
                    }
                }
                let parsed = payment_request_schema()
                    .parse(&request)
                    .map_err(|e| SendFailure::Other(e.to_string()))?;
                b["payment_request"] = parsed;
            }
            replaced.push(b);
        }
        out["button"] = Value::Array(replaced);
    }
    Ok(out)
}

fn payload_invalid(message: String) -> SendFailure {
    SendFailure::Channel(ChannelError::new(message, Category::PayloadInvalid))
}

/// `assertPaymentRequestsSendable`.
pub fn assert_payment_requests_sendable(
    params: &Value,
    components: &[Value],
) -> Result<(), SendFailure> {
    let mismatches = find_payment_request_mismatches(params, components);
    if !mismatches.is_empty() {
        let list: Vec<String> = mismatches
            .iter()
            .map(|m| {
                format!(
                    "button {}: {}",
                    m["index"],
                    m["reason"].as_str().unwrap_or("")
                )
            })
            .collect();
        return Err(payload_invalid(format!(
            "WhatsApp payment request does not match the template ({})",
            list.join(", ")
        )));
    }
    let problems: Vec<String> = params["button"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["payment_request"].is_object())
        .flat_map(|b| {
            collect_payment_request_issues(
                &b["payment_request"],
                CollectPaymentRequestIssuesOptions {
                    allow_variables: false,
                },
            )
            .into_iter()
            .map(move |i| format!("button {}: {} {}", b["index"], i.field, i.kind))
        })
        .collect();
    if !problems.is_empty() {
        return Err(payload_invalid(format!(
            "WhatsApp payment request cannot be sent ({})",
            problems.join(", ")
        )));
    }
    Ok(())
}

/// The flow a template step runs in (`ProcessWhatsappTemplateParams.flow`).
#[derive(Debug, Clone, Default)]
pub struct TemplateFlow {
    pub id: String,
    pub version_id: Option<String>,
    pub buttons: Vec<Value>,
}

/// `ProcessWhatsappTemplateParams` (the ported fields).
#[derive(Debug, Clone, Default)]
pub struct ProcessTemplate {
    pub broadcast_id: Option<String>,
    pub flow: Option<TemplateFlow>,
    /// `{ id, nodeId }` of the step.
    pub step: Option<(String, String)>,
    pub metadata: Option<Value>,
    pub is_bulk_broadcast: bool,
    pub sent_by_user_id: Option<String>,
    /// `{ id, name, language, params }`.
    pub template: Value,
}

/// `withQuickReplyButtonParams`: the step's quick-reply buttons as the
/// template's quick_reply payloads (generated postbacks win).
fn with_quick_reply_button_params(
    params: Value,
    components: &[Value],
    step_buttons: &[Value],
    flow_buttons: &[Value],
) -> Value {
    let postback = |id: &Value| {
        flow_buttons
            .iter()
            .find(|b| &b["id"] == id)
            .and_then(|b| b["postback"].as_str())
            .map(str::to_owned)
    };
    let quick: Vec<Value> = bind_wa_template_quick_reply_buttons(components, step_buttons)
        .iter()
        .filter_map(|binding| {
            let payload = postback(&binding["stepButton"]["id"])?;
            Some(json!({ "sub_type": "quick_reply", "index": binding["templateButtonIndex"], "payload": payload }))
        })
        .collect();
    if quick.is_empty() {
        return params;
    }
    let bound: Vec<&Value> = quick.iter().map(|q| &q["index"]).collect();
    let mut buttons: Vec<Value> = params["button"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            !(p["sub_type"] == "quick_reply"
                && p["index"].is_number()
                && bound.contains(&&p["index"]))
        })
        .cloned()
        .collect();
    buttons.extend(quick);
    let mut out = params;
    out["button"] = Value::Array(buttons);
    out
}

/// `processWhatsappTemplate`: validate, resolve, store the outgoing row,
/// send through the channel's `sendFlowStep`. The provider id becomes the
/// row's `sourceId`.
pub async fn process_template(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    input: ProcessTemplate,
) -> Result<Value, SendFailure> {
    let is_bulk = crate::chat::is_bulk_outbound(
        input.metadata.as_ref(),
        input.is_bulk_broadcast || input.broadcast_id.is_some(),
    );
    let template_id = str_of(&input.template, "id").unwrap_or_default().to_owned();
    let validated = validate_template(
        deps,
        template_id.parse().unwrap_or(-1),
        contact_inbox.inbox_id,
    )
    .await
    .map_err(|e| SendFailure::Other(e.to_string()))?
    .ok_or_else(|| SendFailure::Other(format!("Template validation failed: {template_id}")))?;
    if chatbotx_business::whatsapp::should_address_by_source_user_id(
        &contact_inbox.source_id,
        contact_inbox.source_user_id.as_deref(),
    ) && BSUID_RESTRICTED_TEMPLATE_CATEGORIES.contains(&validated.category.as_str())
    {
        return Err(SendFailure::Channel(
            ChannelError::new(
                format!(
                    "Cannot send a {} template to a Business-Scoped User ID recipient",
                    validated.category
                ),
                Category::PayloadInvalid,
            )
            .with_code("131062"),
        ));
    }
    let components = validated
        .components
        .0
        .as_array()
        .cloned()
        .unwrap_or_default();
    let replaced = replace_template_variables(
        deps,
        conversation,
        contact_inbox,
        input.sent_by_user_id.clone(),
        &input.template["params"],
        &components,
    )
    .await?;
    assert_payment_requests_sendable(&replaced, &components)?;
    let flow_buttons = match &input.flow {
        Some(flow) if !flow.buttons.is_empty() => convert_buttons_to_template(
            &contact_inbox.channel,
            &flow.id,
            flow.version_id.as_deref(),
            &flow.buttons,
            input.metadata.as_ref(),
            Some(&contact_inbox.id.to_string()),
        ),
        _ => Vec::new(),
    };
    let step_buttons = input
        .flow
        .as_ref()
        .map(|f| f.buttons.clone())
        .unwrap_or_default();
    let resolved =
        with_quick_reply_button_params(replaced, &components, &step_buttons, &flow_buttons);

    let mut attributes = Map::new();
    attributes.insert("type".into(), json!("whatsapp_template"));
    attributes.insert(
        "template".into(),
        json!({ "name": input.template["name"], "language": input.template["language"],
            "id": input.template["id"], "params": resolved }),
    );
    if let Some((step_id, node_id)) = &input.step {
        attributes.insert("stepId".into(), json!(step_id));
        attributes.insert("nodeId".into(), json!(node_id));
    }
    if let Some(broadcast) = &input.broadcast_id {
        attributes.insert("broadcastId".into(), json!(broadcast));
    }
    if let Some(flow) = &input.flow {
        attributes.insert("flowId".into(), json!(flow.id));
        attributes.insert("flowVersionId".into(), json!(flow.version_id));
    }
    attributes.insert(
        "payload".into(),
        if flow_buttons.is_empty() {
            json!({})
        } else {
            json!({ "templateType": "button", "buttons": flow_buttons })
        },
    );
    if let Some(metadata) = &input.metadata {
        attributes.insert("metadata".into(), metadata.clone());
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
            text: Some(format!(
                "Template: {}",
                input.template["name"].as_str().unwrap_or_default()
            )),
            created_at: Some(created_at),
            ..CreateMessageInput::default()
        },
    )
    .await
    .map_err(|e| SendFailure::Other(e.to_string()))?;
    // Template sends are proactive: never a reply that closes the window.
    record_outbound(
        deps,
        conversation,
        contact_inbox,
        created_at,
        !is_bulk,
        false,
        None,
    )
    .await;
    let message_value = message_json(&message);
    if !is_bulk {
        queue_message_created(deps, conversation, contact_inbox.inbox_id, &message_value);
    }
    let (step_id, node_id) = input
        .step
        .clone()
        .unwrap_or_else(|| (create_id(), create_id()));
    let mut template = input.template.clone();
    template["params"] = resolved;
    let flow = input.flow.clone().unwrap_or_default();
    let result = send_flow_step_to_channel(
        deps,
        conversation,
        contact_inbox,
        SendFlowStep {
            contact: OutgoingContact::new(contact_inbox, conversation),
            flow_id: flow.id,
            flow_version_id: flow.version_id,
            step: json!({ "id": step_id, "nodeId": node_id, "stepType": "sendWaTemplateMessage",
                "buttons": [], "template": template }),
            product_message: None,
            quick_replies: None,
            metadata: input.metadata.clone(),
            rich_response: None,
            send_from: None,
            comment_anchor: None,
            message: message_value.clone(),
        },
        &message,
    )
    .await?;
    if result.sent_count > 0 {
        crate::chat::mark_read_after_delivery(
            deps,
            conversation,
            contact_inbox.inbox_id,
            created_at,
            is_bulk,
        )
        .await;
    }
    let mut out = message_value;
    out["sourceId"] = json!(result.message_ids.first());
    Ok(out)
}

/// `sendWhatsappTemplateToConversation`: one approved template into an
/// open conversation with the caller's params.
pub async fn send_template_to_conversation(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let conversation_id = data.get("conversation").and_then(id_of);
    let Some(conversation) = (match conversation_id {
        Some(id) => load_conversation(&deps.db, id).await?,
        None => None,
    }) else {
        return Err(JobError::unrecoverable(
            "sendWhatsappTemplateToConversation: conversation not found",
        ));
    };
    let contact_inbox_id = data.get("contactInbox").and_then(id_of).unwrap_or(-1);
    let Some(contact_inbox) =
        load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    else {
        return Err(JobError::unrecoverable(
            "sendWhatsappTemplateToConversation: contact inbox not found",
        ));
    };
    let template_id = str_of(data, "templateId").unwrap_or_default().to_owned();
    let Some(template) = validate_template(
        deps,
        template_id.parse().unwrap_or(-1),
        contact_inbox.inbox_id,
    )
    .await?
    else {
        tracing::error!(%template_id, conversation_id = conversation.id, "Error sending WhatsApp template message to conversation");
        return Err(fail(format!(
            "WhatsApp template not found or not approved: {template_id}"
        )));
    };
    let params = match data.get("templateData").filter(|d| !d.is_null()) {
        Some(params) => params.clone(),
        None => extract_template_params(
            &template
                .components
                .0
                .as_array()
                .cloned()
                .unwrap_or_default(),
        ),
    };
    let input = ProcessTemplate {
        metadata: data.get("metadata").filter(|m| !m.is_null()).cloned(),
        template: json!({ "id": template.id.to_string(), "name": template.name,
            "language": template.language, "params": params }),
        ..ProcessTemplate::default()
    };
    match process_template(deps, &conversation, &contact_inbox, input).await {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!(err = %error.message(), conversation_id = conversation.id, %template_id,
                "Error sending WhatsApp template message to conversation");
            if should_suppress_retryable_channel_error(&error, &contact_inbox.channel) {
                Ok(())
            } else {
                Err(fail(error.message()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_quick_reply_postbacks_replace_legacy_ones() {
        let components = vec![json!({ "type": "BUTTONS", "buttons": [
            { "type": "QUICK_REPLY", "text": "Yes" }, { "type": "QUICK_REPLY", "text": "No" } ] })];
        // The first two step buttons are the status branches.
        let step_buttons = vec![
            json!({ "id": "delivered" }),
            json!({ "id": "failed" }),
            json!({ "id": "b1", "label": "Yes" }),
            json!({ "id": "b2", "label": "No" }),
        ];
        let flow_buttons = vec![
            json!({ "id": "b1", "postback": "p1" }),
            json!({ "id": "b2", "postback": "p2" }),
        ];
        let params =
            json!({ "button": [{ "sub_type": "quick_reply", "index": 0, "payload": "legacy" }] });
        let out = with_quick_reply_button_params(
            params.clone(),
            &components,
            &step_buttons,
            &flow_buttons,
        );
        assert_eq!(
            out["button"],
            json!([{ "sub_type": "quick_reply", "index": 0, "payload": "p1" },
                { "sub_type": "quick_reply", "index": 1, "payload": "p2" }])
        );
        assert_eq!(
            with_quick_reply_button_params(params.clone(), &components, &[], &[]),
            params
        );
    }

    #[test]
    fn payment_request_mismatch_is_payload_invalid() {
        let components = vec![json!({ "type": "BUTTONS", "buttons": [] })];
        assert!(assert_payment_requests_sendable(&json!({}), &components).is_ok());
    }
}
