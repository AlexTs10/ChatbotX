//! `integrations/messenger` (Q.I1b): the Facebook Page channel. The Graph
//! calls (`apis/message.ts`, `apis/comment.ts`, `apis/attachment.ts`,
//! `apis/user.ts`, the whitelisted domains of `apis/page.ts`),
//! `lib/error-mapper.ts`, `sendMessage` / `sendFlowStep`
//! (`handlers/message/outgoing-message/*`), the comment handlers,
//! `sendTyping` / `agentMarkAsRead` and `receiveMessage`
//! (`handlers/message/incoming-message.ts`, `lib/echo.ts`).

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use chatbotx_core::flow_config::util::{
    ButtonPayload, append_code_to_magic_link, encode_button_payload, extract_metadata,
};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::BoxFuture;
use serde_json::{Value, json};

use super::graph::{GraphError, GraphHttp, GraphRequest};
use super::meta::{
    META_HUMAN_AGENT_WINDOW_MS, META_RESPONSE_WINDOW_MS, NATIVE_USER_EMAIL,
    NATIVE_USER_PHONE_NUMBER, assert_private_reply_follow_up, canonical_reply_payload, compact,
    flow_button_payload, is_deferrable_type, normalize_meta_ad_referral, pending_attachment,
    str_field,
};
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingContact, IncomingMessage,
    OutgoingContact, Received, SendFailure, SendFlowStep, SendMessage, SendResult, SendTyping,
};

/// `API_URL`.
pub const API_URL: &str = "https://graph.facebook.com";
/// `DEFAULT_API_VERSION`.
pub const DEFAULT_API_VERSION: &str = "v23.0";
/// `MESSENGER_MESSAGE_METADATA`: stamped on every send so the echo is skipped.
pub const MESSAGE_METADATA: &str = "SENT_FROM_CHATBOTX";
/// `META_FIRST_PARTY_ECHO_APP_IDS`.
const FIRST_PARTY_ECHO_APP_IDS: &[&str] = &["26390203743090"];
/// `MAX_BUTTONS`.
const MAX_BUTTONS: usize = 3;
/// `MAX_CAROUSEL_ELEMENTS`.
const MAX_CAROUSEL_ELEMENTS: usize = 10;
/// `MESSENGER_EXTENSION_DOMAIN_NOT_WHITELISTED_SUBCODE`.
const DOMAIN_NOT_WHITELISTED_SUBCODE: i64 = 2_018_062;

/// `ensuredMessengerExtensionDomains` (`<pageId>:<origin>`, per process).
static ENSURED_DOMAINS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn ensured(key: &str) -> bool {
    ENSURED_DOMAINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|s| s.contains(key))
}

fn set_ensured(key: &str, on: bool) {
    let mut guard = ENSURED_DOMAINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let set = guard.get_or_insert_with(HashSet::new);
    if on {
        set.insert(key.to_owned());
    } else {
        set.remove(key);
    }
}

pub struct Messenger {
    graph: GraphHttp,
    /// `facebookAttachmentClient` (60 s, 2 retries, 2 s backoff).
    attachments: GraphHttp,
}

impl Messenger {
    pub fn new() -> Messenger {
        Messenger::with_base_url(API_URL)
    }

    pub fn with_base_url(base_url: &str) -> Messenger {
        Messenger {
            graph: GraphHttp::facebook(base_url),
            attachments: GraphHttp::new(
                base_url,
                std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(2),
            ),
        }
    }
}

impl Default for Messenger {
    fn default() -> Messenger {
        Messenger::new()
    }
}

/// The `MessengerAuthValue` fields the handlers read.
#[derive(Debug, Clone, PartialEq)]
pub struct Auth {
    pub access_token: String,
    /// `auth.version ?? DEFAULT_API_VERSION`.
    pub version: String,
    /// `auth.metadata.version` (upload / profile calls), else `version`.
    pub metadata_version: String,
    pub page_id: String,
}

/// Reads the stored `MessengerAuthValue`.
pub fn auth_of(auth: &Value) -> ChannelResult<Auth> {
    let token = auth.pointer("/tokens/accessToken").and_then(Value::as_str);
    let page = auth.pointer("/metadata/pageId").and_then(Value::as_str);
    let (Some(token), Some(page)) = (token, page) else {
        return Err(ChannelError::new(
            "Messenger integration auth is incomplete",
            Category::AuthFailed,
        )
        .with_code("integration_auth_missing")
        .into());
    };
    let version = auth
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_API_VERSION)
        .to_owned();
    let metadata_version = auth
        .pointer("/metadata/version")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map_or_else(|| version.clone(), str::to_owned);
    Ok(Auth {
        access_token: token.to_owned(),
        version,
        metadata_version,
        page_id: page.to_owned(),
    })
}

// ---- error mapping (`lib/error-mapper.ts`) --------------------------------

/// `categorize(code, subCode, type)`.
pub fn categorize(code: Option<i64>, sub_code: Option<i64>, kind: Option<&str>) -> Category {
    const AUTH_FAILED: &[i64] = &[102, 190, 458, 459, 460, 463, 464, 467, 492];
    const PERMISSION_DENIED: &[i64] = &[3, 10, 341, 368];
    const RATE_LIMITED: &[i64] = &[4, 17, 32, 613, 80_001, 80_006];
    const PAYLOAD_INVALID: &[i64] = &[506, 1_609_005];
    const NETWORK_ERROR: &[i64] = &[1, 2];
    let Some(code) = code else {
        return Category::Unknown;
    };
    if AUTH_FAILED.contains(&code) {
        Category::AuthFailed
    } else if RATE_LIMITED.contains(&code) {
        Category::RateLimited
    } else if code == 551 || (code == 200 && sub_code == Some(1_545_041)) {
        Category::UserBlocked
    } else if PERMISSION_DENIED.contains(&code) || (200..=299).contains(&code) {
        Category::PermissionDenied
    } else if NETWORK_ERROR.contains(&code) {
        Category::NetworkError
    } else if PAYLOAD_INVALID.contains(&code) {
        Category::PayloadInvalid
    } else if kind == Some("OAuthException") {
        Category::AuthFailed
    } else {
        Category::Unknown
    }
}

