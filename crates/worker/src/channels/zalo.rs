//! `integrations/zalo` (Q.I2): the OA API client (`lib/http-client.ts`,
//! `lib/exception.ts`, `api/message.ts`), `lib/error-mapper.ts`,
//! `receiveMessage` (`incoming-message/index.ts`), `sendMessage` /
//! `sendFlowStep` (`outgoing-message/*`) and `handlers/webhook.ts` behind
//! the platform credential (`[...integration]/webhook.ts`). OAuth, tags and
//! `getProfile` are not channel handlers here.

use std::sync::Arc;

use chatbotx_core::encryption::Encryption;
use chatbotx_core::flow_config::util::{
    ButtonPayload, ZALO_POSTBACK_TEXT_PREFIX, append_code_to_magic_link, encode_button_payload,
    extract_metadata,
};
use chatbotx_core::utils::crypto::{sha256_hex, timing_safe_string_equal};
use chatbotx_db::Db;
use chatbotx_jobs::{BoxFuture, Enqueuer, JobOptions, QueueName};
use serde_json::{Map, Value, json};

use super::tiktok::multipart;
use super::whatsapp_media::{WhatsappMedia, copy_to_storage, media_client};
use super::whatsapp_webhook::{WebhookReply, find_platform_config, json_reply};
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingAttachment,
    IncomingContact, IncomingMessage, Received, SendFailure, SendFlowStep, SendMessage, SendResult,
};

/// `ZALO_API_BASE_URL` (with ky's trailing `/`).
pub const API_BASE_URL: &str = "https://openapi.zalo.me/";
/// `MAX_BUTTONS`.
pub const MAX_BUTTONS: usize = 5;
/// `X-ZEvent-Signature`.
pub const SIGNATURE_HEADER: &str = "x-zevent-signature";
const SEND_MESSAGE: &str = "v3.0/oa/message/cs";
const TAG_EVENT_NAMES: [&str; 3] = ["add_user_to_tag", "remove_user_from_tag", "remove_tag"];

pub struct Zalo {
    client: reqwest::Client,
    base_url: String,
    media: Option<Arc<WhatsappMedia>>,
    media_client: reqwest::Client,
}

