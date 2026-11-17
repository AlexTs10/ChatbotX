//! `integrations/tiktok` (Q.I2): the Business API client
//! (`lib/http-client.ts`, `exception.ts`), `lib/error-mapper.ts`,
//! `receiveMessage` (`incoming-message.ts`), `sendMessage` / `sendFlowStep`
//! (`outgoing-message/*`), `apis/direct-reply.ts` and the webhook router
//! (`handlers/webhook.ts`, `app/integrations/[...integration]/webhook.ts`
//! `handleTiktokWebhook`). Comment writes (`handlers/comment`) are Q.D10.

use std::sync::Arc;

use chatbotx_core::flow_config::util::{
    ButtonPayload, append_code_to_magic_link, encode_button_payload, extract_metadata,
};
use chatbotx_core::flow_config::{BUTTON_LABEL_MAX, TIKTOK_CARD_TITLE_MAX};
use chatbotx_core::utils::crypto::{hmac_sha256_hex, timing_safe_string_equal};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{Backoff, BoxFuture, Enqueuer, JobOptions, QueueName};
use serde_json::{Value, json};

use super::whatsapp_webhook::{WebhookReply, json_reply};
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingAttachment,
    IncomingContact, IncomingMessage, Received, SendFailure, SendFlowStep, SendMessage, SendResult,
};

/// `BUSINESS_API_URL`.
pub const BUSINESS_API_URL: &str = "https://business-api.tiktok.com/open_api/v1.3/";
/// `MAX_TEMPLATE_BUTTONS`.
pub const MAX_TEMPLATE_BUTTONS: usize = 3;
/// `TIKTOK_MESSAGE_TEXT_LIMIT`.
pub const MESSAGE_TEXT_LIMIT: usize = 6000;
/// `TIKTOK_DIRECT_REPLY_TYPE_COMMENT_TO_MESSAGE`.
pub const DIRECT_REPLY_TYPE: &str = "COMMENT_TO_MESSAGE";
/// The `TikTok-Signature` header.
pub const SIGNATURE_HEADER: &str = "tiktok-signature";
/// `WEBHOOK_TIMESTAMP_WINDOW_SECONDS`.
const TIMESTAMP_WINDOW_SECONDS: i64 = 300;
/// `WEBHOOK_CLOCK_SKEW_SECONDS`.
const CLOCK_SKEW_SECONDS: i64 = 2;
/// ky retry: 3 retries on these statuses (GET and POST).
const RETRY_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];

pub struct Tiktok {
    client: reqwest::Client,
    base_url: String,
}

