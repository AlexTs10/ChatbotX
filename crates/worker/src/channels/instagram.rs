//! `integrations/instagram` (Q.I1b): the Instagram Login channel on
//! `graph.instagram.com`. `lib/error-mapper.ts`, `sendMessage` /
//! `sendFlowStep` (`handlers/message/outgoing-message/*`), the comment
//! handlers, `sendTyping` / `agentMarkAsRead`, `receiveMessage`
//! (`handlers/message/incoming-message.ts`) and the webhook
//! (`handlers/webhook.ts`).
//!
//! `integrations/instagram-facebook` is the same package with a Page token
//! on `graph.facebook.com`; [`Variant::Facebook`] carries its differences
//! (no messaging policy, quick replies on the last message, the Handover
//! retry, private replies on the Page node, comment likes, no location /
//! button title inbound, comments only from `changes[]`).

use std::collections::HashSet;

use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload, extract_metadata};
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::BoxFuture;
use serde_json::{Value, json};

use super::graph::{GraphError, GraphHttp, GraphRequest};
use super::meta::{
    META_HUMAN_AGENT_WINDOW_MS, META_RESPONSE_WINDOW_MS, assert_private_reply_follow_up,
    canonical_reply_payload, is_deferrable_type, normalize_meta_ad_referral, pending_attachment,
    str_field,
};
use super::whatsapp_webhook::WebhookJob;
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingContact, IncomingMessage,
    OutgoingContact, Received, SendFailure, SendFlowStep, SendMessage, SendResult, SendTyping,
};

/// `API_URL`.
pub const API_URL: &str = "https://graph.instagram.com";
/// `DEFAULT_API_VERSION`.
pub const DEFAULT_API_VERSION: &str = "v22.0";
/// Instagram-Facebook `API_URL`.
pub const FACEBOOK_API_URL: &str = "https://graph.facebook.com";
/// Instagram-Facebook `DEFAULT_API_VERSION`.
pub const FACEBOOK_DEFAULT_API_VERSION: &str = "v23.0";
/// `NOT_THREAD_OWNER_CODE` / `NOT_THREAD_OWNER_SUBCODE`.
const NOT_THREAD_OWNER: (i64, i64) = (200, 2_534_037);
/// `INSTAGRAM_MESSAGE_METADATA`.
pub const MESSAGE_METADATA: &str = "SENT_FROM_CHATBOTX";
/// `MAX_BUTTONS`.
const MAX_BUTTONS: usize = 3;
/// `MAX_CAROUSEL_ELEMENTS`.
const MAX_CAROUSEL_ELEMENTS: usize = 10;

/// Which package: Instagram Login (`instagram`) or Instagram via Facebook
/// Login (`instagramFacebook`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Login,
    Facebook,
}

pub struct Instagram {
    graph: GraphHttp,
    /// `instagramAttachmentClient` (Facebook: 60 s, 2 retries).
    upload_graph: GraphHttp,
    variant: Variant,
}

impl Instagram {
    pub fn new() -> Instagram {
        Instagram::with_base_url(API_URL)
    }

    /// `instagramBusinessClient` (30 s, 3 retries, 1 s backoff).
    pub fn with_base_url(base_url: &str) -> Instagram {
        Instagram {
            graph: GraphHttp::facebook(base_url),
            upload_graph: GraphHttp::facebook(base_url),
            variant: Variant::Login,
        }
    }

    /// The Instagram-Facebook channel on `graph.facebook.com`.
    pub fn facebook() -> Instagram {
        Instagram::facebook_with_base_url(FACEBOOK_API_URL)
    }

    /// `instagramGraphClient` (30 s, 3 retries, 1 s) and
    /// `instagramAttachmentClient` (60 s, 2 retries, 2 s).
    pub fn facebook_with_base_url(base_url: &str) -> Instagram {
        Instagram {
            graph: GraphHttp::facebook(base_url),
            upload_graph: GraphHttp::new(
                base_url,
                std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(2),
            ),
            variant: Variant::Facebook,
        }
    }
}

impl Default for Instagram {
    fn default() -> Instagram {
        Instagram::new()
    }
}

/// The `InstagramAuthValue` fields the handlers read.
#[derive(Debug, Clone, PartialEq)]
pub struct Auth {
    pub access_token: String,
    /// `auth.metadata.version ?? DEFAULT_API_VERSION`.
    pub version: String,
    /// The `sendMessage` version: Instagram-Facebook reads the top-level
    /// `auth.version` (`const { version = DEFAULT_API_VERSION } = auth`).
    pub send_version: String,
    pub ig_id: String,
    /// `auth.metadata.pageId` (Login falls back to the IG id).
    pub page_id: String,
    pub has_page_id: bool,
}

pub fn auth_of(auth: &Value) -> ChannelResult<Auth> {
    auth_for(Variant::Login, auth)
}