impl Zalo {
    pub fn new() -> Zalo {
        Zalo::with_base_url(API_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Zalo {
        Zalo {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            media: None,
            media_client: media_client(),
        }
    }

    pub fn with_media(mut self, media: WhatsappMedia) -> Zalo {
        self.media = Some(Arc::new(media));
        self
    }
}

impl Default for Zalo {
    fn default() -> Zalo {
        Zalo::new()
    }
}

// ---- errors (`exception.ts`, `lib/error-mapper.ts`) ----------------------

/// `ZaloException`: `code` is Zalo's numeric `error` (`None` =
/// `zaloError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub http_status: u16,
    pub code: Option<i64>,
    pub message: String,
}

impl ApiError {
    /// `new ZaloException(message)` (400, `zaloError`).
    pub fn new(message: impl Into<String>) -> ApiError {
        ApiError {
            http_status: 400,
            code: None,
            message: message.into(),
        }
    }
}

const AUTH_FAILED_CODES: &[i64] = &[-101, -103, -104, -124, -1241, -148, -149, -1491, -150];
const PERMISSION_DENIED_CODES: &[i64] = &[-117, -120, -1202, -135, -1351, -136, -138, -1381, -145];
const QUOTA_EXCEEDED_CODES: &[i64] = &[-115, -126, -144, -1441, -147, -1471, -1472, -160];
const USER_BLOCKED_CODES: &[i64] = &[-139, -140, -141, -216];
const INVALID_RECIPIENT_CODES: &[i64] = &[-108, -118];
const PAYLOAD_INVALID_CODES: &[i64] = &[
    -107, -109, -1091, -111, -112, -1121, -1122, -1123, -1124, -113, -1131, -1132, -116, -121,
    -122, -125, -127, -130, -131, -132, -142, -143, -151, -152, -153, -158, -159, -161, -162, -249,
];
const NETWORK_ERROR_CODES: &[i64] = &[-137];

/// `categorize`.
pub fn categorize(code: Option<i64>) -> Category {
    let Some(code) = code else {
        return Category::Unknown;
    };
    for (codes, category) in [
        (AUTH_FAILED_CODES, Category::AuthFailed),
        (PERMISSION_DENIED_CODES, Category::PermissionDenied),
        (QUOTA_EXCEEDED_CODES, Category::QuotaExceeded),
        (USER_BLOCKED_CODES, Category::UserBlocked),
        (INVALID_RECIPIENT_CODES, Category::InvalidRecipient),
        (PAYLOAD_INVALID_CODES, Category::PayloadInvalid),
        (NETWORK_ERROR_CODES, Category::NetworkError),
    ] {
        if codes.contains(&code) {
            return category;
        }
    }
    Category::Unknown
}

/// `mapToChannelError` (a `ZaloException` always carries its status).
pub fn map_to_channel_error(error: &ApiError) -> ChannelError {
    let mut out = ChannelError::new(error.message.clone(), categorize(error.code));
    out.code = Some(
        error
            .code
            .map_or_else(|| "zaloError".to_owned(), |c| c.to_string()),
    );
    out.http_status_code = error.http_status;
    out
}

/// `mapToChannelError` of a plain `Error` (no Zalo code).
fn plain_failure(message: impl Into<String>) -> SendFailure {
    SendFailure::Channel(ChannelError::new(message, Category::Unknown).with_code("UNKNOWN_ERROR"))
}

/// `isRevokedTokenError`: Zalo's `-124`.
pub fn is_revoked_token_error(error: &ApiError) -> bool {
    error.code == Some(-124)
}

fn channel_failure(error: ApiError) -> SendFailure {
    tracing::error!(error = %error.message, "An error occurred while sending the message");
    SendFailure::Channel(map_to_channel_error(&error))
}

// ---- OA API client --------------------------------------------------------

/// A POST body: JSON or `multipart/form-data` (boundary, bytes).
enum Body {
    Json(Value),
    Form(String, Vec<u8>),
}

/// One authenticated `ZaloHttpClient.post`: the `access_token` header, a
/// non-2xx status or a non-zero `error` (or OAuth `error_name`) rejected.
/// ky does not retry POST.
async fn api_post(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    endpoint: &str,
    body: Body,
) -> Result<Value, ApiError> {
    let request = client
        .post(format!("{base}{endpoint}"))
        .header("access_token", token);
    let request = match body {
        Body::Json(value) => request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(value.to_string()),
        Body::Form(boundary, bytes) => request
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(bytes),
    };
    let response = request
        .send()
        .await
        .map_err(|e| ApiError::new(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        tracing::error!(endpoint, status, error = text, "Zalo OA API error response");
        return Err(ApiError {
            http_status: status,
            code: None,
            message: format!("API request failed: {status} {text}"),
        });
    }
    let data: Value = serde_json::from_str(&text).map_err(|e| ApiError::new(e.to_string()))?;
    check_api_error(&data)?;
    Ok(data)
}

/// `handleResponse`'s envelope check.
fn check_api_error(data: &Value) -> Result<(), ApiError> {
    let code = match &data["error"] {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().or(Some(f64::NAN)),
        Value::Null => None,
        _ => Some(f64::NAN),
    };
    let numeric_error = data.get("error").is_some() && code != Some(0.0);
    let oauth_error = data["error_name"].is_string();
    if !(numeric_error || oauth_error) {
        return Ok(());
    }
    let message = ["message", "error_description", "error_reason", "error_name"]
        .iter()
        .find_map(|k| data[*k].as_str())
        .map_or_else(|| format!("Zalo OA API error: {data}"), str::to_owned);
    Err(ApiError {
        http_status: 400,
        code: code.filter(|c| c.fract() == 0.0).map(|c| c as i64),
        message,
    })
}

/// `sendMessageToZaloOA`: the provider `message_id`.
async fn send_message_api(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    recipient: &str,
    message: Value,
) -> Result<Option<String>, ApiError> {
    let payload = json!({ "recipient": { "user_id": recipient }, "message": message });
    let data = api_post(client, base, token, SEND_MESSAGE, Body::Json(payload)).await?;
    Ok(data["data"]["message_id"].as_str().map(str::to_owned))
}

/// `uploadAttachment`: the response `data` (`attachment_id` | `token`,
/// Zalo's own `width` / `height`).
// ponytail: the local `image-size` read is dropped; callers only read
// `data.width` / `data.height`, which come from Zalo.
async fn upload_attachment(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    kind: &str,
    url: &str,
) -> Result<Value, ApiError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::new(e.to_string()))?;
    if !response.status().is_success() {
        return Err(ApiError::new(format!(
            "Failed to fetch file: {}",
            response.status().as_u16()
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::new("No content-type header received"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ApiError::new(e.to_string()))?;
    let endpoint = match kind {
        "image" => "v2.0/oa/upload/image",
        "file" => "v2.0/oa/upload/file",
        "gif" => "v2.0/oa/upload/gif",
        _ => return Err(ApiError::new("Invalid upload type")),
    };
    let (boundary, form) = multipart(&[], &bytes, &content_type);
    let result = api_post(client, base, token, endpoint, Body::Form(boundary, form)).await?;
    Ok(result["data"].clone())
}

// ---- outgoing (`outgoing-message/*`) ---------------------------------------

/// What `getButtonTemplate` needs besides the button.
#[derive(Debug, Clone, Default)]
pub struct ButtonContext {
    pub flow_id: String,
    pub flow_version_id: Option<String>,
    pub metadata: Option<Value>,
    pub contact_inbox_id: Option<String>,
}

/// `toCallButton`: digits with the country code, no `+`.
fn call_button(title: &Value, e164: &str) -> Value {
    json!({
        "type": "oa.open.phone",
        "title": title,
        "payload": { "phone_code": e164.strip_prefix('+').unwrap_or(e164) },
    })
}

/// `getButtonTemplate`.
pub fn button_template(ctx: &ButtonContext, button: &Value) -> Value {
    let payload = encode_button_payload(&ButtonPayload {
        flow_id: ctx.flow_id.clone(),
        flow_version_id: ctx.flow_version_id.clone(),
        button_id: button["id"].as_str().map(str::to_owned),
        broadcast_id: extract_metadata("broadcastId", ctx.metadata.as_ref()),
        sequence_step_id: extract_metadata("sequenceStepId", ctx.metadata.as_ref()),
        contact_inbox_id: ctx.contact_inbox_id.clone(),
        ..ButtonPayload::default()
    });
    let title = &button["label"];
    let before = &button["beforeStep"];
    match button["buttonType"].as_str() {
        Some("openWebsite") => json!({
            "type": "oa.open.url",
            "title": title,
            "payload": { "url": append_code_to_magic_link(before["url"].as_str().unwrap_or(""), &payload) },
        }),
        Some("callPhoneNumber") => call_button(title, before["phoneNumber"].as_str().unwrap_or("")),
        _ => json!({
            "type": "oa.query.hide",
            "title": title,
            "payload": format!("{ZALO_POSTBACK_TEXT_PREFIX}{payload}"),
        }),
    }
}

/// `getCanonicalButtonTemplate`: a stored quick reply.
pub fn canonical_button_template(button: &Value) -> Value {
    let title = &button["label"];
    match button["buttonType"].as_str() {
        Some("url") => {
            json!({ "type": "oa.open.url", "title": title, "payload": { "url": button["url"] } })
        }
        Some("phoneNumber") => call_button(title, button["phoneNumber"].as_str().unwrap_or("")),
        _ => json!({
            "type": "oa.query.hide",
            "title": title,
            "payload": format!("{ZALO_POSTBACK_TEXT_PREFIX}{}", button["postback"].as_str().unwrap_or("")),
        }),
    }
}

/// `convertZaloButtons`: the first [`MAX_BUTTONS`].
fn step_buttons(ctx: &ButtonContext, buttons: &[Value]) -> Vec<Value> {
    buttons
        .iter()
        .take(MAX_BUTTONS)
        .map(|b| button_template(ctx, b))
        .collect()
}

fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `convertFlowStepText`.
pub fn convert_flow_step_text(ctx: &ButtonContext, step: &Value, quick: Option<&Value>) -> Value {
    let buttons = array(step.get("buttons"));
    let quick = array(quick);
    let total = buttons.len() + quick.len();
    if total == 0 {
        return json!({ "text": step["text"] });
    }
    let kept = buttons.len().min(MAX_BUTTONS);
    let kept_quick = &quick[..quick.len().min(MAX_BUTTONS - kept)];
    if kept + kept_quick.len() < total {
        tracing::warn!(
            total,
            kept = MAX_BUTTONS,
            "Zalo template buttons support at most {MAX_BUTTONS}; truncating extra buttons"
        );
    }
    let mut out = step_buttons(ctx, &buttons[..kept]);
    out.extend(kept_quick.iter().map(canonical_button_template));
    json!({
        "text": step["text"],
        "attachment": { "type": "template", "payload": { "buttons": out } },
    })
}

/// One message to send: ready, or an upload first.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Ready(Value),
    /// `media_type` (`image` | `gif`), url, template buttons, and whether
    /// Zalo's `width` / `height` go on the element (flow steps).
    Media {
        media_type: &'static str,
        url: String,
        buttons: Vec<Value>,
        dims: bool,
    },
    File(String),
}

/// `convertMessageToZaloMessage` without the uploads.
pub fn message_parts(message: &Value) -> ChannelResult<Vec<Part>> {
    let text = message["text"].as_str().filter(|t| !t.is_empty());
    let attachments = array(message.get("attachments"));
    if text.is_none() && attachments.is_empty() {
        return Err(plain_failure("Unsupported message type or missing content"));
    }
    let mut parts: Vec<Part> = text
        .map(|t| Part::Ready(json!({ "text": t })))
        .into_iter()
        .collect();
    for attachment in attachments {
        let url = attachment["url"].as_str().unwrap_or("").to_owned();
        parts.push(match attachment["fileType"].as_str() {
            Some("image") => Part::Media {
                media_type: "image",
                url,
                buttons: Vec::new(),
                dims: false,
            },
            Some("file") => Part::File(url),
            other => {
                return Err(plain_failure(format!(
                    "Unsupported attachment type: {}",
                    other.unwrap_or("undefined")
                )));
            }
        });
    }
    Ok(parts)
}

/// `convertFlowStepToZaloMessage` without the uploads.
pub fn flow_step_parts(
    ctx: &ButtonContext,
    step: &Value,
    quick: Option<&Value>,
) -> ChannelResult<Vec<Part>> {
    let url = |value: &Value| value["url"].as_str().unwrap_or("").to_owned();
    match step["stepType"].as_str().unwrap_or("") {
        "sendText" => Ok(vec![Part::Ready(convert_flow_step_text(ctx, step, quick))]),
        kind @ ("sendImage" | "sendGif") => {
            let mut buttons = if kind == "sendImage" {
                step_buttons(ctx, array(step.get("buttons")))
            } else {
                Vec::new()
            };
            buttons.extend(array(quick).iter().map(canonical_button_template));
            if buttons.len() > MAX_BUTTONS {
                tracing::warn!(
                    total = buttons.len(),
                    kept = MAX_BUTTONS,
                    "Zalo template buttons support at most {MAX_BUTTONS}; truncating extra buttons"
                );
                buttons.truncate(MAX_BUTTONS);
            }
            Ok(vec![Part::Media {
                media_type: if kind == "sendGif" { "gif" } else { "image" },
                url: url(step),
                buttons,
                dims: true,
            }])
        }
        "sendMultipleImages" => Ok(array(step.get("images"))
            .iter()
            .map(|image| Part::Media {
                media_type: "image",
                url: url(image),
                buttons: Vec::new(),
                dims: true,
            })
            .collect()),
        "sendFile" => Ok(vec![Part::File(url(step))]),
        other => Err(plain_failure(format!(
            "Unsupported Zalo flow step: {other}"
        ))),
    }
}

/// The message template of one part (uploading first when needed).
async fn resolve(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    part: Part,
) -> Result<Value, ApiError> {
    match part {
        Part::Ready(message) => Ok(message),
        Part::Media {
            media_type,
            url,
            buttons,
            dims,
        } => {
            if url.trim().is_empty() {
                return Err(ApiError::new("Image URL is required"));
            }
            let data = upload_attachment(client, base, token, media_type, &url).await?;
            let Some(id) = data["attachment_id"].as_str().filter(|i| !i.is_empty()) else {
                return Err(ApiError::new(
                    "Failed to upload image: No attachment ID received",
                ));
            };
            let mut element = json!({ "media_type": media_type, "attachment_id": id });
            if dims {
                for key in ["width", "height"] {
                    if data[key].is_number() {
                        element[key] = data[key].clone();
                    }
                }
            }
            let mut payload = json!({ "template_type": "media", "elements": [element] });
            if !buttons.is_empty() {
                payload["buttons"] = Value::Array(buttons);
            }
            Ok(json!({ "attachment": { "type": "template", "payload": payload } }))
        }
        Part::File(url) => {
            if url.trim().is_empty() {
                return Err(ApiError::new("File URL is required"));
            }
            let data = upload_attachment(client, base, token, "file", &url).await?;
            let Some(file_token) = data["token"].as_str().filter(|t| !t.is_empty()) else {
                return Err(ApiError::new("Failed to upload file: No token received"));
            };
            Ok(json!({ "attachment": { "type": "file", "payload": { "token": file_token } } }))
        }
    }
}

fn token_of(ctx: &ChannelCtx) -> String {
    ctx.auth["tokens"]["accessToken"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

/// `buildMessagePayload`'s recipient check.
fn recipient_of(source_id: &str) -> ChannelResult<String> {
    if source_id.trim().is_empty() {
        return Err(plain_failure(
            "Recipient ID is required and cannot be empty",
        ));
    }
    Ok(source_id.to_owned())
}

/// Resolves and sends the parts in order. `upload_first` (`sendMessage`)
/// uploads every attachment before anything is sent, so a bad file fails
/// the send up front and a retry never repeats the text.
pub async fn send_parts(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    source_id: &str,
    parts: Vec<Part>,
    upload_first: bool,
) -> ChannelResult<SendResult> {
    let mut ready = Vec::new();
    let mut pending = parts.into_iter();
    if upload_first {
        for part in pending.by_ref() {
            ready.push(
                resolve(client, base, token, part)
                    .await
                    .map_err(channel_failure)?,
            );
        }
    }
    let mut result = SendResult::default();
    let mut ready = ready.into_iter();
    loop {
        let message = match ready.next() {
            Some(message) => message,
            None => match pending.next() {
                Some(part) => resolve(client, base, token, part)
                    .await
                    .map_err(channel_failure)?,
                None => break,
            },
        };
        let recipient = recipient_of(source_id)?;
        let id = send_message_api(client, base, token, &recipient, message)
            .await
            .map_err(channel_failure)?;
        result.sent_count += 1;
        result.message_ids.extend(id);
        tracing::info!("Message sent for Zalo OA UID: {recipient}");
    }
    Ok(result)
}

impl Channel for Zalo {
    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let token = token_of(ctx);
        let (media, client) = (self.media.clone(), self.media_client.clone());
        let loc = chatbotx_files::ChannelMediaLocation {
            channel: ctx.channel.clone(),
            workspace_id: ctx.workspace_id.to_string(),
            integration_id: ctx.integration_id.to_string(),
        };
        Box::pin(async move {
            let Some((mut received, files)) = parse_event(&payload).map_err(SendFailure::Other)?
            else {
                return Ok(None);
            };
            let mut attachments = Vec::new();
            for (kind, url) in files {
                let mut attachment = IncomingAttachment {
                    source_id: Some(chatbotx_core::utils::id::create_id()),
                    file_type: lazy_file_type(&kind).into(),
                    origin_path: url.clone(),
                    is_photo_message: kind == "image",
                    ..IncomingAttachment::default()
                };
                // ponytail: without a file store the Zalo URL is kept.
                if let Some(media) = &media {
                    // `Promise.allSettled`: a failed download drops only that file.
                    if let Err(error) = copy_to_storage(
                        media,
                        &client,
                        &url,
                        Some(&token),
                        None,
                        &loc,
                        &mut attachment,
                    )
                    .await
                    {
                        tracing::error!(%error, "Get message attachment failed");
                        continue;
                    }
                    attachment.file_type =
                        chatbotx_files::guess_file_type_from_mime_type(&attachment.mime_type)
                            .as_str()
                            .to_owned();
                }
                attachments.push(attachment);
            }
            let Some(message) = received.message.as_mut() else {
                return Ok(None);
            };
            message.attachments = attachments;
            if message.text.as_deref().unwrap_or("").is_empty() && message.attachments.is_empty() {
                tracing::warn!("Zalo message has no content: {}", payload["event_name"]);
                return Ok(None);
            }
            Ok(Some(received))
        })
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let (client, base, token) = (self.client.clone(), self.base_url.clone(), token_of(ctx));
        Box::pin(async move {
            let parts = message_parts(&data.message)?;
            let source_id = &data.contact.contact_inbox.source_id;
            send_parts(&client, &base, &token, source_id, parts, true).await
        })
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let (client, base, token) = (self.client.clone(), self.base_url.clone(), token_of(ctx));
        Box::pin(async move {
            let ctx = ButtonContext {
                flow_id: data.flow_id.clone(),
                flow_version_id: data.flow_version_id.clone(),
                metadata: data.metadata.clone(),
                contact_inbox_id: Some(data.contact.contact_inbox.id.to_string()),
            };
            let parts = flow_step_parts(&ctx, &data.step, data.quick_replies.as_ref())?;
            let source_id = &data.contact.contact_inbox.source_id;
            send_parts(&client, &base, &token, source_id, parts, false).await
        })
    }
}

// ---- incoming (`incoming-message/index.ts`) --------------------------------

/// The file type of an attachment kept by URL (no bytes to sniff).
fn lazy_file_type(kind: &str) -> &'static str {
    match kind {
        "image" | "gif" | "sticker" => "image",
        "video" => "video",
        "audio" => "audio",
        _ => "file",
    }
}

fn id_of(value: &Value) -> Option<&str> {
    value["id"].as_str()
}

/// A parsed event and its attachments (`type`, `url`) to copy.
pub type Parsed = (Received, Vec<(String, String)>);

/// `receiveMessage` before the downloads: the result and the attachments
/// (`type`, `url`) to copy. `None` = not a message event.
pub fn parse_event(event: &Value) -> Result<Option<Parsed>, String> {
    let (message, sender, recipient) = (&event["message"], &event["sender"], &event["recipient"]);
    if !(message.is_object() && sender.is_object() && recipient.is_object()) {
        return Ok(None);
    }
    let Some(msg_id) = message["msg_id"].as_str().filter(|m| !m.is_empty()) else {
        return Err("Missing msg_id in message event".into());
    };
    let name = event["event_name"].as_str().unwrap_or("");
    let user_sent = name.contains("user_send");
    let mut out = IncomingMessage::incoming(Some(msg_id.to_owned()));
    if !user_sent {
        out.message_type = "outgoing".into();
    }
    out.text = message["text"].as_str().map(str::to_owned);
    let files: Vec<(String, String)> = array(message.get("attachments"))
        .iter()
        .filter_map(|a| {
            let url = a["payload"]["url"].as_str().filter(|u| !u.is_empty())?;
            Some((a["type"].as_str().unwrap_or("").to_owned(), url.to_owned()))
        })
        .collect();
    let files = match name {
        "user_send_text" | "oa_send_text" | "user_send_image" | "oa_send_image"
        | "user_send_sticker" | "oa_send_sticker" | "user_send_file" | "oa_send_file"
        | "user_send_audio" | "user_send_gif" | "oa_send_gif" | "user_send_video"
        | "oa_send_video" => files,
        "user_send_location" => {
            let coords = &message["attachments"][0]["payload"]["coordinates"];
            out.text = Some(if coords.is_object() {
                format!(
                    "https://www.google.com/maps/search/?api=1&query={},{}",
                    js_string(&coords["latitude"]),
                    js_string(&coords["longitude"])
                )
            } else {
                "Location".into()
            });
            out.content_type = "location".into();
            let mut attrs = Map::new();
            for key in ["latitude", "longitude"] {
                if !coords[key].is_null() {
                    attrs.insert(key.into(), coords[key].clone());
                }
            }
            out.content_attributes = Some(Value::Object(attrs));
            files
        }
        _ => Vec::new(),
    };
    let postback_action = out
        .text
        .as_deref()
        .and_then(|t| t.strip_prefix(ZALO_POSTBACK_TEXT_PREFIX))
        .map(str::to_owned);
    let source_id = if user_sent {
        id_of(sender)
    } else {
        id_of(recipient)
    };
    Ok(Some((
        Received {
            message: Some(out),
            contact: IncomingContact {
                source_id: source_id.unwrap_or("").to_owned(),
                ..IncomingContact::default()
            },
            postback_action,
            ..Received::default()
        },
        files,
    )))
}

/// `${value}` of a JSON scalar.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "undefined".into(),
        other => other.to_string(),
    }
}

// ---- webhook (`handlers/webhook.ts`) ---------------------------------------

/// One job to enqueue.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub name: &'static str,
    pub data: Value,
}