/// `mapToChannelError` of a Graph failure; `endpoint` is `rescue`'s
/// fallback message.
pub fn map_error(error: &GraphError, endpoint: &str) -> ChannelError {
    let category = categorize(
        error.numeric_code(),
        error.numeric_sub_code(),
        error.kind.as_deref(),
    );
    let mut mapped = ChannelError::new(
        error.message.clone().unwrap_or_else(|| endpoint.to_owned()),
        category,
    );
    mapped.code = Some(error.code_text());
    mapped.http_status_code = error.http_status;
    mapped
}

/// `REVOKED_TOKEN_SUBCODES` + `isRevokedTokenError`.
pub fn is_revoked_token_error(error: &GraphError) -> bool {
    error.numeric_code() == Some(190)
        && error
            .numeric_sub_code()
            .is_some_and(|s| [458, 460, 463, 467].contains(&s))
}

/// `isDisconnectSafeError`: the page is gone or the app lost access, so a
/// disconnect may skip the Graph cleanup.
pub fn is_disconnect_safe_error(error: &GraphError) -> bool {
    if is_revoked_token_error(error) {
        return true;
    }
    let Some(code) = error.numeric_code() else {
        return false;
    };
    let message = error.message.as_deref().unwrap_or("").to_lowercase();
    if code == 803 || [10, 200, 210].contains(&code) {
        return true;
    }
    if code == 190 && message.contains("invalid application id") {
        return true;
    }
    if code != 100 {
        return false;
    }
    error.numeric_sub_code() == Some(33) || message.contains("app is not installed")
}

/// `isExpectedPolicyError`: logged as a warning, not an error.
pub fn is_expected_policy_error(error: &GraphError) -> bool {
    match error.numeric_code() {
        Some(230) => true,
        Some(100) => error.numeric_sub_code() == Some(33),
        _ => false,
    }
}

fn log_error(error: &GraphError, endpoint: &str) {
    if is_expected_policy_error(error) {
        tracing::warn!(endpoint, status = error.http_status, code = ?error.code, "Messenger API expected policy error: {}", error.message.as_deref().unwrap_or("unknown"));
    } else {
        tracing::error!(endpoint, status = error.http_status, code = ?error.code, sub_code = ?error.sub_code, "Messenger API error: {}", error.message.as_deref().unwrap_or("unknown"));
    }
}

// ---- messaging policy -----------------------------------------------------

/// `MessengerMessagingPolicy`: `(messaging_type, tag)`.
pub type Policy = (&'static str, Option<&'static str>);

/// `resolveMessagingPolicy`.
pub fn resolve_messaging_policy(
    last_incoming_at: Option<i64>,
    send_from: Option<&str>,
    force_human_agent: bool,
    now_ms: i64,
) -> Result<Policy, ChannelError> {
    if send_from != Some("inbox") {
        return Ok(("RESPONSE", None));
    }
    let Some(last) = last_incoming_at else {
        return Ok(("RESPONSE", None));
    };
    let elapsed = now_ms - last;
    if elapsed <= META_HUMAN_AGENT_WINDOW_MS {
        if force_human_agent || elapsed > META_RESPONSE_WINDOW_MS {
            return Ok(("MESSAGE_TAG", Some("HUMAN_AGENT")));
        }
        return Ok(("RESPONSE", None));
    }
    Err(ChannelError::new(
        "Cannot send a Messenger inbox message more than 7 days after the last incoming message",
        Category::PayloadInvalid,
    )
    .with_code("messenger_human_agent_window_expired"))
}

/// `isForcedHumanAgentSend`.
fn is_forced_human_agent(message: &Value) -> bool {
    message.pointer("/contentAttributes/metadata/bypassThreadControlLock") == Some(&json!(true))
}

/// `buildMessagePayload`.
pub fn build_message_payload(
    psid: &str,
    message: &Value,
    policy: Policy,
    persona_id: Option<&str>,
) -> Value {
    let mut message = message.clone();
    message["metadata"] = json!(MESSAGE_METADATA);
    compact(json!({
        "recipient": { "id": psid },
        "message": message,
        "messaging_type": policy.0,
        "tag": policy.1,
        "persona_id": persona_id,
    }))
}

// ---- outgoing shapes (`handlers/message/outgoing-message/*`) --------------

/// `getAttachmentTemplate`.
fn attachment_template(url: &str, kind: &str) -> Value {
    json!({ "type": kind, "payload": { "url": url, "is_reusable": true } })
}

/// `convertCanonicalQuickReplies`.
pub fn canonical_quick_replies(buttons: &[Value]) -> Vec<Value> {
    buttons
        .iter()
        .map(|button| {
            let payload = canonical_reply_payload(button);
            if payload == NATIVE_USER_EMAIL {
                json!({ "content_type": "user_email" })
            } else if payload == NATIVE_USER_PHONE_NUMBER {
                json!({ "content_type": "user_phone_number" })
            } else {
                json!({ "content_type": "text", "title": str_field(button, "label"), "payload": payload })
            }
        })
        .collect()
}

/// `isMessageButtonTemplate` (and not a native quick reply).
fn is_message_button_template(button: &Value) -> bool {
    let text = |k: &str| button.get(k).and_then(Value::as_str);
    if text("id").is_none() || text("label").is_none() {
        return false;
    }
    match text("buttonType") {
        Some("url") => text("url").is_some(),
        Some("phoneNumber") => text("phoneNumber").is_some() && text("postback").is_some(),
        Some("postback") if text("postback").is_some() => {
            let payload = canonical_reply_payload(button);
            payload != NATIVE_USER_EMAIL && payload != NATIVE_USER_PHONE_NUMBER
        }
        _ => false,
    }
}

/// `toFacebookButton`.
fn to_facebook_button(button: &Value) -> Value {
    let label = str_field(button, "label");
    match str_field(button, "buttonType") {
        "url" => {
            let mut out =
                json!({ "type": "web_url", "title": label, "url": str_field(button, "url") });
            if button.get("messengerExtensions") == Some(&json!(true)) {
                out["messenger_extensions"] = json!(true);
                out["webview_height_ratio"] = json!("full");
            }
            out
        }
        "phoneNumber" => call_button(label, str_field(button, "phoneNumber")),
        _ => {
            json!({ "type": "postback", "title": label, "payload": str_field(button, "postback") })
        }
    }
}