pub fn auth_for(variant: Variant, auth: &Value) -> ChannelResult<Auth> {
    let text = |p: &str| auth.pointer(p).and_then(Value::as_str);
    let (Some(token), Some(ig_id)) = (text("/tokens/accessToken"), text("/metadata/igId")) else {
        return Err(ChannelError::new(
            "Instagram integration auth is incomplete",
            Category::AuthFailed,
        )
        .with_code("integration_auth_missing")
        .into());
    };
    let default = match variant {
        Variant::Login => DEFAULT_API_VERSION,
        Variant::Facebook => FACEBOOK_DEFAULT_API_VERSION,
    };
    let version = text("/metadata/version")
        .filter(|v| !v.is_empty())
        .unwrap_or(default)
        .to_owned();
    let page_id = text("/metadata/pageId").filter(|p| !p.is_empty());
    Ok(Auth {
        access_token: token.to_owned(),
        send_version: match variant {
            Variant::Login => version.clone(),
            Variant::Facebook => text("/version").unwrap_or(default).to_owned(),
        },
        version,
        ig_id: ig_id.to_owned(),
        page_id: page_id.unwrap_or(ig_id).to_owned(),
        has_page_id: page_id.is_some(),
    })
}

// ---- error mapper ---------------------------------------------------------

/// `categorize` of `lib/error-mapper.ts`.
pub fn categorize(code: Option<i64>, sub_code: Option<i64>, kind: Option<&str>) -> Category {
    let by_sub = match sub_code {
        Some(2_018_028 | 2_207_042) => Some(Category::QuotaExceeded),
        Some(2_018_001) => Some(Category::InvalidRecipient),
        Some(1_545_041) => Some(Category::UserBlocked),
        Some(2_207_050) => Some(Category::PermissionDenied),
        Some(2_207_051) => Some(Category::RateLimited),
        Some(2_207_020 | 2_207_052) => Some(Category::PayloadInvalid),
        _ => None,
    };
    if let Some(category) = by_sub {
        return category;
    }
    let Some(code) = code else {
        return Category::Unknown;
    };
    match code {
        190 => Category::AuthFailed,
        4 | 17 | 613 => Category::RateLimited,
        9 => Category::QuotaExceeded,
        551 => Category::UserBlocked,
        3 | 10 | 24 | 25 | 368 | 200..=299 => Category::PermissionDenied,
        -1 | -2 => Category::NetworkError,
        1 | 100 | 352 | 9004 | 9007 | 36_000 | 36_001 | 36_003 | 36_004 => Category::PayloadInvalid,
        _ if kind == Some("OAuthException") => Category::AuthFailed,
        _ => Category::Unknown,
    }
}

/// `mapToChannelError`: `subCode` counts only as a JSON number.
pub fn map_error(error: &GraphError) -> ChannelError {
    let sub_code = error.sub_code.as_ref().and_then(Value::as_i64);
    let category = categorize(error.numeric_code(), sub_code, error.kind.as_deref());
    let mut mapped = ChannelError::new(
        error
            .message
            .clone()
            .unwrap_or_else(|| "Unknown error.".into()),
        category,
    );
    mapped.code = Some(error.code_text());
    mapped.http_status_code = error.http_status;
    mapped
}

/// `isRevokedTokenError`: code 190 with sub code 458 / 460 / 463 / 467.
pub fn is_revoked_token_error(error: &GraphError) -> bool {
    error.numeric_code() == Some(190)
        && error
            .numeric_sub_code()
            .is_some_and(|s| [458, 460, 463, 467].contains(&s))
}

// ---- messaging policy -----------------------------------------------------

pub type Policy = (&'static str, Option<&'static str>);

/// `resolveMessagingPolicy`.
pub fn resolve_messaging_policy(
    last_incoming_at: Option<i64>,
    send_from: Option<&str>,
    now: i64,
) -> Result<Policy, ChannelError> {
    if send_from != Some("inbox") {
        if let Some(at) = last_incoming_at
            && now - at > META_RESPONSE_WINDOW_MS
        {
            return Err(ChannelError::new(
                "Cannot send an Instagram automated message outside the 24-hour response window",
                Category::PayloadInvalid,
            )
            .with_code("instagram_response_window_expired"));
        }
        return Ok(("RESPONSE", None));
    }
    let Some(at) = last_incoming_at else {
        return Ok(("RESPONSE", None));
    };
    let elapsed = now - at;
    if elapsed <= META_RESPONSE_WINDOW_MS {
        return Ok(("RESPONSE", None));
    }
    if elapsed <= META_HUMAN_AGENT_WINDOW_MS {
        return Ok(("MESSAGE_TAG", Some("HUMAN_AGENT")));
    }
    Err(ChannelError::new(
        "Cannot send an Instagram inbox message more than 7 days after the last incoming message",
        Category::PayloadInvalid,
    )
    .with_code("instagram_human_agent_window_expired"))
}

// ---- outgoing conversion ----------------------------------------------------

/// `getAttachmentTemplate`.
fn attachment_template(url: &str, kind: &str) -> Value {
    json!({ "type": kind, "payload": { "url": url, "is_reusable": true } })
}

/// `convertCanonicalQuickReplies`.
pub fn canonical_quick_replies(buttons: &[Value]) -> Vec<Value> {
    buttons
        .iter()
        .map(|b| json!({ "content_type": "text", "title": str_field(b, "label"), "payload": canonical_reply_payload(b) }))
        .collect()
}

fn quick_replies_of(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .map(|b| canonical_quick_replies(b))
        .unwrap_or_default()
}