/// The keys `zaloWebhookEventSchema` keeps (zod strips the others).
const EVENT_KEYS: [&str; 11] = [
    "app_id",
    "oa_id",
    "event_name",
    "timestamp",
    "sender",
    "recipient",
    "message",
    "follower",
    "source",
    "user_id_by_app",
    "tag",
];

/// `zaloWebhookEventSchema.parse`.
// ponytail: top-level shape only (nested keys are kept as sent), and the
// error text is fixed, not zod's issue list.
fn parse_schema(body: &Value) -> Result<Value, String> {
    let ok = body["app_id"].is_string()
        && body["event_name"].is_string()
        && ["oa_id", "timestamp", "source", "user_id_by_app"]
            .iter()
            .all(|k| body[*k].is_null() || body[*k].is_string())
        && ["sender", "recipient", "follower"]
            .iter()
            .all(|k| body[*k].is_null() || id_of(&body[*k]).is_some())
        && (body["message"].is_null() || body["message"].is_object());
    if !ok {
        return Err("Invalid Zalo webhook payload".into());
    }
    let object = body.as_object().cloned().unwrap_or_default();
    Ok(Value::Object(
        object
            .into_iter()
            .filter(|(k, v)| EVENT_KEYS.contains(&k.as_str()) && !v.is_null())
            .collect(),
    ))
}