impl Tiktok {
    pub fn new() -> Tiktok {
        Tiktok::with_base_url(BUSINESS_API_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Tiktok {
        Tiktok {
            client: http_client(),
            base_url: base_url.into(),
        }
    }
}

impl Default for Tiktok {
    fn default() -> Tiktok {
        Tiktok::new()
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

// ---- errors (`exception.ts`, `lib/error-mapper.ts`) ----------------------

/// `TiktokAPIException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub http_status: u16,
    pub code: Option<String>,
    pub message: String,
}

impl ApiError {
    /// `new TiktokAPIException(message)` (400, `tiktokError`).
    pub fn new(message: impl Into<String>) -> ApiError {
        ApiError {
            http_status: 400,
            code: Some("tiktokError".into()),
            message: message.into(),
        }
    }
}

const AUTH_FAILED_PATTERNS: &[&str] = &[
    "access_token_invalid",
    "access_token_expired",
    "token_not_valid",
    "unauthorized",
    "invalid_token",
    "token revoked",
    "auth_failed",
];
const USER_BLOCKED_PATTERNS: &[&str] = &[
    "user_blocked",
    "message_blocked",
    "user_restricted",
    "cannot send message",
    "dm_disabled",
];
const INVALID_RECIPIENT_PATTERNS: &[&str] = &[
    "user_not_found",
    "open_id_not_found",
    "recipient_not_found",
    "invalid_open_id",
];
const PERMISSION_DENIED_PATTERNS: &[&str] = &[
    "permission_denied",
    "scope_not_authorized",
    "insufficient_scope",
    "not_authorized",
];
const RATE_LIMITED_PATTERNS: &[&str] =
    &["rate_limit_exceeded", "too_many_requests", "quota_exceeded"];
const NETWORK_ERROR_PATTERNS: &[&str] = &["etimedout", "econnreset", "webhook error"];
const PAYLOAD_INVALID_PATTERNS: &[&str] = &[
    "message_too_long",
    "invalid_message",
    "bad_request",
    "parameter_invalid",
];

fn matches_any(description: &str, patterns: &[&str]) -> bool {
    let lower = description.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

/// `categorize(httpStatus, description)`.
pub fn categorize(http_status: u16, description: &str) -> Category {
    for (patterns, category) in [
        (AUTH_FAILED_PATTERNS, Category::AuthFailed),
        (USER_BLOCKED_PATTERNS, Category::UserBlocked),
        (INVALID_RECIPIENT_PATTERNS, Category::InvalidRecipient),
        (PERMISSION_DENIED_PATTERNS, Category::PermissionDenied),
        (RATE_LIMITED_PATTERNS, Category::RateLimited),
        (NETWORK_ERROR_PATTERNS, Category::NetworkError),
        (PAYLOAD_INVALID_PATTERNS, Category::PayloadInvalid),
    ] {
        if matches_any(description, patterns) {
            return category;
        }
    }
    match http_status {
        401 => Category::AuthFailed,
        403 => Category::PermissionDenied,
        404 => Category::InvalidRecipient,
        429 => Category::RateLimited,
        400 => Category::PayloadInvalid,
        s if s >= 500 => Category::NetworkError,
        _ => Category::Unknown,
    }
}

/// `mapToChannelError`.
pub fn map_to_channel_error(error: &ApiError) -> ChannelError {
    let mut out = ChannelError::new(
        error.message.clone(),
        categorize(error.http_status, &error.message),
    );
    out.code = Some(error.code.clone().unwrap_or_else(|| "UNKNOWN_ERROR".into()));
    out.http_status_code = error.http_status;
    out
}

/// `isRevokedTokenError`.
pub fn is_revoked_token_error(error: &ApiError) -> bool {
    matches_any(error.code.as_deref().unwrap_or(""), AUTH_FAILED_PATTERNS)
        || matches_any(&error.message, AUTH_FAILED_PATTERNS)
        || error.http_status == 401
}

fn channel_failure(error: ApiError) -> SendFailure {
    SendFailure::Channel(map_to_channel_error(&error))
}

// ---- Business API client -------------------------------------------------

/// A request body: JSON or `multipart/form-data` (boundary, bytes).
pub enum Body {
    None,
    Json(Value),
    Form(String, Vec<u8>),
}

/// One Business API call (`createTiktokBusinessClient(token)` in `rescue`):
/// the `Access-Token` header, ky's retry policy, `{ code, message, data }`
/// with a non-zero `code` rejected.
pub async fn business_call(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    endpoint: &str,
    query: &[(&str, &str)],
    body: &Body,
) -> Result<Value, ApiError> {
    let mut url = format!("{base}{endpoint}");
    if !query.is_empty() {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        url = format!("{url}?{encoded}");
    }
    let mut attempt = 0u64;
    loop {
        let request = match body {
            Body::None => client.get(&url),
            Body::Json(value) => client
                .post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(value.to_string()),
            Body::Form(boundary, bytes) => client
                .post(&url)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(bytes.clone()),
        }
        .header("Access-Token", token);
        let (status, text) = match request.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                (status, response.text().await.unwrap_or_default())
            }
            Err(error) => {
                if attempt < 3 {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
                    continue;
                }
                tracing::error!(err = %error, "TikTok API call failed: {endpoint}");
                return Err(ApiError {
                    http_status: 400,
                    code: Some("tiktokError".into()),
                    message: error.to_string(),
                });
            }
        };
        if !(200..300).contains(&status) {
            if RETRY_STATUSES.contains(&status) && attempt < 3 {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis((300 * attempt).min(1000)))
                    .await;
                continue;
            }
            tracing::error!(status, "TikTok API call failed: {endpoint}");
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let code = &parsed["error"]["code"];
            return Err(ApiError {
                http_status: status,
                code: code
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| code.as_i64().map(|c| c.to_string()))
                    .or_else(|| Some("tiktokError".into())),
                message: parsed["error"]["message"]
                    .as_str()
                    .unwrap_or(endpoint)
                    .to_owned(),
            });
        }
        let parsed: Value =
            serde_json::from_str(&text).map_err(|e| ApiError::new(e.to_string()))?;
        if parsed["code"].as_i64() != Some(0) {
            return Err(ApiError::new(
                parsed["message"]
                    .as_str()
                    .unwrap_or("TikTok API call failed"),
            ));
        }
        return Ok(parsed["data"].clone());
    }
}