/// `toCallButton`.
fn call_button(title: &str, phone: &str) -> Value {
    json!({ "type": "phone_number", "title": title, "payload": phone })
}

/// `getMessageTemplateButtons`: the button-template buttons of a chat
/// message, when one of them needs the template (a URL or a call).
fn message_template_buttons(message: &Value) -> Vec<Value> {
    let attrs = message.get("contentAttributes").unwrap_or(&Value::Null);
    if attrs.get("type").and_then(Value::as_str) != Some("template")
        || attrs
            .pointer("/payload/templateType")
            .and_then(Value::as_str)
            != Some("button")
    {
        return Vec::new();
    }
    let buttons: Vec<&Value> = attrs
        .pointer("/payload/buttons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| is_message_button_template(b))
        .collect();
    if !buttons
        .iter()
        .any(|b| matches!(str_field(b, "buttonType"), "url" | "phoneNumber"))
    {
        return Vec::new();
    }
    buttons.into_iter().map(to_facebook_button).collect()
}

/// `convertMessage`: the Messenger messages of one stored chat message.
pub fn convert_message(message: &Value) -> Vec<Value> {
    let text = message
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    if message
        .get("contentType")
        .and_then(Value::as_str)
        .unwrap_or("text")
        != "text"
    {
        return vec![
            json!({ "text": message.get("text").and_then(Value::as_str).unwrap_or("not handled yet") }),
        ];
    }
    let mut out = Vec::new();
    let buttons = message_template_buttons(message);
    if let Some(text) = text {
        out.push(if buttons.is_empty() {
            json!({ "text": text })
        } else {
            json!({ "attachment": { "type": "template", "payload": {
                "template_type": "button", "text": text, "buttons": buttons } } })
        });
    }
    let attachments: Vec<&Value> = message
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let url = |a: &Value| str_field(a, "url").to_owned();
    let (images, others): (Vec<&Value>, Vec<&Value>) = attachments
        .into_iter()
        .partition(|a| str_field(a, "fileType") == "image");
    match images.as_slice() {
        [] => {}
        [one] => out.push(json!({ "attachment": attachment_template(&url(one), "image") })),
        many => out.push(json!({ "attachments": many.iter().map(|a| attachment_template(&url(a), "image")).collect::<Vec<_>>() })),
    }
    for attachment in others {
        let kind = match str_field(attachment, "fileType") {
            "video" => "video",
            "audio" => "audio",
            _ => "file",
        };
        out.push(json!({ "attachment": attachment_template(&url(attachment), kind) }));
    }
    out
}

/// `sendMessage`'s messages: [`convert_message`], the native (non-URL)
/// quick replies on the last one.
pub fn chat_messages(message: &Value, quick_replies: Option<&Value>) -> Vec<Value> {
    let mut messages = convert_message(message);
    let native: Vec<Value> = quick_replies
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| str_field(b, "buttonType") != "url")
        .cloned()
        .collect();
    if let Some(last) = messages.last_mut()
        && !native.is_empty()
    {
        last["quick_replies"] = json!(canonical_quick_replies(&native));
    }
    messages
}

/// `getButtonTemplate` of a flow step button.
fn flow_button(data: &SendFlowStep, button: &Value) -> Value {
    let payload = flow_button_payload(
        data,
        str_field(button, "id"),
        Some(data.contact.contact_inbox.id.to_string()),
    );
    let label = str_field(button, "label");
    match str_field(button, "buttonType") {
        "openWebsite" => json!({ "type": "web_url", "title": label,
            "url": append_code_to_magic_link(
                button.pointer("/beforeStep/url").and_then(Value::as_str).unwrap_or(""), &payload) }),
        "callPhoneNumber" => call_button(
            label,
            button
                .pointer("/beforeStep/phoneNumber")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        _ => json!({ "type": "postback", "title": label, "payload": payload }),
    }
}

/// `convertButtons`: the first three buttons.
fn flow_buttons(data: &SendFlowStep, buttons: &[Value]) -> Vec<Value> {
    buttons
        .iter()
        .take(MAX_BUTTONS)
        .map(|b| flow_button(data, b))
        .collect()
}

fn with_quick_replies(mut message: Value, quick_replies: &[Value]) -> Value {
    if !quick_replies.is_empty() {
        message["quick_replies"] = json!(canonical_quick_replies(quick_replies));
    }
    message
}

/// `convertMediaType`.
fn media_type(step_type: &str) -> &'static str {
    match step_type {
        "sendImage" | "sendGif" => "image",
        "sendAudio" => "audio",
        "sendVideo" => "video",
        _ => "file",
    }
}

/// One flow step message, or a media upload to do first.
#[derive(Debug, Clone, PartialEq)]
pub enum StepMessage {
    Ready(Value),
    /// `uploadAttachment(url, type)`, then `build(attachment_id)`.
    Upload {
        url: String,
        media_type: &'static str,
        /// The message with `"__ATTACHMENT_ID__"` where the id goes.
        template: Value,
    },
}

const ATTACHMENT_ID: &str = "__ATTACHMENT_ID__";

fn fill_attachment_id(value: &mut Value, id: &str) {
    match value {
        Value::String(s) if s == ATTACHMENT_ID => *s = id.to_owned(),
        Value::Array(items) => items.iter_mut().for_each(|v| fill_attachment_id(v, id)),
        Value::Object(map) => map.values_mut().for_each(|v| fill_attachment_id(v, id)),
        _ => {}
    }
}

/// `convertFlowStepCarousel` over `cards`.
fn carousel(data: &SendFlowStep, cards: &[Value], layout: &str) -> Vec<StepMessage> {
    let ratio = if layout == "vertical" {
        "square"
    } else {
        "horizontal"
    };
    cards
        .chunks(MAX_CAROUSEL_ELEMENTS)
        .map(|chunk| {
            let elements: Vec<Value> = chunk
                .iter()
                .map(|card| {
                    let buttons: Vec<Value> = card
                        .get("buttons")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|b| flow_button(data, b))
                        .collect();
                    compact(json!({
                        "title": card.get("title"),
                        "subtitle": card.get("subtitle"),
                        "image_url": card.pointer("/image/url"),
                        "buttons": (!buttons.is_empty()).then_some(buttons),
                    }))
                })
                .collect();
            StepMessage::Ready(json!({ "attachment": { "type": "template", "payload": {
                "template_type": "generic", "image_aspect_ratio": ratio, "elements": elements } } }))
        })
        .collect()
}