/// `zaloTimestampToIso`: epoch ms in a string, else now.
fn timestamp_iso(timestamp: &Value) -> String {
    let ms = timestamp
        .as_str()
        .map(|t| {
            if t.trim().is_empty() {
                Some(0.0)
            } else {
                t.trim().parse::<f64>().ok()
            }
        })
        .unwrap_or(None)
        .filter(|m| m.is_finite() && *m > 0.0)
        .map_or_else(chatbotx_db::repo::now_ms, |m| m as i64);
    chatbotx_core::js::to_iso_string(ms).unwrap_or_default()
}

/// `parseZaloFollowEvent`: the `channelAudienceEvent` data.
pub fn parse_follow_event(event: &Value) -> Option<Value> {
    let name = event["event_name"].as_str()?;
    if name != "follow" && name != "unfollow" {
        return None;
    }
    let oa_id = event["oa_id"].as_str().filter(|o| !o.is_empty())?;
    let follower = id_of(&event["follower"]).filter(|f| !f.is_empty())?;
    let mut inner = json!({
        "kind": name,
        "user": { "sourceId": follower },
        "occurredAt": timestamp_iso(&event["timestamp"]),
    });
    if let Some(source) = event["source"].as_str().filter(|s| !s.is_empty()) {
        inner["source"] = json!(source);
    }
    Some(json!({
        "integrationType": "zalo",
        "integrationIdentifier": oa_id,
        "event": inner,
    }))
}