/// `sendMessage` (`business/message/send/`): the created message id.
pub async fn send_message_api(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    payload: &Value,
) -> Result<Option<String>, ApiError> {
    let data = business_call(
        client,
        base,
        token,
        "business/message/send/",
        &[],
        &Body::Json(payload.clone()),
    )
    .await
    .map_err(|e| ApiError {
        message: if e.message.is_empty() {
            "TikTok message send failed".into()
        } else {
            e.message
        },
        ..e
    })?;
    let id = data["message"]["message_id"]
        .as_str()
        .or_else(|| data["message_id"].as_str())
        .map(str::to_owned);
    if id.is_none() {
        tracing::warn!(%data, "No message_id in TikTok send response — outgoing message row will keep sourceId=null and its echo will be inserted as a new row");
    }
    Ok(id)
}

/// A `multipart/form-data` body of text fields plus one file part.
/// A hand-built `multipart/form-data` body (the worker `reqwest` has no
/// `multipart` feature): the fields, then `file`.
pub(crate) fn multipart(
    fields: &[(&str, &str)],
    file: &[u8],
    content_type: &str,
) -> (String, Vec<u8>) {
    let boundary = format!("----chatbotx{}", chatbotx_core::utils::id::create_id());
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"blob\"\r\nContent-Type: {content_type}\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(file);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, out)
}

/// `uploadAttachment` (`business/message/media/upload/`): the media id.
pub async fn upload_attachment(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    business_id: &str,
    image_url: &str,
) -> Result<String, ApiError> {
    let failed = || ApiError::new(format!("Failed to fetch image: {image_url}"));
    let response = client.get(image_url).send().await.map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(failed());
    }
    let bytes = response.bytes().await.map_err(|_| failed())?;
    let (boundary, form) = multipart(
        &[("business_id", business_id), ("media_type", "IMAGE")],
        &bytes,
        "application/octet-stream",
    );
    let data = business_call(
        client,
        base,
        token,
        "business/message/media/upload/",
        &[],
        &Body::Form(boundary, form),
    )
    .await?;
    data["media_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ApiError::new("No media_id in TikTok upload response"))
}

/// `getTiktokDirectReplyStatus`.
pub async fn get_direct_reply_status(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    business_id: &str,
) -> Result<Option<String>, ApiError> {
    let data = business_call(
        client,
        base,
        token,
        "business/message/direct_reply/get/",
        &[
            ("business_id", business_id),
            ("direct_reply_type", DIRECT_REPLY_TYPE),
        ],
        &Body::None,
    )
    .await?;
    Ok(data["operation_status"].as_str().map(str::to_owned))
}

/// `updateTiktokDirectReplyStatus`.
pub async fn update_direct_reply_status(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    business_id: &str,
    status: &str,
) -> Result<(), ApiError> {
    business_call(
        client,
        base,
        token,
        "business/message/direct_reply/update/",
        &[],
        &Body::Json(json!({
            "business_id": business_id,
            "direct_reply_type": DIRECT_REPLY_TYPE,
            "operation_status": status,
        })),
    )
    .await
    .map(|_| ())
}

// ---- outgoing (`send-button.ts`, `send-text.ts`) -------------------------