/// `withQuickReplies`.
fn with_quick_replies(mut message: Value, quick_replies: &[Value]) -> Value {
    if !quick_replies.is_empty() {
        message["quick_replies"] = json!(quick_replies);
    }
    message
}

/// `convertMessage`: text, then the images (one call for several), then
/// every other attachment on its own.
pub fn convert_message(message: &Value) -> Vec<Value> {
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
    if let Some(text) = message
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        out.push(json!({ "text": text }));
    }
    let attachments: Vec<&Value> = message
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let (images, others): (Vec<&Value>, Vec<&Value>) = attachments
        .into_iter()
        .partition(|a| str_field(a, "fileType") == "image");
    match images.as_slice() {
        [] => {}
        [one] => {
            out.push(json!({ "attachment": attachment_template(str_field(one, "url"), "image") }))
        }
        many => out.push(json!({ "attachments": many.iter()
            .map(|a| attachment_template(str_field(a, "url"), "image")).collect::<Vec<_>>() })),
    }
    for attachment in others {
        let kind = match str_field(attachment, "fileType") {
            "video" => "video",
            "audio" => "audio",
            _ => "file",
        };
        out.push(json!({ "attachment": attachment_template(str_field(attachment, "url"), kind) }));
    }
    out
}

/// `buildMessagePayload` (fails without a recipient).
pub fn build_message_payload(
    recipient: &str,
    message: &Value,
    policy: Option<Policy>,
) -> ChannelResult<Value> {
    if recipient.is_empty() {
        return Err(SendFailure::other("Missing recipient ID in conversation"));
    }
    let mut message = message.clone();
    message["metadata"] = json!(MESSAGE_METADATA);
    let mut payload = json!({ "recipient": { "id": recipient }, "message": message });
    if let Some((kind, tag)) = policy {
        payload["messaging_type"] = json!(kind);
        if let Some(tag) = tag {
            payload["tag"] = json!(tag);
        }
    }
    Ok(payload)
}

/// `encodeButtonPayload` of `getButtonTemplate` / `convertQuickReplies`.
fn button_payload(data: &SendFlowStep, button: &Value) -> String {
    encode_button_payload(&ButtonPayload {
        flow_id: data.flow_id.clone(),
        flow_version_id: data.flow_version_id.clone(),
        button_id: Some(str_field(button, "id").to_owned()),
        comment_automation_id: extract_metadata("commentAutomationId", data.metadata.as_ref()),
        ..ButtonPayload::default()
    })
}

/// `getButtonTemplate`.
fn button_template(data: &SendFlowStep, button: &Value) -> Value {
    if str_field(button, "buttonType") == "openWebsite" {
        return json!({ "type": "web_url", "title": str_field(button, "label"),
            "url": button.pointer("/beforeStep/url") });
    }
    json!({ "type": "postback", "title": str_field(button, "label"), "payload": button_payload(data, button) })
}

fn buttons_of(step: &Value) -> &[Value] {
    step.get("buttons")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
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

/// One flow step message, or a file to upload first (`convertFlowStepFile`).
#[derive(Debug, Clone, PartialEq)]
pub enum StepMessage {
    Ready(Value),
    Upload {
        url: String,
        media_type: &'static str,
        quick_replies: Vec<Value>,
    },
}

/// `convertFlowStep` of Instagram Login.
pub fn convert_flow_step(data: &SendFlowStep) -> ChannelResult<Vec<StepMessage>> {
    convert_flow_step_for(Variant::Login, data)
}

/// `convertFlowStep`; Instagram-Facebook then stamps the step's quick
/// replies on the last message (`sendFlowStep`), whatever its kind.
pub fn convert_flow_step_for(
    variant: Variant,
    data: &SendFlowStep,
) -> ChannelResult<Vec<StepMessage>> {
    let mut messages = convert_flow_step_messages(data)?;
    let quick_replies = quick_replies_of(data.quick_replies.as_ref());
    if variant == Variant::Facebook && !quick_replies.is_empty() {
        match messages.last_mut() {
            Some(StepMessage::Ready(message)) => message["quick_replies"] = json!(quick_replies),
            Some(StepMessage::Upload {
                quick_replies: q, ..
            }) => *q = quick_replies,
            None => {}
        }
    }
    Ok(messages)
}

fn convert_flow_step_messages(data: &SendFlowStep) -> ChannelResult<Vec<StepMessage>> {
    let step = &data.step;
    let step_type = str_field(step, "stepType");
    let quick_replies = quick_replies_of(data.quick_replies.as_ref());
    let ready = |m: Value| vec![StepMessage::Ready(m)];
    Ok(match step_type {
        "sendText" => {
            let text = str_field(step, "text");
            let buttons = buttons_of(step);
            if buttons.is_empty() {
                ready(with_quick_replies(json!({ "text": text }), &quick_replies))
            } else {
                let buttons: Vec<Value> =
                    buttons.iter().take(MAX_BUTTONS).map(|b| button_template(data, b)).collect();
                ready(with_quick_replies(
                    json!({ "attachment": { "type": "template", "payload": {
                        "template_type": "button", "text": text, "buttons": buttons } } }),
                    &quick_replies,
                ))
            }
        }
        "sendImage" | "sendVideo" => {
            if !buttons_of(step).is_empty() {
                tracing::warn!(
                    step_type,
                    button_count = buttons_of(step).len(),
                    "Instagram cannot attach buttons to a media message — sending the media without them"
                );
            }
            ready(with_quick_replies(
                json!({ "attachment": attachment_template(str_field(step, "url"), media_type(step_type)) }),
                &quick_replies,
            ))
        }
        "sendMultipleImages" => {
            let images: Vec<Value> = step
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|i| json!({ "type": "image", "payload": { "url": str_field(i, "url") } }))
                .collect();
            ready(with_quick_replies(json!({ "attachments": images }), &quick_replies))
        }
        "sendAudio" | "sendFile" => vec![StepMessage::Upload {
            url: str_field(step, "url").to_owned(),
            media_type: media_type(step_type),
            quick_replies,
        }],
        "sendGif" => ready(with_quick_replies(
            json!({ "attachment": attachment_template(str_field(step, "url"), "image") }),
            &quick_replies,
        )),
        "sendQuickReply" => {
            let message = str_field(step, "message");
            let buttons = buttons_of(step);
            if buttons.is_empty() {
                ready(json!({ "text": message }))
            } else {
                let replies: Vec<Value> = buttons
                    .iter()
                    .map(|b| json!({ "content_type": "text", "title": str_field(b, "label"), "payload": button_payload(data, b) }))
                    .collect();
                ready(json!({ "text": message, "quick_replies": replies }))
            }
        }
        "sendCarousel" => step
            .get("cards")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .chunks(MAX_CAROUSEL_ELEMENTS)
            .map(|cards| {
                let elements: Vec<Value> = cards
                    .iter()
                    .map(|card| {
                        let buttons = buttons_of(card);
                        super::meta::compact(json!({
                            "title": card.get("title"),
                            "subtitle": card.get("subtitle"),
                            "image_url": card.pointer("/image/url"),
                            "buttons": (!buttons.is_empty()).then(|| buttons.iter().map(|b| button_template(data, b)).collect::<Vec<_>>()),
                        }))
                    })
                    .collect();
                StepMessage::Ready(json!({ "attachment": { "type": "template", "payload": {
                    "template_type": "generic", "elements": elements } } }))
            })
            .collect(),
        other => return Err(SendFailure::other(format!("Unsupported Instagram flow step: {other}"))),
    })
}