/// `verifyRawWebhookSignature`: `mac=sha256(app_id + rawBody + timestamp +
/// verifyToken)`.
pub fn verify_raw_signature(
    raw: &str,
    event: &Value,
    signature: &str,
    verify_token: Option<&str>,
) -> bool {
    let hash = signature.split('=').nth(1).unwrap_or("");
    let Some(secret) = verify_token.filter(|s| !s.is_empty()) else {
        return false;
    };
    if hash.is_empty() {
        return false;
    }
    let expected = sha256_hex(&format!(
        "{}{raw}{}{secret}",
        js_string(&event["app_id"]),
        js_string(&event["timestamp"])
    ));
    timing_safe_string_equal(hash, &expected)
}

fn job(name: &'static str, data: Value) -> Option<Job> {
    Some(Job {
        name,
        data: json!({ "type": name, "data": data }),
    })
}

/// `handleWebhookEvent` without the queue: the job of one event, `None` =
/// ack and drop, `Err` = the inner error text.
pub fn route_event(
    client_id: Option<&str>,
    verify_token: Option<&str>,
    signature: Option<&str>,
    raw: &str,
) -> Result<Option<Job>, String> {
    let body: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(raw).map_err(|e| e.to_string())?
    };
    if body.is_null() || body == false || body == 0 || body == "" {
        return Err("Empty webhook payload".into());
    }
    let event = parse_schema(&body)?;
    let name = event["event_name"].as_str().unwrap_or("");
    let oa_id = event["oa_id"].as_str().filter(|o| !o.is_empty());
    if let Some(oa_id) = oa_id.filter(|_| TAG_EVENT_NAMES.contains(&name)) {
        return Ok(job(
            "channelLabelChange",
            json!({ "integrationType": "zalo", "integrationIdentifier": oa_id, "payload": event }),
        ));
    }
    if event["app_id"].as_str() != client_id {
        return Err("Invalid app_id in webhook payload".into());
    }
    if let Some(follow) = parse_follow_event(&event) {
        if !verify_raw_signature(raw, &event, signature.unwrap_or(""), verify_token) {
            // Ack (no retry) but never score an unverifiable follow.
            return Ok(None);
        }
        return Ok(job("channelAudienceEvent", follow));
    }
    if name == "follow" || name == "unfollow" {
        return Ok(None);
    }
    let (Some(sender), Some(recipient)) = (id_of(&event["sender"]), id_of(&event["recipient"]))
    else {
        return Err("Missing sender/recipient in message event".into());
    };
    match name {
        "user_received_message" => return Ok(None),
        "user_seen_message" => {
            return Ok(job(
                "contactMarkAsRead",
                json!({
                    "integrationType": "zalo",
                    "integrationIdentifier": recipient,
                    "sourceConversationId": sender,
                    "payload": event,
                }),
            ));
        }
        _ => {}
    }
    if event["message"].is_null() {
        tracing::info!(
            event_name = name,
            "Dropping Zalo webhook event without a message"
        );
        return Ok(None);
    }
    let identifier = if name.contains("user_send") {
        recipient
    } else {
        sender
    };
    let name = if name.starts_with("oa_send") {
        "incomingMessageEcho"
    } else {
        "incomingMessage"
    };
    Ok(job(
        name,
        json!({ "integrationType": "zalo", "integrationIdentifier": identifier, "payload": event }),
    ))
}