/// `buildMessengerTemplateComponents` + `buildButtonParameters`.
pub fn template_components(
    params: &Value,
    parameter_format: &str,
    data: Option<&SendFlowStep>,
    flow_buttons: &[Value],
) -> Vec<Value> {
    let named = parameter_format == "NAMED";
    let text_param = |param: &Value| {
        let mut out = json!({ "type": "text", "text": param.get("text").and_then(Value::as_str).unwrap_or("") });
        if named
            && let Some(name) = param
                .get("parameter_name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
        {
            out["parameter_name"] = json!(name);
        }
        out
    };
    let list = |k: &str| {
        params
            .get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut components = Vec::new();
    let header: Vec<Value> = list("header")
        .iter()
        .filter(|p| str_field(p, "type") == "text")
        .map(text_param)
        .collect();
    if !header.is_empty() {
        components.push(json!({ "type": "header", "parameters": header }));
    }
    let body = list("body");
    if !body.is_empty() {
        components.push(json!({ "type": "body", "parameters": body.iter().map(text_param).collect::<Vec<_>>() }));
    }
    let mut indexed: Vec<(i64, Value)> = Vec::new();
    for (fallback, button) in list("button").iter().enumerate() {
        let index = button
            .get("index")
            .and_then(Value::as_i64)
            .unwrap_or(fallback as i64);
        match str_field(button, "sub_type") {
            "url" => indexed.push((
                index,
                json!({ "type": "URL", "url": str_field(button, "text") }),
            )),
            "phone_number" => indexed.push((index, json!({ "type": "PHONE_NUMBER" }))),
            _ => {}
        }
    }
    let mut used: HashSet<i64> = indexed.iter().map(|(i, _)| *i).collect();
    let metadata = data.and_then(|d| d.metadata.as_ref());
    let mut next = 0;
    for button in flow_buttons {
        if str_field(button, "buttonType") == "openMiniApp" {
            continue;
        }
        while used.contains(&next) {
            next += 1;
        }
        let payload = if str_field(button, "buttonType") == "startExternalFlow" {
            encode_button_payload(&ButtonPayload {
                flow_id: button
                    .pointer("/beforeStep/flowId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                broadcast_id: extract_metadata("broadcastId", metadata),
                sequence_step_id: extract_metadata("sequenceStepId", metadata),
                comment_automation_id: extract_metadata("commentAutomationId", metadata),
                ..ButtonPayload::default()
            })
        } else {
            encode_button_payload(&ButtonPayload {
                flow_id: data.map(|d| d.flow_id.clone()).unwrap_or_default(),
                flow_version_id: data.and_then(|d| d.flow_version_id.clone()),
                button_id: Some(str_field(button, "id").to_owned()),
                broadcast_id: extract_metadata("broadcastId", metadata),
                sequence_step_id: extract_metadata("sequenceStepId", metadata),
                comment_automation_id: extract_metadata("commentAutomationId", metadata),
                ..ButtonPayload::default()
            })
        };
        indexed.push((next, json!({ "type": "POSTBACK", "payload": payload })));
        used.insert(next);
    }
    indexed.sort_by_key(|(i, _)| *i);
    if !indexed.is_empty() {
        components.push(json!({ "type": "buttons", "parameters": indexed.into_iter().map(|(_, p)| p).collect::<Vec<_>>() }));
    }
    components
}

/// `buildMessengerTemplateSendRequest`.
pub fn template_send_request(data: &SendFlowStep, persona_id: Option<&str>) -> Value {
    let template = &data.step["template"];
    let empty = Vec::new();
    let buttons = data
        .step
        .get("buttons")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let components = template_components(
        template.get("params").unwrap_or(&Value::Null),
        str_field(template, "parameterFormat"),
        Some(data),
        buttons,
    );
    compact(json!({
        "recipient": { "id": data.contact.contact_inbox.source_id },
        "messaging_type": "UTILITY",
        "persona_id": persona_id,
        "message": {
            "template": {
                "name": template.get("name"),
                "language": { "code": template.get("language") },
                "components": components,
            },
            "metadata": MESSAGE_METADATA,
        },
    }))
}

/// `convertFlowStep` (without the template step).
pub fn convert_flow_step(data: &SendFlowStep) -> ChannelResult<Vec<StepMessage>> {
    let step = &data.step;
    let empty = Vec::new();
    let buttons = step
        .get("buttons")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let quick: Vec<Value> = data
        .quick_replies
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let step_type = str_field(step, "stepType");
    Ok(match step_type {
        "sendText" => {
            let text = str_field(step, "text");
            vec![StepMessage::Ready(if buttons.is_empty() {
                with_quick_replies(json!({ "text": text }), &quick)
            } else {
                with_quick_replies(
                    json!({ "attachment": { "type": "template", "payload": {
                        "template_type": "button", "text": text, "buttons": flow_buttons(data, buttons) } } }),
                    &quick,
                )
            })]
        }
        "sendImage" | "sendVideo" => {
            let url = str_field(step, "url");
            let kind = media_type(step_type);
            if buttons.is_empty() {
                vec![StepMessage::Ready(with_quick_replies(
                    json!({ "attachment": attachment_template(url, kind) }),
                    &quick,
                ))]
            } else {
                let mut element = json!({ "media_type": kind, "attachment_id": ATTACHMENT_ID });
                let fb_buttons = flow_buttons(data, buttons);
                if !fb_buttons.is_empty() {
                    element["buttons"] = json!(fb_buttons);
                }
                vec![StepMessage::Upload {
                    url: url.to_owned(),
                    media_type: kind,
                    template: with_quick_replies(
                        json!({ "attachment": { "type": "template", "payload": {
                            "template_type": "media", "elements": [element] } } }),
                        &quick,
                    ),
                }]
            }
        }
        "sendMultipleImages" => {
            let images: Vec<Value> = step
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|i| json!({ "type": "image", "payload": { "url": str_field(i, "url") } }))
                .collect();
            vec![StepMessage::Ready(with_quick_replies(
                json!({ "attachments": images }),
                &quick,
            ))]
        }
        "sendAudio" | "sendFile" => {
            let kind = media_type(step_type);
            vec![StepMessage::Upload {
                url: str_field(step, "url").to_owned(),
                media_type: kind,
                template: with_quick_replies(
                    json!({ "attachment": { "type": kind, "payload": { "attachment_id": ATTACHMENT_ID } } }),
                    &quick,
                ),
            }]
        }
        "sendGif" => vec![StepMessage::Ready(with_quick_replies(
            json!({ "attachment": attachment_template(str_field(step, "url"), "image") }),
            &quick,
        ))],
        "sendQuickReply" => {
            let text = str_field(step, "message");
            vec![StepMessage::Ready(if buttons.is_empty() {
                json!({ "text": text })
            } else {
                let replies: Vec<Value> = buttons
                    .iter()
                    .map(|b| {
                        json!({ "content_type": "text", "title": str_field(b, "label"),
                        "payload": flow_button_payload(data, str_field(b, "id"), None) })
                    })
                    .collect();
                json!({ "text": text, "quick_replies": replies })
            })]
        }
        "sendCard" => carousel(data, std::slice::from_ref(step), "horizontal"),
        "sendCarousel" => carousel(
            data,
            step.get("cards")
                .and_then(Value::as_array)
                .unwrap_or(&empty),
            str_field(step, "layout"),
        ),
        "sendCollection" => carousel(
            data,
            step.get("cards")
                .and_then(Value::as_array)
                .unwrap_or(&empty),
            "horizontal",
        ),
        other => {
            return Err(SendFailure::Other(format!(
                "Unsupported Messenger flow step: {other}"
            )));
        }
    })
}

/// `resolveMessengerPersonaId`: the contact's registered persona, else the
/// page default.
pub fn resolve_persona_id(
    personas: &Value,
    default_persona_id: Option<&str>,
    contact_persona_id: Option<&str>,
) -> Option<String> {
    if let Some(wanted) = contact_persona_id {
        let found = personas.as_array().into_iter().flatten().find(|p| {
            p.get("id").and_then(Value::as_str) == Some(wanted)
                && p.get("facebookPersonaId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
        });
        if let Some(found) = found {
            return found
                .get("facebookPersonaId")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    default_persona_id
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// The Messenger extension URL of a payload (`getMessengerExtensionUrl`).
fn messenger_extension_url(payload: &Value) -> Option<String> {
    let attachment = payload.pointer("/message/attachment/payload")?;
    let find = |buttons: Option<&Value>| {
        buttons
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|b| b.get("messenger_extensions") == Some(&json!(true)) && b.get("url").is_some())
            .and_then(|b| b.get("url").and_then(Value::as_str).map(str::to_owned))
    };
    find(attachment.get("buttons")).or_else(|| {
        attachment
            .get("elements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|e| find(e.get("buttons")))
    })
}

// ---- Graph calls ----------------------------------------------------------

/// What a send handler needs besides the auth: the client and the persona row.
#[derive(Clone)]
struct Sender {
    graph: GraphHttp,
    attachments: GraphHttp,
    auth: Auth,
}

impl Sender {
    async fn call(&self, request: GraphRequest) -> Result<Value, SendFailure> {
        let endpoint = request.path.clone();
        self.graph.send(request).await.map_err(|e| {
            log_error(&e, &endpoint);
            map_error(&e, &endpoint).into()
        })
    }

    /// `sendMessage` (`POST {version}/me/messages`, no retry).
    async fn send_raw(&self, payload: &Value) -> Result<Value, GraphError> {
        let endpoint = format!("{}/me/messages", self.auth.version);
        self.graph
            .send(
                GraphRequest::post(&endpoint)
                    .bearer(&self.auth.access_token)
                    .json(payload.clone())
                    .retries(0),
            )
            .await
            .inspect_err(|e| log_error(e, &endpoint))
    }

    /// `ensureMessengerWhitelistedDomain` for one URL (https only).
    async fn ensure_domain(&self, url: &str) -> Result<(), SendFailure> {
        let Some(origin) = url::Url::parse(url)
            .ok()
            .filter(|u| u.scheme() == "https")
            .map(|u| u.origin().ascii_serialization())
        else {
            return Ok(());
        };
        let key = format!("{}:{origin}", self.auth.page_id);
        if ensured(&key) {
            return Ok(());
        }
        let endpoint = format!("{}/me/messenger_profile", self.auth.version);
        let current = self
            .call(
                GraphRequest::get(&endpoint)
                    .bearer(&self.auth.access_token)
                    .query("fields", "whitelisted_domains"),
            )
            .await?;
        let mut domains: Vec<Value> = current
            .get("whitelisted_domains")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !domains.iter().any(|d| d.as_str() == Some(origin.as_str())) {
            domains.push(json!(origin));
            self.call(
                GraphRequest::post(&endpoint)
                    .bearer(&self.auth.access_token)
                    .json(json!({ "whitelisted_domains": domains })),
            )
            .await?;
        }
        set_ensured(&key, true);
        Ok(())
    }

    /// `sendPageMessageWithMessengerExtensionWhitelistRetry`.
    async fn send_page_message(&self, payload: &Value) -> Result<Value, SendFailure> {
        let extension_url = messenger_extension_url(payload);
        if let Some(url) = &extension_url {
            self.ensure_domain(url).await?;
        }
        let endpoint = format!("{}/me/messages", self.auth.version);
        match self.send_raw(payload).await {
            Ok(response) => Ok(response),
            Err(error) if error.numeric_sub_code() == Some(DOMAIN_NOT_WHITELISTED_SUBCODE) => {
                if let Some(url) = &extension_url {
                    if let Ok(parsed) = url::Url::parse(url) {
                        set_ensured(
                            &format!(
                                "{}:{}",
                                self.auth.page_id,
                                parsed.origin().ascii_serialization()
                            ),
                            false,
                        );
                    }
                    self.ensure_domain(url).await?;
                }
                self.send_raw(payload)
                    .await
                    .map_err(|e| map_error(&e, &endpoint).into())
            }
            Err(error) => Err(map_error(&error, &endpoint).into()),
        }
    }

    /// `sendPrivateReplyMessage` (`POST {version}/{pageId}/messages`).
    async fn send_private_reply(
        &self,
        comment_id: &str,
        message: &Value,
        persona_id: Option<&str>,
    ) -> Result<Value, SendFailure> {
        let mut message = message.clone();
        message["metadata"] = json!(MESSAGE_METADATA);
        self.call(
            GraphRequest::post(format!(
                "{}/{}/messages",
                self.auth.version, self.auth.page_id
            ))
            .bearer(&self.auth.access_token)
            .json(compact(json!({
                "recipient": { "comment_id": comment_id },
                "message": message,
                "persona_id": persona_id,
            })))
            .retries(0),
        )
        .await
    }

    /// `uploadAttachment`: the reusable attachment id; `None` = the upload
    /// failed (logged, the step sends nothing, as the original's catch).
    async fn upload(&self, url: &str, media_type: &str) -> Option<String> {
        let endpoint = format!("{}/me/message_attachments", self.auth.metadata_version);
        let response = self
            .attachments
            .send(
                GraphRequest::post(&endpoint)
                    .bearer(&self.auth.access_token)
                    .json(json!({ "message": { "attachment": { "type": media_type,
                        "payload": { "is_reusable": true, "url": url } } } }))
                    .retries(2),
            )
            .await;
        match response {
            Ok(body) => body
                .get("attachment_id")
                .and_then(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| Some(v.to_string()))
                })
                .filter(|id| id != "null"),
            Err(error) => {
                log_error(&error, &endpoint);
                tracing::error!("Error uploading media");
                None
            }
        }
    }
}

/// The persona columns of the integration row (`ctx.integrationDetail`).
async fn persona_columns(ctx: &ChannelCtx) -> (Value, Option<String>) {
    let id = ctx.integration_id;
    ctx.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "personas", "personaId" FROM "IntegrationMessenger" WHERE "id" = ?1"#,
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .ok()
        .flatten()
        .map(|(personas, persona_id)| {
            (
                personas
                    .and_then(|p| serde_json::from_str(&p).ok())
                    .unwrap_or(Value::Null),
                persona_id,
            )
        })
        .unwrap_or((Value::Null, None))
}

fn message_id_of(response: &Value) -> Option<String> {
    response
        .get("message_id")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
}

impl Messenger {
    fn sender(&self, ctx: &ChannelCtx) -> ChannelResult<Sender> {
        Ok(Sender {
            graph: self.graph.clone(),
            attachments: self.attachments.clone(),
            auth: auth_of(&ctx.auth)?,
        })
    }
}

fn psid_of(contact: &OutgoingContact) -> ChannelResult<String> {
    let psid = contact.contact_inbox.source_id.clone();
    if psid.is_empty() {
        return Err(SendFailure::other("Missing recipient ID in conversation"));
    }
    Ok(psid)
}

impl Channel for Messenger {
    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let page_id = auth_of(&ctx.auth).map(|a| a.page_id);
        Box::pin(async move { parse_incoming(&page_id?, &payload, now_ms()).map(Some) })
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let sender = self.sender(ctx);
        let ctx = ctx.clone();
        Box::pin(async move {
            let sender = sender?;
            let ci = &data.contact.contact_inbox;
            let policy = resolve_messaging_policy(
                ci.last_incoming_message_at,
                data.send_from.as_deref(),
                is_forced_human_agent(&data.message),
                now_ms(),
            )?;
            let messages = chat_messages(&data.message, data.quick_replies.as_ref());
            let (personas, default_persona) = persona_columns(&ctx).await;
            let persona = resolve_persona_id(
                &personas,
                default_persona.as_deref(),
                ci.persona_id.as_deref(),
            );
            let mut result = SendResult::default();
            for message in messages {
                let payload =
                    build_message_payload(&ci.source_id, &message, policy, persona.as_deref());
                let response = sender.send_page_message(&payload).await?;
                result.sent_count += 1;
                result.message_ids.extend(message_id_of(&response));
            }
            Ok(result)
        })
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let sender = self.sender(ctx);
        let ctx = ctx.clone();
        Box::pin(async move {
            let sender = sender?;
            let ci = &data.contact.contact_inbox;
            let anchor = data
                .comment_anchor
                .as_ref()
                .filter(|a| str_field(a, "replyChannel") == "private");
            let (personas, default_persona) = persona_columns(&ctx).await;
            let persona = resolve_persona_id(
                &personas,
                default_persona.as_deref(),
                ci.persona_id.as_deref(),
            );
            if str_field(&data.step, "stepType") == "sendMessengerTemplateMessage" {
                if anchor.is_some() {
                    assert_private_reply_follow_up(ci.last_incoming_message_at, now_ms())?;
                }
                let payload = template_send_request(&data, persona.as_deref());
                let response = sender.send_page_message(&payload).await?;
                return Ok(SendResult {
                    message_ids: message_id_of(&response).into_iter().collect(),
                    sent_count: 1,
                });
            }
            let policy = resolve_messaging_policy(
                ci.last_incoming_message_at,
                data.send_from.as_deref(),
                false,
                now_ms(),
            )?;
            let mut anchor_comment = anchor
                .filter(|a| a.get("spent") != Some(&json!(true)))
                .map(|a| str_field(a, "commentId").to_owned());
            let mut result = SendResult::default();
            for step_message in convert_flow_step(&data)? {
                let message = match step_message {
                    StepMessage::Ready(message) => message,
                    StepMessage::Upload {
                        url,
                        media_type,
                        mut template,
                    } => {
                        let Some(id) = sender.upload(&url, media_type).await else {
                            continue;
                        };
                        fill_attachment_id(&mut template, &id);
                        template
                    }
                };
                if anchor.is_some() && anchor_comment.is_none() {
                    assert_private_reply_follow_up(ci.last_incoming_message_at, now_ms())?;
                }
                let response = match anchor_comment.take() {
                    Some(comment) => {
                        sender
                            .send_private_reply(&comment, &message, persona.as_deref())
                            .await?
                    }
                    None => {
                        sender
                            .send_page_message(&build_message_payload(
                                &ci.source_id,
                                &message,
                                policy,
                                persona.as_deref(),
                            ))
                            .await?
                    }
                };
                result.sent_count += 1;
                result.message_ids.extend(message_id_of(&response));
            }
            Ok(result)
        })
    }

    fn send_comment(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let Some(parent) = data
                .message
                .pointer("/contentAttributes/replyToCommentId")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                return Err(ChannelError::new(
                    "Cannot send comment reply: replyToCommentId is missing. The outgoing message must be linked to a parent comment.",
                    Category::PayloadInvalid,
                )
                .into());
            };
            let text = data
                .message
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty());
            let attachment_url = data
                .message
                .pointer("/attachments/0/url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty());
            if text.is_none() && attachment_url.is_none() {
                return Ok(SendResult::default());
            }
            let sender = sender?;
            let response = sender
                .call(
                    GraphRequest::post(format!("{}/{parent}/comments", sender.auth.version))
                        .bearer(&sender.auth.access_token)
                        .json(compact(
                            json!({ "message": text, "attachment_url": attachment_url }),
                        ))
                        .retries(0),
                )
                .await?;
            Ok(SendResult {
                message_ids: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect(),
                sent_count: 1,
            })
        })
    }

    fn send_private_reply(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let Some(parent) = data
                .message
                .pointer("/contentAttributes/replyToCommentId")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                return Err(ChannelError::new(
                    "Cannot send private reply: replyToCommentId is missing. The outgoing message must be linked to a parent comment.",
                    Category::PayloadInvalid,
                )
                .into());
            };
            let messages = convert_message(&data.message);
            if messages.is_empty() {
                tracing::warn!(
                    parent,
                    "sendPrivateReply: message has no text or attachments — skipping API call"
                );
                return Ok(SendResult::default());
            }
            let sender = sender?;
            let mut result = SendResult::default();
            for message in messages {
                let response = sender.send_private_reply(&parent, &message, None).await?;
                result.sent_count += 1;
                result.message_ids.extend(message_id_of(&response));
            }
            Ok(result)
        })
    }

    fn delete_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let sender = sender?;
            sender
                .call(
                    GraphRequest::delete(format!("{}/{comment_id}", sender.auth.version))
                        .bearer(&sender.auth.access_token),
                )
                .await?;
            Ok(())
        })
    }

    fn edit_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        new_text: String,
        new_attachment_url: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let sender = sender?;
            let body = match new_attachment_url.filter(|u| !u.is_empty()) {
                Some(url) => json!({ "message": new_text, "attachment_url": url }),
                None => json!({ "message": new_text }),
            };
            sender
                .call(
                    GraphRequest::post(format!("{}/{comment_id}", sender.auth.version))
                        .bearer(&sender.auth.access_token)
                        .json(body),
                )
                .await?;
            Ok(())
        })
    }

    fn like_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        liked: bool,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let sender = sender?;
            let path = format!("{}/{comment_id}/likes", sender.auth.version);
            let request = if liked {
                GraphRequest::post(path)
            } else {
                GraphRequest::delete(path)
            };
            sender
                .call(request.bearer(&sender.auth.access_token))
                .await?;
            Ok(())
        })
    }

    fn hide_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        hidden: bool,
        _post_id: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let sender = sender?;
            sender
                .call(
                    GraphRequest::post(format!("{}/{comment_id}", sender.auth.version))
                        .bearer(&sender.auth.access_token)
                        .json(json!({ "is_hidden": hidden })),
                )
                .await?;
            Ok(())
        })
    }

    fn send_typing(
        &self,
        ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let psid = psid_of(&data.contact)?;
            let sender = sender?;
            let action = if data.typing {
                "typing_on"
            } else {
                "typing_off"
            };
            sender
                .send_raw(&json!({ "recipient": { "id": psid }, "sender_action": action }))
                .await
                .map_err(|e| map_error(&e, &format!("{}/me/messages", sender.auth.version)))?;
            Ok(())
        })
    }

    fn agent_mark_as_read(
        &self,
        ctx: &ChannelCtx,
        contact: OutgoingContact,
        _message_source_id: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let sender = self.sender(ctx);
        Box::pin(async move {
            let psid = psid_of(&contact)?;
            let sender = sender?;
            sender
                .send_raw(&json!({ "recipient": { "id": psid }, "sender_action": "mark_seen" }))
                .await
                .map_err(|e| map_error(&e, &format!("{}/me/messages", sender.auth.version)))?;
            Ok(())
        })
    }
}