/// `truncateText`: cut by code point.
fn truncate_text(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

const CARD_TYPES: [&str; 2] = ["QA_BUTTON_CARD", "QA_LINK_CARD"];

/// What `getButtonTemplate` needs besides the button.
#[derive(Debug, Clone, Default)]
pub struct ButtonContext {
    pub flow_id: String,
    pub flow_version_id: Option<String>,
    pub metadata: Option<Value>,
    pub contact_inbox_id: Option<String>,
}

/// `getButtonTemplateGroup`: `(card type, REPLY button)`.
pub fn button_template_group(ctx: &ButtonContext, button: &Value) -> (&'static str, Value) {
    let payload = encode_button_payload(&ButtonPayload {
        flow_id: ctx.flow_id.clone(),
        flow_version_id: ctx.flow_version_id.clone(),
        button_id: button["id"].as_str().map(str::to_owned),
        broadcast_id: extract_metadata("broadcastId", ctx.metadata.as_ref()),
        sequence_step_id: extract_metadata("sequenceStepId", ctx.metadata.as_ref()),
        contact_inbox_id: ctx.contact_inbox_id.clone(),
        ..ButtonPayload::default()
    });
    let title = truncate_text(button["label"].as_str().unwrap_or(""), BUTTON_LABEL_MAX);
    if button["buttonType"] == "openWebsite" {
        let url = button["beforeStep"]["url"].as_str().unwrap_or("");
        return (
            "QA_LINK_CARD",
            json!({ "type": "REPLY", "title": title, "id": append_code_to_magic_link(url, &payload) }),
        );
    }
    (
        "QA_BUTTON_CARD",
        json!({ "type": "REPLY", "title": title, "id": payload }),
    )
}

/// `getCanonicalButtonTemplateGroup`: a stored quick reply.
pub fn canonical_button_template_group(button: &Value) -> (&'static str, Value) {
    let title = truncate_text(button["label"].as_str().unwrap_or(""), BUTTON_LABEL_MAX);
    if button["buttonType"] == "url" {
        return (
            "QA_LINK_CARD",
            json!({ "type": "REPLY", "title": title, "id": button["url"] }),
        );
    }
    (
        "QA_BUTTON_CARD",
        json!({ "type": "REPLY", "title": title, "id": button["postback"] }),
    )
}

/// `buildTiktokTemplatesFromGroups` (and `buildTiktokTemplates`, which is
/// the same split for step buttons only).
pub fn templates_from_groups(title: &str, groups: &[(&'static str, Value)]) -> Vec<Value> {
    if !groups.is_empty() && title.chars().count() > TIKTOK_CARD_TITLE_MAX {
        tracing::warn!(
            button_count = groups.len(),
            "TikTok message text exceeds {TIKTOK_CARD_TITLE_MAX} chars with buttons attached; truncating card title"
        );
    }
    let title = truncate_text(title, TIKTOK_CARD_TITLE_MAX);
    let mut out = Vec::new();
    for card in CARD_TYPES {
        let buttons: Vec<&Value> = groups
            .iter()
            .filter(|(t, _)| *t == card)
            .map(|(_, b)| b)
            .collect();
        for chunk in buttons.chunks(MAX_TEMPLATE_BUTTONS) {
            out.push(json!({ "type": card, "title": title, "buttons": chunk }));
        }
    }
    out
}

/// `convertFlowStepText`: one TEXT payload, or the TEMPLATE cards.
pub fn convert_flow_step_text(
    business_id: &str,
    recipient: &str,
    ctx: &ButtonContext,
    step: &Value,
    quick: Option<&Value>,
) -> Vec<Value> {
    let text = step["text"].as_str().unwrap_or("");
    let buttons = step["buttons"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let quick = quick
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let base = json!({ "business_id": business_id, "recipient_type": "CONVERSATION", "recipient": recipient });
    if buttons.is_empty() && quick.is_empty() {
        let mut payload = base;
        payload["message_type"] = json!("TEXT");
        payload["text"] = json!({ "body": text });
        return vec![payload];
    }
    let groups: Vec<(&'static str, Value)> = buttons
        .iter()
        .map(|b| button_template_group(ctx, b))
        .chain(quick.iter().map(canonical_button_template_group))
        .collect();
    templates_from_groups(text, &groups)
        .into_iter()
        .map(|template| {
            let mut payload = base.clone();
            payload["message_type"] = json!("TEMPLATE");
            payload["template"] = template;
            payload
        })
        .collect()
}

/// `requireConversationId`.
fn require_conversation_id(id: Option<&str>) -> ChannelResult<String> {
    id.filter(|id| !id.is_empty()).map(str::to_owned).ok_or_else(|| {
        SendFailure::Channel(
            ChannelError::new(
                "TikTok requires a conversation_id to send messages (recipient_type: CONVERSATION). This contact has no sourceConversationId — wait for an inbound message or backfill the column.",
                Category::InvalidRecipient,
            )
            .with_code("tiktok_missing_conversation_id"),
        )
    })
}

struct Auth {
    token: String,
    business_id: String,
}

fn auth_of(ctx: &ChannelCtx) -> ChannelResult<Auth> {
    let token = ctx.auth["tokens"]["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty());
    let business_id = ctx.auth["metadata"]["openId"]
        .as_str()
        .filter(|t| !t.is_empty());
    match (token, business_id) {
        (Some(token), Some(business_id)) => Ok(Auth {
            token: token.to_owned(),
            business_id: business_id.to_owned(),
        }),
        _ => Err(SendFailure::Channel(
            ChannelError::new(
                "TikTok integration auth is incomplete",
                Category::AuthFailed,
            )
            .with_code("integration_auth_missing"),
        )),
    }
}

fn image_payload(business_id: &str, conversation_id: &str, media_id: &str) -> Value {
    json!({
        "business_id": business_id,
        "recipient_type": "CONVERSATION",
        "recipient": conversation_id,
        "message_type": "IMAGE",
        "image": { "media_id": media_id },
    })
}

/// What one send does: a ready payload, or an image to upload first.
enum Part {
    Payload(Value),
    Image(String),
}

async fn send_parts(
    client: reqwest::Client,
    base: String,
    auth: Auth,
    conversation_id: String,
    parts: Vec<Part>,
) -> ChannelResult<SendResult> {
    let mut result = SendResult::default();
    for part in parts {
        let payload = match part {
            Part::Payload(payload) => payload,
            Part::Image(url) => {
                let media_id =
                    upload_attachment(&client, &base, &auth.token, &auth.business_id, &url)
                        .await
                        .map_err(channel_failure)?;
                image_payload(&auth.business_id, &conversation_id, &media_id)
            }
        };
        let id = send_message_api(&client, &base, &auth.token, &payload)
            .await
            .map_err(|e| {
                tracing::error!(error = %e.message, "An error occurred while sending TikTok message");
                channel_failure(e)
            })?;
        result.sent_count += 1;
        result.message_ids.extend(id);
    }
    Ok(result)
}

impl Channel for Tiktok {
    fn receive_message(
        &self,
        _ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        Box::pin(async move { parse_event(&payload).map(Some).map_err(SendFailure::Other) })
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let (client, base, auth) = (self.client.clone(), self.base_url.clone(), auth_of(ctx));
        Box::pin(async move {
            let auth = auth?;
            let conversation_id =
                require_conversation_id(data.contact.source_conversation_id.as_deref())?;
            let mut parts = Vec::new();
            if let Some(text) = data.message["text"].as_str().filter(|t| !t.is_empty()) {
                parts.push(Part::Payload(json!({
                    "business_id": auth.business_id,
                    "recipient_type": "CONVERSATION",
                    "recipient": conversation_id,
                    "message_type": "TEXT",
                    "text": { "body": text },
                })));
            }
            for attachment in data.message["attachments"].as_array().into_iter().flatten() {
                if attachment["fileType"] == "image"
                    && let Some(url) = attachment["url"].as_str().filter(|u| !u.is_empty())
                {
                    parts.push(Part::Image(url.to_owned()));
                }
            }
            send_parts(client, base, auth, conversation_id, parts).await
        })
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let (client, base, auth) = (self.client.clone(), self.base_url.clone(), auth_of(ctx));
        Box::pin(async move {
            let auth = auth?;
            let conversation_id =
                require_conversation_id(data.contact.source_conversation_id.as_deref())?;
            let parts = match data.step["stepType"].as_str().unwrap_or("") {
                "sendText" => {
                    let ctx = ButtonContext {
                        flow_id: data.flow_id.clone(),
                        flow_version_id: data.flow_version_id.clone(),
                        metadata: data.metadata.clone(),
                        contact_inbox_id: Some(data.contact.contact_inbox.id.to_string()),
                    };
                    convert_flow_step_text(
                        &auth.business_id,
                        &conversation_id,
                        &ctx,
                        &data.step,
                        data.quick_replies.as_ref(),
                    )
                    .into_iter()
                    .map(Part::Payload)
                    .collect()
                }
                "sendImage" => vec![Part::Image(
                    data.step["url"].as_str().unwrap_or("").to_owned(),
                )],
                "sendMultipleImages" => data.step["images"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|image| Part::Image(image["url"].as_str().unwrap_or("").to_owned()))
                    .collect(),
                other => {
                    let error = ApiError {
                        http_status: 400,
                        code: None,
                        message: format!("Unsupported TikTok flow step: {other}"),
                    };
                    return Err(channel_failure(error));
                }
            };
            send_parts(client, base, auth, conversation_id, parts).await
        })
    }
}

// ---- incoming (`incoming-message.ts`) -------------------------------------

/// `detectImageMimeType`.
pub fn detect_image_mime_type(url: &str) -> &'static str {
    let path = url.split('?').next().unwrap_or("");
    match path.rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

fn s(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}

/// `tiktokWebhookEventSchema`.
fn is_event(event: &Value) -> bool {
    event["client_key"].is_string()
        && event["event"].is_string()
        && event["create_time"].is_number()
        && event["user_openid"].is_string()
        && event["content"].is_string()
}

/// `receiveMessage`: a DM event (`im_receive_msg` / `im_send_msg` echo).
pub fn parse_event(event: &Value) -> Result<Received, String> {
    if !is_event(event) {
        return Err("Invalid TikTok webhook event".into());
    }
    let content: Value = serde_json::from_str(event["content"].as_str().unwrap_or(""))
        .map_err(|e| format!("Webhook error: Failed to parse message content: {e}"))?;
    let well_formed = content["from_user"]["id"].is_string()
        && content["conversation_id"].is_string()
        && content["type"].is_string();
    if !well_formed {
        return Err("Unrecognized message content format".into());
    }
    let is_echo = event["event"] == "im_send_msg";
    let kind = content["type"].as_str().unwrap_or("");
    let create_time = event["create_time"].to_string();
    let source_id = s(&content, "message_id").unwrap_or(create_time);
    // `share_post` `.catch(undefined)`: an unexpected shape costs the preview only.
    let share = &content["share_post"];
    let shared = (kind == "share_post" && share["video_id"].is_string()).then(|| {
        let embed = share["embed_url"].as_str();
        (
            embed.or(share["video_id"].as_str()).unwrap_or("").to_owned(),
            json!({ "type": "shared_post", "sharedPost": { "postId": share["video_id"], "url": embed } }),
        )
    });
    let mut message = IncomingMessage::incoming(Some(source_id.clone()));
    if is_echo {
        message.message_type = "outgoing".into();
    }
    message.text = if kind == "text" {
        content["text"]["body"].as_str().map(str::to_owned)
    } else {
        shared.as_ref().map(|(text, _)| text.clone())
    };
    message.content_attributes = shared.map(|(_, attributes)| attributes);
    if kind == "image"
        && let Some(url) = content["media_url"].as_str()
    {
        message.attachments.push(IncomingAttachment {
            source_id: Some(source_id),
            file_type: "image".into(),
            mime_type: detect_image_mime_type(url).into(),
            origin_path: url.to_owned(),
            size: Some(0),
            ..IncomingAttachment::default()
        });
    }
    if message.text.is_none() && message.attachments.is_empty() {
        tracing::warn!(content_type = kind, message_id = ?content["message_id"], "TikTok DM produced an empty message row — unrendered content type");
    }
    let from_id = content["from_user"]["id"].as_str().unwrap_or("").to_owned();
    let to_user = content["to_user"]["id"].as_str();
    let to = content["to"].as_str();
    let (customer, first_name) = if is_echo {
        (
            to_user.or(to).unwrap_or(&from_id).to_owned(),
            to.or(to_user).unwrap_or(&from_id).to_owned(),
        )
    } else {
        (
            from_id.clone(),
            content["from"].as_str().unwrap_or(&from_id).to_owned(),
        )
    };
    let postback = content["reply_source_payload"]["reply_source_unique_id"]
        .as_str()
        .filter(|id| !id.starts_with("http"))
        .map(str::to_owned);
    Ok(Received {
        message: Some(message),
        contact: IncomingContact {
            source_id: customer,
            first_name: Some(first_name),
            last_name: Some(String::new()),
            channel_conversation_id: s(&content, "conversation_id"),
            ..IncomingContact::default()
        },
        postback_action: postback,
        ..Received::default()
    })
}

// ---- webhook (`handlers/webhook.ts`) ---------------------------------------

/// `verifySignature`: `t=<secs>,s=<hmac(secret, "t.body")>`, inside the
/// replay window.
pub fn verify_signature(client_secret: &str, signature: &str, body: &str, now_secs: i64) -> bool {
    let parts: Vec<&str> = signature.split(',').map(str::trim).collect();
    let (Some(t), Some(sig)) = (
        parts.iter().find_map(|p| p.strip_prefix("t=")),
        parts.iter().find_map(|p| p.strip_prefix("s=")),
    ) else {
        return false;
    };
    let Ok(timestamp) = t.parse::<f64>() else {
        return false;
    };
    if !timestamp.is_finite() {
        return false;
    }
    let diff = now_secs as f64 - timestamp;
    if diff < -(CLOCK_SKEW_SECONDS as f64) || diff > TIMESTAMP_WINDOW_SECONDS as f64 {
        return false;
    }
    let payload = format!("{t}.{body}");
    timing_safe_string_equal(&hmac_sha256_hex(client_secret, &payload), sig)
}

/// `quoteSnowflakeIds`: quotes numeric ids so they survive parsing.
fn quote_snowflake_ids(json: &str, fields: &[&str]) -> String {
    let pattern = format!(r#""({})"\s*:\s*(\d+)"#, fields.join("|"));
    regex::Regex::new(&pattern)
        .map(|re| re.replace_all(json, r#""$1":"$2""#).into_owned())
        .unwrap_or_else(|_| json.to_owned())
}

fn opt_str_or_null(value: &Value) -> bool {
    value.is_null() || value.is_string()
}

/// `parseTiktokCommentEventContent`.
pub fn parse_comment_content(content: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(&quote_snowflake_ids(
        content,
        &["comment_id", "video_id", "parent_comment_id"],
    ))
    .ok()?;
    let action_ok = matches!(
        parsed["comment_action"].as_str(),
        Some("insert" | "delete" | "set_to_hidden" | "set_to_friends_only" | "set_to_public")
    );
    let type_ok = matches!(
        parsed.get("comment_type").and_then(Value::as_str),
        None | Some("comment" | "reply")
    ) && parsed.get("comment_type").is_none_or(|v| v.is_string());
    let ok = parsed["comment_id"].is_string()
        && parsed["video_id"].is_string()
        && opt_str_or_null(&parsed["parent_comment_id"])
        && action_ok
        && type_ok
        && (parsed["timestamp"].is_null() || parsed["timestamp"].is_number())
        && (parsed["unique_identifier"].is_null() || parsed["unique_identifier"].is_string())
        && (parsed["text"].is_null() || parsed["text"].is_string());
    ok.then_some(parsed)
}

/// `parseTiktokHighIntentCommentContent`.
pub fn parse_high_intent_content(content: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(&quote_snowflake_ids(
        content,
        &["comment_id", "unique_identifier"],
    ))
    .ok()?;
    let ok = parsed["comment_id"].is_string()
        && (parsed["comment_text"].is_null() || parsed["comment_text"].is_string())
        && (parsed["unique_identifier"].is_null() || parsed["unique_identifier"].is_string())
        && (parsed["is_follower"].is_null() || parsed["is_follower"].is_boolean())
        && (parsed["timestamp"].is_null() || parsed["timestamp"].is_number());
    ok.then_some(parsed)
}

/// `resolveParentCommentId`.
fn parent_comment_id(content: &Value) -> Option<String> {
    let parent = content["parent_comment_id"]
        .as_str()
        .filter(|p| !p.is_empty() && *p != "0")?;
    (content["comment_type"] != "comment").then(|| parent.to_owned())
}

/// The comment's own time in seconds, else the envelope `create_time`.
fn commented_at(content: &Value, event: &Value) -> Value {
    match content["timestamp"].as_f64() {
        Some(ms) if ms != 0.0 => json!((ms / 1000.0).floor() as i64),
        _ => event["create_time"].clone(),
    }
}

/// One job the webhook enqueues.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub name: &'static str,
    pub data: Value,
    pub options: JobOptions,
}

/// `webhookHandler` after the body checks: the job of one verified event,
/// or `None` (answered `ok`).
pub fn route_event(config_open_id: Option<&str>, event: &Value) -> Option<Job> {
    if !is_event(event) {
        tracing::warn!("Invalid TikTok webhook payload");
        return None;
    }
    let identifier = config_open_id
        .or(event["user_openid"].as_str())
        .unwrap_or("")
        .to_owned();
    let base = || json!({ "integrationType": "tiktok", "integrationIdentifier": identifier });
    let content = event["content"].as_str().unwrap_or("");
    match event["event"].as_str().unwrap_or("") {
        "authorization.removed" => {
            tracing::warn!(
                integration_identifier = identifier,
                "TikTok authorization removed — inbox should be marked disconnected"
            );
            None
        }
        "comment.update" => {
            let Some(content) = parse_comment_content(content) else {
                tracing::warn!(
                    integration_identifier = identifier,
                    "Unrecognized TikTok comment event content"
                );
                return None;
            };
            let mut data = base();
            if content["comment_action"] == "delete" {
                data["commentId"] = content["comment_id"].clone();
                return Some(Job {
                    name: "deleteIncomingComment",
                    data: json!({ "type": "deleteIncomingComment", "data": data }),
                    options: JobOptions::default(),
                });
            }
            if content["comment_action"] != "insert" {
                tracing::info!(integration_identifier = identifier, action = ?content["comment_action"], "Ignoring TikTok comment visibility change");
                return None;
            }
            let Some(from_id) = content["unique_identifier"].as_str() else {
                tracing::warn!(
                    integration_identifier = identifier,
                    "TikTok comment has no unique_identifier; cannot identify commenter"
                );
                return None;
            };
            let mut comment = json!({
                "commentId": content["comment_id"],
                "postId": content["video_id"],
                "fromId": from_id,
                "message": content["text"],
                "createdTime": commented_at(&content, event),
            });
            if let Some(parent) = parent_comment_id(&content) {
                comment["parentId"] = json!(parent);
            }
            data["commentData"] = comment;
            Some(Job {
                name: "incomingComment",
                data: json!({ "type": "incomingComment", "data": data }),
                options: JobOptions::default(),
            })
        }
        "im_receive_high_intent_comment" => {
            let Some(content) = parse_high_intent_content(content) else {
                tracing::warn!(
                    integration_identifier = identifier,
                    "Unrecognized TikTok high-intent comment event content"
                );
                return None;
            };
            Some(Job {
                name: "tiktokHighIntentComment",
                data: json!({ "type": "tiktokHighIntentComment", "data": {
                    "integrationType": "tiktok",
                    "integrationIdentifier": identifier,
                    "commentId": content["comment_id"],
                    "commentText": content["comment_text"],
                    "uniqueIdentifier": content["unique_identifier"],
                    "isFollower": content["is_follower"],
                    "commentedAt": commented_at(&content, event),
                }}),
                options: JobOptions {
                    attempts: Some(6),
                    backoff: Some(Backoff::Exponential(30_000)),
                    ..JobOptions::default()
                },
            })
        }
        kind @ ("im_receive_msg" | "im_send_msg") => {
            let is_echo = kind == "im_send_msg";
            let name = if is_echo {
                "incomingMessageEcho"
            } else {
                "incomingMessage"
            };
            Some(Job {
                name,
                data: json!({ "type": name, "data": {
                    "integrationType": "tiktok",
                    "integrationIdentifier": identifier,
                    "payload": event,
                }}),
                options: JobOptions {
                    delay_ms: is_echo.then_some(2000),
                    ..JobOptions::default()
                },
            })
        }
        _ => None,
    }
}

/// `webhookHandler(props)`: body, secret and signature checks, then
/// [`route_event`].
pub fn handle_request(
    client_secret: Option<&str>,
    config_open_id: Option<&str>,
    signature: Option<&str>,
    body: &str,
    now_secs: i64,
) -> Result<Option<Job>, String> {
    if body.is_empty() {
        return Err("Webhook error: Empty webhook payload".into());
    }
    let Some(secret) = client_secret.filter(|s| !s.is_empty()) else {
        return Err("Webhook error: Missing client secret for webhook verification".into());
    };
    let signature = signature.unwrap_or("");
    if signature.is_empty() || !verify_signature(secret, signature, body, now_secs) {
        return Err("Webhook error: Invalid or missing webhook signature".into());
    }
    let event: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(route_event(config_open_id, &event))
}

/// `handleTiktokWebhook` (`/integrations/tiktok/webhook`).
// ponytail: the credential-scoped route (`/integrations/tiktok/webhook/{credentialId}`,
// cloud resellers) is not ported.
pub async fn webhook(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    signature: Option<&str>,
    body: &str,
) -> WebhookReply {
    if body.is_empty() {
        return json_reply(400, "Empty webhook payload");
    }
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let Some(open_id) = parsed["user_openid"].as_str().map(str::to_owned) else {
        return json_reply(400, "Missing user_openid in payload");
    };
    let lookup = open_id.clone();
    let row = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "workspaceId", "inboxId", "openId", "auth" FROM "IntegrationTiktok"
                       WHERE "openId" = ?1 LIMIT 1"#,
                    params![lookup],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await;
    let (workspace_id, inbox_id, account_open_id, auth) = match row {
        Ok(Some(row)) => row,
        Ok(None) => return json_reply(404, "TikTok account not found"),
        Err(e) => return json_reply(400, &e.to_string()),
    };
    // As in the original, the revocation is recorded before the signature
    // check (it carries no state the sender could abuse beyond a reconnect).
    if parsed["event"] == "authorization.removed" {
        match chatbotx_business::connection::mark_unhealthy_by_identifier(
            db,
            "tiktok",
            &open_id,
            workspace_id,
            inbox_id,
        )
        .await
        {
            Ok(true) => tracing::info!(
                open_id,
                "TikTok authorization removed — connection marked unhealthy"
            ),
            Ok(false) => tracing::info!(
                open_id,
                workspace_id,
                "TikTok authorization removed — no Connection row; marked legacy inbox unhealthy"
            ),
            Err(e) => return json_reply(400, &e.to_string()),
        }
        return ok();
    }
    let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
    let now_secs = chatbotx_db::repo::now_ms() / 1000;
    match handle_request(
        auth["clientSecret"].as_str(),
        Some(&account_open_id),
        signature,
        body,
        now_secs,
    ) {
        Ok(None) => ok(),
        Ok(Some(job)) => match jobs
            .add(QueueName::Integration, job.name, job.data, job.options)
            .await
        {
            Ok(_) => ok(),
            Err(e) => {
                tracing::error!(err = %e, integration_type = "tiktok", "TikTok handleRequest failed");
                json_reply(400, &e.to_string())
            }
        },
        Err(message) => {
            tracing::error!(
                err = message,
                integration_type = "tiktok",
                "TikTok handleRequest failed"
            );
            json_reply(400, &message)
        }
    }
}

fn ok() -> WebhookReply {
    WebhookReply {
        status: 200,
        content_type: "text/plain;charset=UTF-8",
        body: "ok".into(),
    }
}

#[cfg(test)]
#[path = "tiktok_tests.rs"]
mod tests;
