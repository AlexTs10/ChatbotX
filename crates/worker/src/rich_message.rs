//! The AI rich response mode (`rich-response/index.ts` `parseRichResponse`,
//! `rich-response/message-sender.ts`, `automated-response/rich-reply.ts`):
//! the model answers one JSON document of Messenger-style messages and rich
//! actions; each message becomes a flow step sent with its button payloads.
//!
//! ponytail: the last-resort `jsonrepair` pass is not ported: a document
//! that is neither valid JSON nor a valid JSON prefix is `invalid_json`.

use std::collections::BTreeMap;

use chatbotx_core::flow_config as fc;
use chatbotx_core::utils::id::create_id;
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Map, Value, json};

use crate::Deps;
use crate::rich_response::{RichAction, parse_rich_actions};

/// `helpTexts.richResponseFormat`.
pub const RICH_RESPONSE_FORMAT: &str = r#"Output:
{"messages":[...],"actions":[...]}

messages[]:
- integer 0-30 (delay seconds)
- {"message":{"text":"..."}}
- {"message":{"attachment":{"type":"image","payload":{"url":"..."}}}}
- {"message":{"attachment":{"type":"template","payload":{"template_type":"button","text":"...","buttons":[{"type":"web_url","title":"...","url":"..."}]}}}}
- {"message":{"attachment":{"type":"template","payload":{"template_type":"generic","elements":[{"title":"...","image_url":"...","buttons":[...]}]}}}}
- {"message":{"text":"...","quick_replies":[{"content_type":"text","title":"...","payload":"..."}]}}
- text, attachment, quick_replies MUST be inside "message" — never beside it.

actions[]:
- {"action":"add_tag","tag_name":"..."}
- {"action":"remove_tag","tag_name":"..."}
- {"action":"set_field_value","field_name":"...","value":"..."}
- {"action":"unset_field_value","field_name":"..."}
- {"action":"send_flow","flow_id":"..."}
- {"action":"transfer_conversation_to","value":"human"}
- {"action":"assign_conversation","admin_id":"..."}
- {"action":"unassign_conversation"}

Rules:
- Root only has messages and actions.
- Either array may be empty, but at least one must have an item.
- Option question => quick_replies inside message + actions:[]
- Add actions only after explicit confirmation or selection.
- quick_replies <= 10, buttons <= 3, carousel elements <= 10. Titles <= 20 chars.
- Postback payload = flow_id string OR JSON string {"actions":[...]}
- No WhatsApp interactive/list/catalog/location payloads.
- Output valid JSON only. No markdown fences, no prose outside JSON.
- Balanced {} and [] — count every open bracket and close it."#;

/// `MAX_BUTTON_LABEL_LENGTH`.
const MAX_LABEL: usize = 20;
/// `MAX_GENERIC_ELEMENT_TEXT_LENGTH`.
const MAX_ELEMENT_TEXT: usize = 80;
/// `MESSAGE_ORDER_DELAY_MS`.
const MESSAGE_ORDER_DELAY_MS: u64 = 250;

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `.slice(0, n)` (UTF-16 units).
fn slice16(text: &str, n: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

fn is_url(value: &str) -> bool {
    url::Url::parse(value).is_ok()
}

/// `z.string().trim().min(min).max(max)` of `value[key]`; `Err` = invalid,
/// `Ok(None)` = absent.
fn text_field(value: &Value, key: &str, min: usize, max: usize) -> Result<Option<String>, ()> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let text = text.trim();
            let len = utf16_len(text);
            if len < min || len > max {
                Err(())
            } else {
                Ok(Some(text.to_owned()))
            }
        }
        Some(_) => Err(()),
    }
}

/// A button / quick reply title: trimmed, at least 1, cut to 20.
fn title(value: &Value) -> Option<String> {
    let title = value.get("title")?.as_str()?.trim();
    (!title.is_empty()).then(|| slice16(title, MAX_LABEL))
}

fn opt_string(value: &Value, key: &str) -> Result<Option<String>, ()> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(()),
    }
}

/// `buttonSchema`.
#[derive(Debug, Clone, PartialEq)]
pub struct RichButton {
    pub title: String,
    pub kind: String,
    pub url: Option<String>,
    pub payload: Option<String>,
}