// ---- inbound (`incoming-message.ts`, `lib/echo.ts`) -----------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| match x {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// `parseEcho`: `(origin, templateTitle)` of an echo.
fn parse_echo(message: &Value) -> (Option<&'static str>, Option<String>) {
    if message.get("is_echo") != Some(&json!(true)) {
        return (None, None);
    }
    let app_id = match message.get("app_id") {
        Some(Value::String(s)) => Some(s.clone()),
        // `Number.isSafeInteger`: a rounded id is no id.
        Some(Value::Number(n)) => n
            .as_i64()
            .filter(|n| n.unsigned_abs() < (1u64 << 53))
            .map(|n| n.to_string()),
        _ => None,
    };
    let origin = app_id.map(|id| {
        if FIRST_PARTY_ECHO_APP_IDS.contains(&id.as_str()) {
            "firstParty"
        } else {
            "thirdParty"
        }
    });
    let titles = |elements: Option<&Value>| -> Vec<String> {
        elements
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| e.get("title").and_then(Value::as_str))
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let title = message
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| str_field(a, "type") == "template")
        .find_map(|a| {
            let trimmed = |v: Option<&str>| {
                v.map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
            };
            trimmed(a.get("title").and_then(Value::as_str))
                .or_else(|| trimmed(a.pointer("/payload/text").and_then(Value::as_str)))
                .or_else(|| {
                    let mut all = titles(a.pointer("/payload/elements"));
                    all.extend(titles(a.pointer("/payload/product/elements")));
                    (!all.is_empty()).then(|| all.join("\n"))
                })
        });
    (origin, title)
}