fn message_id_of(response: &Value) -> Option<String> {
    response
        .get("message_id")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
}

// ---- Graph calls --------------------------------------------------------------

struct Sender {
    graph: GraphHttp,
    upload_graph: GraphHttp,
    auth: Auth,
    variant: Variant,
}

fn log_graph_error(endpoint: &str, e: &GraphError) {
    tracing::error!(endpoint, status = e.http_status, code = ?e.code, sub_code = ?e.sub_code,
        "Instagram API error: {}", e.message.as_deref().unwrap_or("unknown"));
}

/// `isNotThreadOwnerError`: code 200 + sub code 2534037.
pub fn is_not_thread_owner_error(error: &GraphError) -> bool {
    (
        error.numeric_code(),
        error.sub_code.as_ref().and_then(Value::as_i64),
    ) == (Some(NOT_THREAD_OWNER.0), Some(NOT_THREAD_OWNER.1))
}

impl Sender {
    async fn raw(&self, graph: &GraphHttp, request: GraphRequest) -> Result<Value, GraphError> {
        let endpoint = request.path.clone();
        graph
            .send(request.bearer(&self.auth.access_token))
            .await
            .inspect_err(|e| log_graph_error(&endpoint, e))
    }

    async fn call(&self, request: GraphRequest) -> Result<Value, SendFailure> {
        self.raw(&self.graph, request)
            .await
            .map_err(|e| map_error(&e).into())
    }

    fn send_request(&self, payload: Value) -> GraphRequest {
        GraphRequest::post(format!("{}/me/messages", self.auth.send_version))
            .json(payload)
            .retries(0)
    }

    /// `sendMessage` (`POST {version}/me/messages`, no retry).
    async fn send(&self, payload: Value) -> Result<Value, SendFailure> {
        self.call(self.send_request(payload)).await
    }

    /// One DM. Instagram-Facebook `sendWithHandover`: when another app owns
    /// the thread (2534037), take it back for this IGSID and retry once.
    async fn send_dm(&self, recipient: &str, payload: Value) -> Result<Value, SendFailure> {
        if self.variant == Variant::Login {
            return self.send(payload).await;
        }
        match self
            .raw(&self.graph, self.send_request(payload.clone()))
            .await
        {
            Ok(value) => Ok(value),
            Err(e) if is_not_thread_owner_error(&e) => {
                tracing::info!(
                    "Send refused (2534037): taking thread control for IGSID {recipient} and retrying once"
                );
                self.take_thread_control(recipient).await?;
                self.send(payload).await
            }
            Err(e) => Err(map_error(&e).into()),
        }
    }