/// The request facts the route needs.
pub struct WebhookRequest<'a> {
    pub method: &'a str,
    pub signature: Option<&'a str>,
    /// `x-domain` (cloud only).
    pub domain: Option<&'a str>,
    pub body: &'a str,
}

/// `POST /integrations/zalo/webhook`: the platform `zalo` credential, then
/// `webhookHandler`. Every handler error answers 400 with its message.
pub async fn webhook(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    encryption: Option<&Encryption>,
    is_cloud: bool,
    broker_host: &str,
    request: WebhookRequest<'_>,
) -> WebhookReply {
    let not_configured = || json_reply(404, "Integration is not configured");
    // ponytail: as `platform_webhook`, only the broker host speaks for the
    // platform in cloud (no tenant custom domain, no `/c/{id}` route).
    if is_cloud && request.domain.unwrap_or("") != broker_host {
        return not_configured();
    }
    let Some(encryption) = encryption else {
        return not_configured();
    };
    let Some(config) = find_platform_config(db, encryption, "zalo").await else {
        tracing::debug!("Integration zalo is not configured");
        return not_configured();
    };
    let fail = |message: String| {
        let message = format!("Webhook processing failed: {message}");
        tracing::error!(
            err = message,
            integration_type = "zalo",
            "Integration handleRequest failed"
        );
        json_reply(400, &message)
    };
    if request.method != "POST" {
        return fail(format!("Unsupported HTTP method: {}", request.method));
    }
    let text = |key: &str| config.get(key).and_then(Value::as_str);
    let routed = route_event(
        text("clientId"),
        text("verifyToken"),
        request.signature,
        request.body,
    );
    let result = match routed {
        Ok(Some(job)) => jobs
            .add(
                QueueName::Integration,
                job.name,
                job.data,
                JobOptions::default(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Ok(None) => Ok(()),
        Err(message) => Err(message),
    };
    match result {
        Ok(()) => WebhookReply {
            status: 200,
            content_type: "text/plain;charset=UTF-8",
            body: "ok".into(),
        },
        Err(message) => fail(format!("Failed to process webhook event: {message}")),
    }
}

/// `GET /api/zalo-verifier/{verifier}` (and the `/zalo_verifier{v}.html`
/// rewrite): the site-verification page.
pub fn verifier_page(verifier: &str) -> WebhookReply {
    if verifier.is_empty() {
        return WebhookReply {
            status: 400,
            content_type: "text/plain;charset=UTF-8",
            body: "Verifier parameter is required".into(),
        };
    }
    // The original interpolates the raw value (reflected XSS); it is
    // HTML-escaped here (DECISIONS Q.I2).
    let escaped = verifier
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    WebhookReply {
        status: 200,
        content_type: "text/html",
        body: format!(
            "\n<!DOCTYPE html>\n<html lang=\"en\">\n\n<head>\n    <meta property=\"zalo-platform-site-verification\" content=\"{escaped}\" />\n</head>\n\n<body>\nThere Is No Limit To What You Can Accomplish Using Zalo!\n</body>\n\n</html>\n    "
        ),
    }
}

#[cfg(test)]
#[path = "zalo_tests.rs"]
mod tests;