/// `messengerTimestampToOccurredAt`: whole seconds, else now.
fn occurred_at(timestamp: Option<&Value>, now: i64) -> i64 {
    let ms = timestamp.and_then(|t| {
        t.as_f64()
            .or_else(|| t.as_str().and_then(|s| s.parse().ok()))
    });
    let ms = match ms {
        Some(ms) if ms.is_finite() && ms > 0.0 => ms as i64,
        _ => now,
    };
    ms - ms.rem_euclid(1000)
}

/// `receiveMessage` of one queued `{ object, entry: [{ messaging: [event] }] }`.
pub fn parse_incoming(page_id: &str, payload: &Value, now: i64) -> ChannelResult<Received> {
    let Some(messaging) = payload.pointer("/entry/0/messaging/0") else {
        // ponytail: standby deliveries (`threadControlEvent` jobs) are not
        // queued yet (thread control, Q.D12); an entry without `messaging`
        // fails as the original's non-standby branch.
        return Err(SendFailure::other("No messaging found"));
    };
    let message = messaging.get("message").filter(|m| m.is_object());
    let postback = messaging.get("postback").filter(|p| p.is_object());
    let referral_top = messaging.get("referral").filter(|r| r.is_object());
    if message.is_none() && postback.is_none() && referral_top.is_none() {
        return Err(SendFailure::other("No message found"));
    }
    let sender = messaging
        .pointer("/sender/id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let recipient = messaging
        .pointer("/recipient/id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let from_page = sender == page_id;
    let mut received = Received {
        contact: IncomingContact {
            source_id: if from_page { recipient } else { sender }.to_owned(),
            ..IncomingContact::default()
        },
        ..Received::default()
    };
    let timestamp = occurred_at(messaging.get("timestamp"), now);
    let is_echo = message.is_some_and(|m| m.get("is_echo") == Some(&json!(true)));
    let (echo_origin, template_title) = message.map(parse_echo).unwrap_or((None, None));
    if let Some(m) = message {
        let mut incoming = IncomingMessage::incoming(s(m, "mid"));
        if from_page {
            incoming.message_type = "outgoing".into();
        }
        // `resolveMessageText`: text, the ad open-thread referral text, the
        // echo template title.
        let referral = m.get("referral").filter(|r| r.is_object());
        let ad_text = referral
            .filter(|r| {
                str_field(r, "source") == "ADS"
                    && str_field(r, "type") == "OPEN_THREAD"
                    && !is_echo
                    && m.get("attachments")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty)
                    && r.get("ref")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
            })
            .and_then(|r| s(r, "text"));
        incoming.text = s(m, "text").or(ad_text).or(template_title);
        let attachments: Vec<&Value> = m
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        if let Some(coords) = attachments
            .iter()
            .find(|a| str_field(a, "type") == "location")
            .and_then(|a| a.pointer("/payload/coordinates"))
        {
            let lat = coords
                .get("latitude")
                .or_else(|| coords.get("lat"))
                .filter(|v| v.is_number());
            let long = coords
                .get("longitude")
                .or_else(|| coords.get("long"))
                .filter(|v| v.is_number());
            if let (Some(lat), Some(long)) = (lat, long) {
                incoming.content_type = "location".into();
                incoming.content_attributes =
                    Some(json!({ "latitude": lat.to_string(), "longitude": long.to_string() }));
            }
        }
        let mut seen = HashSet::new();
        for attachment in attachments {
            let Some(url) = attachment.pointer("/payload/url").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(url.to_owned()) {
                continue;
            }
            let kind = str_field(attachment, "type");
            // ponytail: attachments the live-media rule does not defer are
            // downloaded and stored by the original (`getMessageAttachmentEntity`);
            // here every one keeps its provider URL (a pending attachment).
            if is_deferrable_type(kind)
                && let Some(pending) = pending_attachment(url, kind)
            {
                incoming.attachments.push(pending);
            }
        }
        received.quick_reply_action = m
            .pointer("/quick_reply/payload")
            .and_then(Value::as_str)
            .map(str::to_owned);
        received.button_title = m
            .pointer("/quick_reply/title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        received.message = Some(incoming);
    }
    if let Some(p) = postback {
        let mut incoming = IncomingMessage::incoming(s(p, "mid"));
        incoming.text = s(p, "title");
        received.message = Some(incoming);
        received.postback_action = s(p, "payload");
        received.button_title = s(p, "title");
    }
    let raw_referral = referral_top
        .or_else(|| {
            postback
                .and_then(|p| p.get("referral"))
                .filter(|r| r.is_object())
        })
        .or_else(|| {
            message
                .and_then(|m| m.get("referral"))
                .filter(|r| r.is_object())
        });
    if let Some(raw) = raw_referral {
        let mut stored = raw.clone();
        if let Some(map) = stored.as_object_mut() {
            map.remove("text");
        }
        received.r#ref = s(raw, "ref");
        received.referral_source = s(raw, "source");
        let mut referral = normalize_meta_ad_referral(&stored);
        let captured = chatbotx_core::utils::google_click::consume_google_click_ref(
            raw.get("ref").and_then(Value::as_str),
            Some(timestamp),
            now,
        );
        if let Some(google) = captured.google_referral {
            received.r#ref = captured.reference;
            if let Some(map) = stored.as_object_mut() {
                map.remove("ref");
            }
            referral["ref"] = Value::Null;
            referral["raw"] = stored;
            if let (Some(target), Ok(Value::Object(extra))) =
                (referral.as_object_mut(), serde_json::to_value(google))
            {
                target.extend(extra);
            }
        }
        received.referral = Some(referral);
    }
    received.echo_origin = echo_origin.map(str::to_owned);
    if !is_echo && !from_page {
        received.thread_control = Some(json!({
            "delivery": "owner",
            "occurredAt": chatbotx_db::repo::iso(timestamp),
        }));
    }
    Ok(received)
}

#[cfg(test)]
#[path = "messenger_tests.rs"]
mod tests;