    /// `takeThreadControl` (`{version}/{pageId}/take_thread_control`).
    async fn take_thread_control(&self, recipient: &str) -> Result<(), SendFailure> {
        if recipient.is_empty() {
            return Err(ChannelError::new(
                "Cannot take Instagram thread control: no recipient id.",
                Category::Unknown,
            )
            .into());
        }
        if !self.auth.has_page_id {
            return Err(ChannelError::new(
                "Cannot take Instagram thread control: the integration has no pageId. Reconnect the Instagram account.",
                Category::Unknown,
            )
            .into());
        }
        let page_id = &self.auth.page_id;
        let response = self
            .call(
                GraphRequest::post(format!(
                    "{}/{page_id}/take_thread_control",
                    self.auth.version
                ))
                .json(json!({ "recipient": { "id": recipient },
                    "metadata": "ChatbotX handover: send refused with 2534037" }))
                .retries(0),
            )
            .await?;
        if response.get("success") != Some(&json!(true)) {
            return Err(ChannelError::new(
                format!("Instagram take_thread_control was not accepted for page {page_id}"),
                Category::Unknown,
            )
            .into());
        }
        Ok(())
    }

    /// `sendPrivateReplyMessage`: the comment id as the recipient, on
    /// `me/messages` (Login) or the Page node (Facebook, which needs a
    /// `pageId`).
    async fn send_private_reply(
        &self,
        comment_id: &str,
        message: &Value,
    ) -> Result<Value, SendFailure> {
        let mut message = message.clone();
        message["metadata"] = json!(MESSAGE_METADATA);
        let payload = json!({ "recipient": { "comment_id": comment_id }, "message": message });
        if self.variant == Variant::Login {
            return self.send(payload).await;
        }
        if !self.auth.has_page_id {
            return Err(ChannelError::new(
                "Cannot send an Instagram private reply: the integration has no pageId. Reconnect the Instagram account.",
                Category::Unknown,
            )
            .into());
        }
        self.call(
            GraphRequest::post(format!(
                "{}/{}/messages",
                self.auth.version, self.auth.page_id
            ))
            .json(payload)
            .retries(0),
        )
        .await
    }

    /// `uploadAttachment` (`{version}/{pageId}/message_attachments`).
    async fn upload(&self, url: &str, kind: &str) -> Result<Option<String>, SendFailure> {
        let retries = match self.variant {
            Variant::Login => 3,
            Variant::Facebook => 2,
        };
        let response = self
            .raw(
                &self.upload_graph,
                GraphRequest::post(format!(
                    "{}/{}/message_attachments",
                    self.auth.version, self.auth.page_id
                ))
                .retries(retries)
                .json(
                    json!({ "platform": "instagram", "message": { "attachment": {
                        "type": kind, "payload": { "url": url, "is_reusable": true } } } }),
                ),
            )
            .await
            .map_err(|e| SendFailure::from(map_error(&e)))?;
        Ok(response.get("attachment_id").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }))
    }
}

impl Instagram {
    fn sender(&self, ctx: &ChannelCtx) -> ChannelResult<Sender> {
        Ok(Sender {
            graph: self.graph.clone(),
            upload_graph: self.upload_graph.clone(),
            auth: auth_for(self.variant, &ctx.auth)?,
            variant: self.variant,
        })
    }
}

fn recipient_of(contact: &OutgoingContact) -> ChannelResult<String> {
    let id = contact.contact_inbox.source_id.clone();
    if id.is_empty() {
        return Err(SendFailure::other("Missing recipient ID in conversation"));
    }
    Ok(id)
}

fn reply_to_comment_id(message: &Value, what: &str) -> ChannelResult<String> {
    message
        .pointer("/contentAttributes/replyToCommentId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            ChannelError::new(
                format!("Cannot send {what}: replyToCommentId is missing. The outgoing message must be linked to a parent comment."),
                Category::PayloadInvalid,
            )
            .into()
        })
}