fn parse_button(value: &Value) -> Option<RichButton> {
    let kind = value.get("type")?.as_str()?;
    if !matches!(kind, "web_url" | "postback" | "phone_number") {
        return None;
    }
    let url = opt_string(value, "url").ok()?;
    if url.as_deref().is_some_and(|u| !is_url(u)) {
        return None;
    }
    Some(RichButton {
        title: title(value)?,
        kind: kind.to_owned(),
        url,
        payload: opt_string(value, "payload").ok()?,
    })
}

/// `z.array(buttonSchema).max(3).default([])`.
fn parse_buttons(value: Option<&Value>) -> Option<Vec<RichButton>> {
    match value {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) if items.len() <= 3 => items.iter().map(parse_button).collect(),
        Some(_) => None,
    }
}

/// One element of a generic template.
#[derive(Debug, Clone, PartialEq)]
pub struct RichElement {
    pub title: String,
    pub subtitle: Option<String>,
    pub image_url: Option<String>,
    pub buttons: Vec<RichButton>,
}

/// `mediaAttachmentSchema | buttonTemplateSchema | genericTemplateSchema`.
#[derive(Debug, Clone, PartialEq)]
pub enum RichAttachment {
    Media {
        kind: String,
        url: String,
    },
    Buttons {
        text: String,
        buttons: Vec<RichButton>,
    },
    Generic {
        elements: Vec<RichElement>,
    },
}

fn parse_attachment(value: &Value) -> Option<RichAttachment> {
    let kind = value.get("type")?.as_str()?;
    let payload = value.get("payload")?;
    if matches!(kind, "image" | "video" | "audio" | "file") {
        let url = payload.get("url")?.as_str()?;
        return is_url(url).then(|| RichAttachment::Media {
            kind: kind.to_owned(),
            url: url.to_owned(),
        });
    }
    if !matches!(kind, "template" | "button") {
        return None;
    }
    match payload.get("template_type")?.as_str()? {
        "button" => Some(RichAttachment::Buttons {
            text: text_field(payload, "text", 1, 1000).ok()??,
            buttons: parse_buttons(payload.get("buttons"))?,
        }),
        "generic" if kind == "template" => {
            if let Some(ratio) = payload.get("image_aspect_ratio").filter(|v| !v.is_null()) {
                if !matches!(ratio.as_str(), Some("horizontal" | "square")) {
                    return None;
                }
            }
            let elements = payload.get("elements")?.as_array()?;
            if elements.is_empty() || elements.len() > 10 {
                return None;
            }
            let elements = elements
                .iter()
                .map(|e| {
                    let image_url = opt_string(e, "image_url").ok()?;
                    if image_url.as_deref().is_some_and(|u| !is_url(u)) {
                        return None;
                    }
                    Some(RichElement {
                        title: text_field(e, "title", 1, MAX_ELEMENT_TEXT).ok()??,
                        subtitle: text_field(e, "subtitle", 0, MAX_ELEMENT_TEXT).ok()?,
                        image_url,
                        buttons: parse_buttons(e.get("buttons"))?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(RichAttachment::Generic { elements })
        }
        _ => None,
    }
}

/// `richMessengerMessageSchema` (parsed).
#[derive(Debug, Clone, PartialEq)]
pub struct RichMessage {
    pub text: Option<String>,
    /// `(title, payload)`; `None` = no `quick_replies` key.
    pub quick_replies: Option<Vec<(String, Option<String>)>>,
    pub attachment: Option<RichAttachment>,
}

fn parse_quick_replies(value: &Value) -> Option<Vec<(String, Option<String>)>> {
    let items = value.as_array()?;
    if items.len() > 10 {
        return None;
    }
    items
        .iter()
        .map(|q| {
            (q.get("content_type")?.as_str()? == "text")
                .then_some(())
                .and_then(|()| Some((title(q)?, opt_string(q, "payload").ok()?)))
        })
        .collect()
}

pub fn parse_messenger_message(value: &Value) -> Option<RichMessage> {
    if !value.is_object() {
        return None;
    }
    let text = text_field(value, "text", 1, 1000).ok()?;
    let quick_replies = match value.get("quick_replies") {
        None | Some(Value::Null) => None,
        Some(q) => Some(parse_quick_replies(q)?),
    };
    let attachment = match value.get("attachment") {
        None | Some(Value::Null) => None,
        Some(a) => Some(parse_attachment(a)?),
    };
    if text.is_none() && quick_replies.is_none() && attachment.is_none() {
        return None;
    }
    Some(RichMessage {
        text,
        quick_replies,
        attachment,
    })
}

/// `richMessageSchema` (parsed).
#[derive(Debug, Clone, PartialEq)]
pub enum RichItem {
    Delay(u64),
    Message(RichMessage),
    WhatsappNative,
}

fn parse_item(value: &Value) -> Option<RichItem> {
    if value.is_number() {
        let n = value.as_f64()?;
        return (n.fract() == 0.0 && (0.0..=30.0).contains(&n))
            .then_some(RichItem::Delay(n as u64));
    }
    let object = value.as_object()?;
    if let (Some(Value::Object(message)), Some(quick)) =
        (object.get("message"), object.get("quick_replies"))
        && parse_quick_replies(quick).is_some()
    {
        let mut merged = message.clone();
        merged.insert("quick_replies".into(), quick.clone());
        return parse_messenger_message(&Value::Object(merged)).map(RichItem::Message);
    }
    if let Some(message) = object.get("message")
        && let Some(parsed) = parse_messenger_message(message)
    {
        return Some(RichItem::Message(parsed));
    }
    if object.get("messaging_product").and_then(Value::as_str) == Some("whatsapp") {
        return Some(RichItem::WhatsappNative);
    }
    parse_messenger_message(value).map(RichItem::Message)
}

/// `ParsedRichResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRich {
    pub messages: Vec<RichItem>,
    pub actions: Vec<RichAction>,
}

/// `RichResponseParseResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum RichParse {
    Ok(ParsedRich),
    PlainText,
    InvalidJson,
    SchemaError,
}

impl RichParse {
    pub fn reason(&self) -> &'static str {
        match self {
            RichParse::Ok(_) => "ok",
            RichParse::PlainText => "plain_text",
            RichParse::InvalidJson => "invalid_json",
            RichParse::SchemaError => "schema_error",
        }
    }
}

fn strip_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some(body) = rest.strip_suffix("```") else {
        return text;
    };
    let body = if body.len() >= 4 && body[..4].eq_ignore_ascii_case("json") {
        &body[4..]
    } else {
        body
    };
    body.trim()
}