impl Channel for Instagram {
    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let ig_id = auth_for(self.variant, &ctx.auth).map(|a| a.ig_id);
        let variant = self.variant;
        Box::pin(async move { parse_incoming_for(variant, &ig_id?, &payload).map(Some) })
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let sender = self.sender(ctx);
        let variant = self.variant;
        Box::pin(async move {
            let ci = &data.contact.contact_inbox;
            // Instagram-Facebook sends with no messaging policy.
            let policy = match variant {
                Variant::Login => Some(resolve_messaging_policy(
                    ci.last_incoming_message_at,
                    data.send_from.as_deref(),
                    now_ms(),
                )?),
                Variant::Facebook => None,
            };
            let sender = sender?;
            let mut messages = convert_message(&data.message);
            let quick_replies = quick_replies_of(data.quick_replies.as_ref());
            if let Some(last) = messages.last_mut()
                && !quick_replies.is_empty()
            {
                last["quick_replies"] = json!(quick_replies);
            }
            let mut result = SendResult::default();
            for message in messages {
                let response = sender
                    .send_dm(
                        &ci.source_id,
                        build_message_payload(&ci.source_id, &message, policy)?,
                    )
                    .await?;
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
        Box::pin(async move {
            let sender = sender?;
            let ci = &data.contact.contact_inbox;
            let anchor = data
                .comment_anchor
                .as_ref()
                .filter(|a| str_field(a, "replyChannel") == "private");
            let mut anchor_comment = anchor
                .filter(|a| a.get("spent") != Some(&json!(true)))
                .map(|a| str_field(a, "commentId").to_owned());
            // Resolved lazily: a comment-anchored first send is exempt from
            // the 24-hour window.
            let mut policy: Option<Policy> = None;
            let mut result = SendResult::default();
            for step_message in convert_flow_step_for(sender.variant, &data)? {
                let message = match step_message {
                    StepMessage::Ready(message) => message,
                    StepMessage::Upload {
                        url,
                        media_type,
                        quick_replies,
                    } => {
                        let id = sender.upload(&url, media_type).await?;
                        with_quick_replies(
                            json!({ "attachment": { "type": media_type, "payload": { "attachment_id": id } } }),
                            &quick_replies,
                        )
                    }
                };
                if anchor.is_some() && anchor_comment.is_none() {
                    assert_private_reply_follow_up(ci.last_incoming_message_at, now_ms())?;
                }
                let response = match anchor_comment.take() {
                    Some(comment) => sender.send_private_reply(&comment, &message).await?,
                    None => {
                        let policy = match (sender.variant, policy) {
                            (Variant::Facebook, _) => None,
                            (Variant::Login, Some(p)) => Some(p),
                            (Variant::Login, None) => {
                                Some(*policy.insert(resolve_messaging_policy(
                                    ci.last_incoming_message_at,
                                    data.send_from.as_deref(),
                                    now_ms(),
                                )?))
                            }
                        };
                        sender
                            .send_dm(
                                &ci.source_id,
                                build_message_payload(&ci.source_id, &message, policy)?,
                            )
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
            let parent = reply_to_comment_id(&data.message, "comment reply")?;
            if data
                .message
                .get("attachments")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                return Err(ChannelError::new(
                    "Instagram comment replies cannot carry media — the reply endpoint accepts text only. Send the image or video as a private reply instead.",
                    Category::PayloadInvalid,
                )
                .into());
            }
            let Some(text) = data
                .message
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            else {
                tracing::warn!(
                    parent,
                    "sendComment: message has no text — skipping API call"
                );
                return Ok(SendResult::default());
            };
            let sender = sender?;
            let response = sender
                .call(
                    GraphRequest::post(format!("{}/{parent}/replies", sender.auth.version))
                        .json(json!({ "message": text }))
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
            let parent = reply_to_comment_id(&data.message, "private reply")?;
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
                let response = sender.send_private_reply(&parent, &message).await?;
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
                .call(GraphRequest::delete(format!(
                    "{}/{comment_id}",
                    sender.auth.version
                )))
                .await?;
            Ok(())
        })
    }

    fn edit_comment(
        &self,
        _ctx: &ChannelCtx,
        comment_id: String,
        _new_text: String,
        _new_attachment_url: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        match self.variant {
            Variant::Login => tracing::info!(
                comment_id,
                "editComment: Instagram does not support editing comments via API, skipping"
            ),
            Variant::Facebook => tracing::info!(
                comment_id,
                "editComment: Facebook does not support editing comments via API, skipping"
            ),
        }
        Box::pin(async { Ok(()) })
    }

    /// Login: a logged no-op. Facebook: `likeComment`
    /// (`POST|DELETE {version}/{igId}/likes?access_token&comment_id`).
    fn like_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        liked: bool,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        if self.variant == Variant::Login {
            tracing::info!(
                comment_id,
                "likeComment: Instagram does not support liking comments via API, skipping"
            );
            return Box::pin(async { Ok(()) });
        }
        let sender = self.sender(ctx);
        Box::pin(async move {
            let sender = sender?;
            let path = format!("{}/{}/likes", sender.auth.version, sender.auth.ig_id);
            let request = if liked {
                GraphRequest::post(path)
            } else {
                GraphRequest::delete(path)
            };
            let request = request
                .query("access_token", sender.auth.access_token.clone())
                .query("comment_id", comment_id.clone());
            let endpoint = request.path.clone();
            sender.graph.send(request).await.map_err(|e| {
                log_graph_error(&endpoint, &e);
                tracing::error!("An error occurred while liking the comment");
                SendFailure::from(map_error(&e))
            })?;
            tracing::info!(
                "Comment {}: {comment_id}",
                if liked { "liked" } else { "unliked" }
            );
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
                        .query("hide", hidden.to_string()),
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
            let recipient = recipient_of(&data.contact)?;
            let action = if data.typing {
                "typing_on"
            } else {
                "typing_off"
            };
            sender?
                .send(json!({ "recipient": { "id": recipient }, "sender_action": action, "messaging_type": "RESPONSE" }))
                .await?;
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
            let recipient = recipient_of(&contact)?;
            sender?
                .send(json!({ "recipient": { "id": recipient }, "sender_action": "mark_seen" }))
                .await?;
            Ok(())
        })
    }
}

// ---- inbound ------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// `getMessageLocation`.
fn location_of(message: &Value) -> Option<Value> {
    let coords = message
        .get("attachments")
        .and_then(Value::as_array)?
        .iter()
        .find(|a| str_field(a, "type") == "location")?
        .pointer("/payload/coordinates")?;
    let pick = |a: &str, b: &str| {
        coords
            .get(a)
            .filter(|v| v.is_number())
            .or_else(|| coords.get(b).filter(|v| v.is_number()))
    };
    let (lat, long) = (pick("latitude", "lat")?, pick("longitude", "long")?);
    Some(json!({ "latitude": lat.to_string(), "longitude": long.to_string() }))
}

/// `hasStoryMention`.
pub fn has_story_mention(message: &Value) -> bool {
    message
        .get("attachments")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|x| str_field(x, "type") == "story_mention"))
}

/// `getMessageContentAttributes`: location (Login only), story reply,
/// story mention.
fn content_attributes(variant: Variant, message: &Value) -> Option<Value> {
    if variant == Variant::Login
        && let Some(location) = location_of(message)
    {
        return Some(location);
    }
    if let Some(story) = message.pointer("/reply_to/story").filter(|s| s.is_object()) {
        return Some(json!({ "type": "story_reply", "story": story }));
    }
    has_story_mention(message).then(|| json!({ "type": "story_mention" }))
}

/// `receiveMessage` + `getMessageEntity` of one single-event payload.
pub fn parse_incoming(ig_id: &str, payload: &Value) -> ChannelResult<Received> {
    parse_incoming_for(Variant::Login, ig_id, payload)
}

/// Instagram-Facebook has no location content type, no `buttonTitle` and
/// no attachment de-duplication.
pub fn parse_incoming_for(
    variant: Variant,
    ig_id: &str,
    payload: &Value,
) -> ChannelResult<Received> {
    let login = variant == Variant::Login;
    let Some(messaging) = payload.pointer("/entry/0/messaging/0") else {
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
    let from_account = sender == ig_id;
    let mut received = Received {
        contact: IncomingContact {
            source_id: if from_account { recipient } else { sender }.to_owned(),
            ..IncomingContact::default()
        },
        ..Received::default()
    };
    if let Some(m) = message {
        let mut incoming = IncomingMessage::incoming(s(m, "mid"));
        if from_account {
            incoming.message_type = "outgoing".into();
        }
        incoming.text = s(m, "text");
        if login && location_of(m).is_some() {
            incoming.content_type = "location".into();
        }
        incoming.content_attributes = content_attributes(variant, m);
        let mut seen = HashSet::new();
        for attachment in m
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = attachment.pointer("/payload/url").and_then(Value::as_str) else {
                continue;
            };
            if login && !seen.insert(url.to_owned()) {
                continue;
            }
            let kind = str_field(attachment, "type");
            // ponytail: non-deferrable attachments (stickers, shares,
            // templates) are downloaded by the original; here only the
            // media types are kept, as pending provider URLs.
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
        if login {
            received.button_title = m
                .pointer("/quick_reply/title")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        received.message = Some(incoming);
    }
    if let Some(p) = postback {
        let mut incoming = IncomingMessage::incoming(s(p, "mid"));
        incoming.text = s(p, "title");
        received.message = Some(incoming);
        received.postback_action = s(p, "payload");
        if login {
            received.button_title = s(p, "title");
        }
    }
    // A `message.referral` without `source` / `type` is dropped (`.catch(undefined)`).
    let valid_referral = |r: &&Value| {
        r.is_object()
            && r.get("source").is_some_and(Value::is_string)
            && r.get("type").is_some_and(Value::is_string)
    };
    let raw = referral_top
        .or_else(|| {
            postback
                .and_then(|p| p.get("referral"))
                .filter(|r| r.is_object())
        })
        .or_else(|| {
            message
                .and_then(|m| m.get("referral"))
                .filter(valid_referral)
        });
    if let Some(raw) = raw {
        received.r#ref = s(raw, "ref");
        received.referral_source = s(raw, "source");
        received.referral = Some(normalize_meta_ad_referral(raw));
    }
    Ok(received)
}

// ---- webhook ------------------------------------------------------------------

fn job(name: &'static str, data: Value) -> WebhookJob {
    WebhookJob {
        name,
        data: json!({ "type": name, "data": data }),
        job_id: String::new(),
    }
}

/// `instagramMessagingEventSchema` (the parts a job reads); Facebook also
/// requires `read.watermark`.
fn is_valid_event(variant: Variant, e: &Value) -> bool {
    let id = |p: &str| e.pointer(p).is_some_and(Value::is_string);
    id("/sender/id")
        && id("/recipient/id")
        && e.get("timestamp").is_some_and(Value::is_number)
        && e.get("message")
            .is_none_or(|m| m.get("mid").is_some_and(Value::is_string))
        && e.get("postback").is_none_or(|p| {
            ["mid", "title", "payload"]
                .iter()
                .all(|k| p.get(*k).is_some_and(Value::is_string))
        })
        && (variant == Variant::Login
            || e.get("read")
                .is_none_or(|r| r.get("watermark").is_some_and(Value::is_number)))
        && e.get("reaction").is_none_or(|r| {
            r.get("mid").is_some_and(Value::is_string)
                && matches!(
                    r.get("action").and_then(Value::as_str),
                    Some("react" | "unreact")
                )
        })
}

/// `handleWebhookEvent` after the signature check: the whole body is
/// skipped when one entry or event is malformed (the schema parses the
/// payload at once).
pub fn webhook_jobs(body: &str) -> Result<Vec<WebhookJob>, String> {
    webhook_jobs_for(Variant::Login, body)
}

/// The Instagram-Facebook webhook (`instagramFacebook` route).
pub fn facebook_webhook_jobs(body: &str) -> Result<Vec<WebhookJob>, String> {
    webhook_jobs_for(Variant::Facebook, body)
}

/// Facebook reads comments only from `changes[]` and tags them
/// `instagramFacebook`; messaging jobs stay `instagram` on both.
pub fn webhook_jobs_for(variant: Variant, body: &str) -> Result<Vec<WebhookJob>, String> {
    let parsed: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let entries = parsed.get("entry").and_then(Value::as_array);
    let well_formed = parsed.get("object").and_then(Value::as_str) == Some("instagram")
        && entries.is_some_and(|entries| {
            entries.iter().all(|entry| {
                entry.get("id").is_some_and(Value::is_string)
                    && entry.get("time").is_some_and(Value::is_number)
                    && entry.get("messaging").is_none_or(|m| {
                        m.as_array()
                            .is_some_and(|m| m.iter().all(|e| is_valid_event(variant, e)))
                    })
            })
        });
    let mut jobs = Vec::new();
    if !well_formed {
        match variant {
            Variant::Login => tracing::warn!("instagram webhook payload unrecognized — skipping"),
            Variant::Facebook => {
                tracing::warn!("instagram facebook webhook payload unrecognized — skipping")
            }
        }
        return Ok(jobs);
    }
    for entry in entries.into_iter().flatten() {
        let id = str_field(entry, "id");
        let time = &entry["time"];
        let top_field = entry
            .get("field")
            .and_then(Value::as_str)
            .filter(|_| variant == Variant::Login);
        let comments: Vec<(&Value, bool)> = match top_field {
            Some(field @ ("comments" | "live_comments")) => {
                vec![(&entry["value"], field == "live_comments")]
            }
            _ => entry
                .get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| match str_field(c, "field") {
                    f @ ("comments" | "live_comments") => Some((&c["value"], f == "live_comments")),
                    _ => None,
                })
                .collect(),
        };
        if !comments.is_empty() {
            for (value, is_live) in comments {
                let valid = value.get("id").is_some_and(Value::is_string)
                    && value.pointer("/from/id").is_some_and(Value::is_string);
                if !valid {
                    tracing::warn!("comment event parse failed — skipping");
                    continue;
                }
                let Some(media_id) = value.pointer("/media/id").and_then(Value::as_str) else {
                    tracing::warn!(
                        comment_id = str_field(value, "id"),
                        "comment webhook missing media.id — skipping"
                    );
                    continue;
                };
                let from_id = value.pointer("/from/id").and_then(Value::as_str);
                let username = value.pointer("/from/username").and_then(Value::as_str);
                let mut comment = super::meta::compact(json!({
                    "commentId": value.get("id"),
                    "postId": media_id,
                    "parentId": value.get("parent_id"),
                    "fromId": from_id,
                    "fromName": username.or(from_id),
                    "fromUsername": username,
                    "message": value.get("text"),
                    "createdTime": time,
                }));
                if is_live {
                    comment["isLive"] = json!(true);
                }
                let integration_type = match variant {
                    Variant::Login => "instagram",
                    Variant::Facebook => "instagramFacebook",
                };
                jobs.push(job("incomingComment", json!({
                    "integrationType": integration_type, "integrationIdentifier": id, "commentData": comment,
                })));
            }
            continue;
        }
        for event in entry
            .get("messaging")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let single = json!({ "object": "instagram", "entry": [{ "id": id, "time": time, "messaging": [event] }] });
            let sender = event
                .pointer("/sender/id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if event.get("read").is_some_and(|r| !r.is_null()) {
                jobs.push(job("contactMarkAsRead", json!({ "integrationType": "instagram",
                    "integrationIdentifier": id, "sourceConversationId": sender, "payload": single })));
                continue;
            }
            if let Some(r) = event.get("reaction").filter(|r| r.is_object()) {
                jobs.push(job("messageReaction", json!({ "integrationType": "instagram", "integrationIdentifier": id,
                    "messageId": r.get("mid"), "action": r.get("action"), "emoji": r.get("emoji"), "contactSourceId": sender })));
                continue;
            }
            let message = event.get("message").filter(|m| m.is_object());
            if let Some(m) = message
                && m.get("is_deleted") == Some(&json!(true))
            {
                jobs.push(job(
                    "deleteIncomingMessage",
                    json!({ "integrationType": "instagram",
                    "integrationIdentifier": id, "messageId": m.get("mid") }),
                ));
                continue;
            }
            if message.is_none()
                && event.get("postback").is_none_or(Value::is_null)
                && event.get("referral").is_none_or(Value::is_null)
            {
                continue;
            }
            let is_echo = message.is_some_and(|m| m.get("is_echo") == Some(&json!(true)));
            if is_echo
                && message
                    .and_then(|m| m.get("metadata"))
                    .and_then(Value::as_str)
                    == Some(MESSAGE_METADATA)
            {
                continue;
            }
            let identifier = if is_echo {
                sender
            } else {
                event
                    .pointer("/recipient/id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            };
            let name = if is_echo {
                "incomingMessageEcho"
            } else {
                "incomingMessage"
            };
            jobs.push(job(name, json!({ "integrationType": "instagram", "integrationIdentifier": identifier, "payload": single })));
        }
    }
    Ok(jobs)
}

#[cfg(test)]
#[path = "instagram_tests.rs"]
mod tests;