fn looks_like_plain_text(text: &str) -> bool {
    let trimmed = text.trim();
    match trimmed.chars().next() {
        None | Some('{' | '[') => false,
        Some(_) => !trimmed.contains('{') && !trimmed.contains('['),
    }
}

/// `extractJsonPrefix`: the first balanced JSON value.
fn extract_json_prefix(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with(['{', '[']) {
        return None;
    }
    let (mut stack, mut in_string, mut escaped) = (Vec::new(), false, false);
    for (index, ch) in trimmed.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop() != Some(ch) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(&trimmed[..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `parseRichResponse`.
pub fn parse_rich_response(text: &str) -> RichParse {
    let stripped = strip_fence(text.trim());
    if stripped.is_empty() {
        return RichParse::InvalidJson;
    }
    let parsed: Value = match serde_json::from_str(stripped) {
        Ok(value) => value,
        Err(_) => match extract_json_prefix(stripped) {
            Some(prefix) => match serde_json::from_str(prefix) {
                Ok(value) => value,
                Err(_) => return RichParse::InvalidJson,
            },
            None if looks_like_plain_text(stripped) => return RichParse::PlainText,
            None => return RichParse::InvalidJson,
        },
    };
    let Some(object) = parsed.as_object() else {
        return RichParse::SchemaError;
    };
    let list = |key: &str| match object.get(key) {
        None => Some(Vec::new()),
        Some(Value::Array(items)) => Some(items.clone()),
        Some(_) => None,
    };
    let (Some(messages), Some(actions)) = (list("messages"), list("actions")) else {
        return RichParse::SchemaError;
    };
    let Some(messages) = messages.iter().map(parse_item).collect::<Option<Vec<_>>>() else {
        return RichParse::SchemaError;
    };
    let actions = if actions.is_empty() {
        Vec::new()
    } else {
        match parse_rich_actions(&Value::Array(actions)) {
            Some(actions) => actions,
            None => return RichParse::SchemaError,
        }
    };
    if messages.is_empty() && actions.is_empty() {
        return RichParse::SchemaError;
    }
    RichParse::Ok(ParsedRich { messages, actions })
}

/// `RichAction` as its schema JSON (the stored button payload).
fn action_json(action: &RichAction) -> Value {
    match action {
        RichAction::AddTag(t) => json!({ "action": "add_tag", "tag_name": t }),
        RichAction::RemoveTag(t) => json!({ "action": "remove_tag", "tag_name": t }),
        RichAction::SetField(f, v) => {
            json!({ "action": "set_field_value", "field_name": f, "value": v })
        }
        RichAction::UnsetField(f) => json!({ "action": "unset_field_value", "field_name": f }),
        RichAction::SendFlow(id) => json!({ "action": "send_flow", "flow_id": id }),
        RichAction::Transfer => json!({ "action": "transfer_conversation_to", "value": "human" }),
        RichAction::Assign(id) => json!({ "action": "assign_conversation", "admin_id": id }),
        RichAction::Unassign => json!({ "action": "unassign_conversation" }),
    }
}

/// `parseRichButtonPayload` as the stored payload JSON; `Err(reason)` =
/// unsupported.
fn parse_button_payload_text(payload: Option<&str>) -> Result<Value, &'static str> {
    let normalized = payload.map(str::trim).unwrap_or("");
    if normalized.is_empty() {
        return Err("empty_payload");
    }
    if normalized.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(json!({ "type": "send_flow", "flowId": normalized }));
    }
    let parsed: Value = serde_json::from_str(normalized).map_err(|_| "invalid_json_payload")?;
    let actions = parsed
        .get("actions")
        .and_then(parse_rich_actions)
        .ok_or("unsupported_payload_shape")?;
    Ok(json!({ "type": "actions", "actions": actions.iter().map(action_json).collect::<Vec<_>>() }))
}

/// The step of one message and its button payloads.
pub struct ConvertedStep {
    pub step: Value,
    pub button_payloads: BTreeMap<String, Value>,
}

fn schema_parsed(schema: fc::z::Schema, step: Value) -> Option<Value> {
    let result = schema.safe_parse(&step);
    result.success.then(|| result.data.unwrap_or(step))
}

fn entry(execution_id: &str, button_id: &str, payload: Value) -> Value {
    json!({ "executionId": execution_id, "buttonId": button_id, "payload": payload })
}

/// `convertButtons`.
fn convert_buttons(
    buttons: &[RichButton],
    execution_id: &str,
    payloads: &mut BTreeMap<String, Value>,
) -> Vec<Value> {
    let mut converted = Vec::new();
    for button in buttons {
        let label = slice16(button.title.trim(), MAX_LABEL);
        if button.kind == "web_url"
            && let Some(url) = &button.url
        {
            let mut step = fc::steps::button_step_default_fn(&json!({ "label": label }));
            let mut before = fc::steps::open_website_step_default_fn();
            before["url"] = json!(url);
            step["buttonType"] = json!("openWebsite");
            step["beforeStep"] = before;
            step["steps"] = json!([]);
            converted.push(step);
            continue;
        }
        if button.kind == "postback" {
            match parse_button_payload_text(button.payload.as_deref()) {
                Ok(payload) => {
                    let step = fc::steps::button_step_default_fn(&json!({ "label": label }));
                    let id = step["id"].as_str().unwrap_or_default().to_owned();
                    payloads.insert(id.clone(), entry(execution_id, &id, payload));
                    converted.push(step);
                }
                Err(reason) => {
                    tracing::warn!(
                        execution_id,
                        reason,
                        "[rich-response] omitted unsupported button payload"
                    );
                }
            }
            continue;
        }
        tracing::warn!(
            execution_id,
            reason = "unsupported_phone_number_button",
            "[rich-response] omitted unsupported button"
        );
    }
    converted
}

fn media_step(kind: &str, url: &str) -> Option<Value> {
    let (mut step, schema) = match kind {
        "image" => (
            fc::steps::send_image_step_default_fn(),
            fc::steps::send_image_step_schema(),
        ),
        "video" => (
            fc::steps::send_video_step_default_fn(),
            fc::steps::send_video_step_schema(),
        ),
        "audio" => (
            fc::steps::send_audio_step_default_fn(
                &json!({ "mode": "url", "url": url, "buttons": [] }),
            ),
            fc::steps::send_audio_step_schema(),
        ),
        _ => (
            fc::steps::send_file_step_default_fn(),
            fc::steps::send_file_step_schema(),
        ),
    };
    step["mode"] = json!("url");
    step["url"] = json!(url);
    step["buttons"] = json!([]);
    schema_parsed(schema, step)
}

/// `convertMessengerMessage` (`None` = the step does not parse).
pub fn convert_message(message: &RichMessage, execution_id: &str) -> Option<ConvertedStep> {
    let mut payloads = BTreeMap::new();
    if let Some(quick) = message.quick_replies.as_ref().filter(|q| !q.is_empty()) {
        let buttons: Vec<Value> = quick
            .iter()
            .map(|(title, payload)| {
                let step = fc::steps::button_step_default_fn(
                    &json!({ "label": slice16(title.trim(), MAX_LABEL) }),
                );
                let id = step["id"].as_str().unwrap_or_default().to_owned();
                let stored = parse_button_payload_text(payload.as_deref()).unwrap_or_else(|_| {
                    let text = payload.as_deref().map(str::trim).filter(|p| !p.is_empty());
                    json!({ "type": "text", "text": text.unwrap_or(title) })
                });
                payloads.insert(id.clone(), entry(execution_id, &id, stored));
                step
            })
            .collect();
        let step = fc::steps::send_quick_reply_step_default_fn(&json!({
            "message": message.text.as_deref().unwrap_or("Please select an option"),
            "buttons": buttons,
        }));
        return schema_parsed(fc::steps::send_quick_reply_step_schema(), step).map(|step| {
            ConvertedStep {
                step,
                button_payloads: payloads,
            }
        });
    }
    let step = match &message.attachment {
        Some(RichAttachment::Buttons { text, buttons }) => {
            let buttons = convert_buttons(buttons, execution_id, &mut payloads);
            schema_parsed(
                fc::steps::send_text_step_schema(),
                fc::steps::send_text_step_default_fn(&json!({ "text": text, "buttons": buttons })),
            )?
        }
        Some(RichAttachment::Generic { elements }) => {
            let cards: Vec<Value> = elements
                .iter()
                .map(|element| {
                    let buttons = convert_buttons(&element.buttons, execution_id, &mut payloads);
                    let mut card = Map::new();
                    card.insert("id".into(), json!(create_id()));
                    card.insert("stepType".into(), json!("sendCard"));
                    card.insert("title".into(), json!(element.title));
                    if let Some(subtitle) = &element.subtitle {
                        card.insert("subtitle".into(), json!(subtitle));
                    }
                    if let Some(url) = &element.image_url {
                        let mut image = fc::steps::send_image_step_default_fn();
                        image["mode"] = json!("url");
                        image["url"] = json!(url);
                        image["buttons"] = json!([]);
                        card.insert("image".into(), image);
                    }
                    card.insert("buttons".into(), Value::Array(buttons));
                    Value::Object(card)
                })
                .collect();
            let mut step = fc::steps::send_carousel_step_default_fn();
            step["cards"] = Value::Array(cards);
            schema_parsed(fc::steps::send_carousel_step_schema(), step)?
        }
        Some(RichAttachment::Media { kind, url }) => media_step(kind, url)?,
        None => schema_parsed(
            fc::steps::send_text_step_schema(),
            fc::steps::send_text_step_default_fn(
                &json!({ "text": message.text.as_deref().unwrap_or(""), "buttons": [] }),
            ),
        )?,
    };
    Some(ConvertedStep {
        step,
        button_payloads: payloads,
    })
}

/// `sendRichMessages`: one delayed `sendFlowMessage` per message (a delay
/// item adds to the schedule; messages stay at least 250 ms apart). Answers
/// `(enqueued, skipped)`.
pub async fn send_rich_messages(
    deps: &Deps,
    items: &[RichItem],
    conversation_id: i64,
    flow_context_id: &str,
    execution_id: &str,
    tracking: Option<Value>,
) -> (usize, usize) {
    let (mut cumulative, mut last) = (0u64, None::<u64>);
    let (mut index, mut candidates, mut enqueued) = (0usize, 0usize, 0usize);
    for item in items {
        let message = match item {
            RichItem::Delay(seconds) => {
                cumulative += seconds * 1000;
                continue;
            }
            RichItem::WhatsappNative => {
                candidates += 1;
                tracing::warn!(
                    conversation_id,
                    execution_id,
                    message_type = "whatsapp_native",
                    reason = "unsupported_phase_1",
                    "[rich-response] skipped native WhatsApp message"
                );
                continue;
            }
            RichItem::Message(message) => message,
        };
        candidates += 1;
        let Some(converted) = convert_message(message, execution_id) else {
            tracing::warn!(
                conversation_id,
                execution_id,
                "[rich-response] failed to convert message"
            );
            continue;
        };
        let delay = last.map_or(cumulative, |last| {
            cumulative.max(last + MESSAGE_ORDER_DELAY_MS)
        });
        last = Some(delay);
        let mut data = json!({
            "conversationId": conversation_id.to_string(),
            "flowId": flow_context_id,
            "step": converted.step,
            "richResponse": { "executionId": execution_id, "buttonPayloads": converted.button_payloads },
        });
        if let Some(tracking) = &tracking {
            data["trackingContext"] = tracking.clone();
        }
        let added = deps
            .jobs
            .add(
                QueueName::Chat,
                crate::chat::SEND_FLOW_MESSAGE,
                json!({ "type": crate::chat::SEND_FLOW_MESSAGE, "data": data }),
                JobOptions {
                    job_id: Some(format!(
                        "rich-response-message-{conversation_id}-{execution_id}-{index}"
                    )),
                    delay_ms: Some(delay),
                    ..JobOptions::default()
                },
            )
            .await;
        match added {
            Ok(_) => enqueued += 1,
            Err(err) => {
                tracing::error!(err = %err, conversation_id, "[rich-response] sendFlowMessage enqueue failed")
            }
        }
        index += 1;
    }
    (enqueued, candidates - enqueued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_response::parse_rich_action;

    #[test]
    fn parses_documents_like_the_original() {
        let doc = r#"```json
{"messages":[2,{"message":{"text":"Pick"},"quick_replies":[{"content_type":"text","title":"A very long title over twenty","payload":"12"}]}],"actions":[{"action":"add_tag","tag_name":"x"}]}
```"#;
        let RichParse::Ok(parsed) = parse_rich_response(doc) else {
            panic!("expected ok");
        };
        assert_eq!(parsed.messages[0], RichItem::Delay(2));
        let RichItem::Message(message) = &parsed.messages[1] else {
            panic!("expected a message");
        };
        let quick = message.quick_replies.as_ref().unwrap();
        assert_eq!(quick[0].0, "A very long title ov");
        assert_eq!(parsed.actions, vec![RichAction::AddTag("x".into())]);

        assert_eq!(parse_rich_response("Hello there"), RichParse::PlainText);
        assert_eq!(parse_rich_response("Hi {broken"), RichParse::InvalidJson);
        assert_eq!(
            parse_rich_response(r#"{"messages":[],"actions":[]}"#),
            RichParse::SchemaError
        );
        assert_eq!(
            parse_rich_response(r#"{"messages":[31]}"#),
            RichParse::SchemaError
        );
        // A valid prefix followed by prose.
        assert!(matches!(
            parse_rich_response(r#"{"messages":[{"text":"hi"}]} trailing"#),
            RichParse::Ok(_)
        ));
    }

    #[test]
    fn button_payloads_become_store_entries() {
        assert_eq!(
            parse_button_payload_text(Some(" 42 ")),
            Ok(json!({ "type": "send_flow", "flowId": "42" }))
        );
        assert_eq!(
            parse_button_payload_text(Some(r#"{"actions":[{"action":"unassign_conversation"}]}"#)),
            Ok(json!({ "type": "actions", "actions": [{ "action": "unassign_conversation" }] }))
        );
        assert_eq!(
            parse_button_payload_text(Some("x")),
            Err("invalid_json_payload")
        );
        assert_eq!(parse_button_payload_text(None), Err("empty_payload"));
        let message = parse_messenger_message(&json!({ "text": "Pick", "quick_replies": [
            { "content_type": "text", "title": "Yes", "payload": "" },
        ]}))
        .unwrap();
        let converted = convert_message(&message, "exec").unwrap();
        let (id, stored) = converted.button_payloads.iter().next().unwrap();
        assert_eq!(stored["buttonId"], json!(id));
        assert_eq!(stored["payload"], json!({ "type": "text", "text": "Yes" }));
        assert!(parse_rich_action(&json!({ "action": "send_flow", "flow_id": " 7 " })).is_some());
    }
}
