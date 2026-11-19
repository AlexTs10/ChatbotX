//! `integrations/telegram` (Q.I2): the Bot API client (`apis/bot.ts`,
//! `lib/http-client.ts`), `lib/error-mapper.ts`, `receiveMessage`
//! (`incoming-message.ts`), `sendMessage` / `sendFlowStep`
//! (`outgoing-message/*`), `sendTyping` and the webhook router
//! (`handlers/webhook.ts`, `app/integrations/[...integration]/webhook.ts`
//! `handleTelegramWebhook`).

use std::sync::Arc;

use chatbotx_core::flow_config::channel_rules::sends_as_native_copy_button;
use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{BoxFuture, Enqueuer, JobOptions, QueueName};
use serde_json::{Value, json};

use super::whatsapp_media::{WhatsappMedia, copy_to_storage, media_client};
use super::whatsapp_webhook::{WebhookReply, json_reply};
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingAttachment,
    IncomingContact, IncomingMessage, Received, SendFailure, SendFlowStep, SendMessage, SendResult,
    SendTyping,
};

/// `API_URL`.
pub const API_URL: &str = "https://api.telegram.org";
/// `MAX_INLINE_BUTTONS_PER_ROW`.
pub const MAX_INLINE_BUTTONS_PER_ROW: usize = 3;
/// `CHANNEL_AUDIENCE_EVENT_JOB_NAME`.
pub const CHANNEL_AUDIENCE_EVENT: &str = "channelAudienceEvent";
/// `X-Telegram-Bot-Api-Secret-Token`.
pub const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";
/// ky retry: 3 retries on these statuses.
const RETRY_STATUSES: [u16; 7] = [408, 413, 429, 500, 502, 503, 504];

pub struct Telegram {
    client: reqwest::Client,
    base_url: String,
    /// `ctx.uploader` + `storageUrl`; `None` keeps the Telegram file id.
    media: Option<Arc<WhatsappMedia>>,
    media_client: reqwest::Client,
}

impl Telegram {
    pub fn new() -> Telegram {
        Telegram::with_base_url(API_URL)
    }

    /// `getTelegramChatMemberStatus` + `isTelegramMemberStatus`.
    pub async fn chat_member_status(
        &self,
        token: &str,
        chat_id: &str,
        user_id: &str,
    ) -> Result<bool, ApiError> {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("chat_id", chat_id)
            .append_pair("user_id", user_id)
            .finish();
        let member = call(
            &self.client,
            &self.base_url,
            token,
            &format!("getChatMember?{query}"),
            None,
        )
        .await?;
        Ok(is_member_status(
            member["status"].as_str().unwrap_or(""),
            member["is_member"].as_bool(),
        ))
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Telegram {
        Telegram {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            media: None,
            media_client: media_client(),
        }
    }

    /// Copies received files to the file store.
    pub fn with_media(mut self, media: WhatsappMedia) -> Telegram {
        self.media = Some(Arc::new(media));
        self
    }
}

impl Default for Telegram {
    fn default() -> Telegram {
        Telegram::new()
    }
}

// ---- errors (`exception.ts`, `lib/error-mapper.ts`) ----------------------

/// `TelegramAPIException` fields (`parseOriginError`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub http_status: u16,
    pub code: Option<String>,
    pub message: String,
}

const AUTH_FAILED_PATTERNS: &[&str] = &[
    "auth_key_unregistered",
    "auth_key_invalid",
    "auth_key_perm_empty",
    "auth_key_duplicated",
    "session_revoked",
    "session_expired",
    "api_id_invalid",
];
const USER_BLOCKED_PATTERNS: &[&str] = &[
    "user_is_blocked",
    "you_blocked_user",
    "user_privacy_restricted",
    "user_not_participant",
    "chat_write_forbidden",
    "bot was blocked by the user",
    "bot can't initiate conversation with a user",
    "bot can't send messages to bots",
    "user is restricted",
];
const INVALID_RECIPIENT_PATTERNS: &[&str] = &[
    "user_deactivated",
    "phone_number_unoccupied",
    "user is deactivated",
    "chat not found",
    "user not found",
    "the group chat was deleted",
    "group chat was upgraded to a supergroup",
    "bot was kicked",
    "chat_id is empty",
    "peer_id_invalid",
];
const PERMISSION_DENIED_PATTERNS: &[&str] = &[
    "chat_admin_required",
    "not enough rights",
    "method is available only for supergroups",
    "bot is not a member",
];
const RATE_LIMITED_PATTERNS: &[&str] = &[
    "flood_wait",
    "flood_premium_wait",
    "slowmode_wait",
    "2fa_confirm_wait",
    "too many requests",
    "retry after",
];
const NETWORK_ERROR_PATTERNS: &[&str] = &[
    "file_migrate",
    "phone_migrate",
    "network_migrate",
    "user_migrate",
    "etimedout",
    "econnreset",
    "webhook error",
];
const PAYLOAD_INVALID_PATTERNS: &[&str] = &[
    "firstname_invalid",
    "lastname_invalid",
    "phone_number_invalid",
    "phone_number_occupied",
    "phone_code_hash_empty",
    "phone_code_empty",
    "phone_code_expired",
    "users_too_few",
    "users_too_much",
    "type_constructor_invalid",
    "file_part_invalid",
    "file_parts_invalid",
    "file_part",
    "md5_checksum_invalid",
    "photo_invalid_dimensions",
    "field_name_invalid",
    "field_name_empty",
    "message to delete not found",
    "message can't be deleted",
    "message is too long",
    "wrong file identifier",
    "failed to get http url content",
    "entity too large",
    "button_data_invalid",
    "query is too old",
    "bad webhook",
    "file is too big",
    "image_process_failed",
    "sticker_png_dimensions",
    "terminated by other getupdates",
    "can't use getupdates method",
];

/// `categorize(httpStatus, description)`.
pub fn categorize(http_status: u16, description: &str) -> Category {
    let any = |patterns: &[&str]| patterns.iter().any(|p| description.contains(p));
    if any(AUTH_FAILED_PATTERNS) {
        return Category::AuthFailed;
    }
    if any(USER_BLOCKED_PATTERNS) {
        return Category::UserBlocked;
    }
    if any(INVALID_RECIPIENT_PATTERNS) {
        return Category::InvalidRecipient;
    }
    if any(PERMISSION_DENIED_PATTERNS) {
        return Category::PermissionDenied;
    }
    if any(RATE_LIMITED_PATTERNS) {
        return Category::RateLimited;
    }
    if any(NETWORK_ERROR_PATTERNS) {
        return Category::NetworkError;
    }
    if any(PAYLOAD_INVALID_PATTERNS) {
        return Category::PayloadInvalid;
    }
    match http_status {
        303 => Category::NetworkError,
        401 => Category::AuthFailed,
        403 => Category::UserBlocked,
        420 | 429 => Category::RateLimited,
        400 | 404 | 405 | 406 | 409 => Category::PayloadInvalid,
        s if s >= 500 => Category::NetworkError,
        _ => Category::Unknown,
    }
}

/// `mapToChannelError`.
pub fn map_to_channel_error(error: &ApiError) -> ChannelError {
    let category = categorize(error.http_status, &error.message.to_lowercase());
    let mut out = ChannelError::new(error.message.clone(), category);
    out.code = Some(error.code.clone().unwrap_or_else(|| "UNKNOWN_ERROR".into()));
    out.http_status_code = error.http_status;
    out
}

/// `isRevokedTokenError`: Telegram answers a revoked bot token with 401.
pub fn is_revoked_token_error(error: &ApiError) -> bool {
    error.http_status == 401
}

/// `describe(auth)`: the bot id before the `:` of the token.
pub fn bot_id_of_token(token: &str) -> Result<&str, &'static str> {
    match token.split_once(':') {
        Some((id, _)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => Ok(id),
        _ => Err("Telegram auth has no bot identity"),
    }
}

/// One Bot API call (`createTelegramClient(token).post/get` in `rescue`).
async fn call(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    method: &str,
    body: Option<&Value>,
) -> Result<Value, ApiError> {
    let url = format!("{base}/bot{token}/{method}");
    let mut attempt = 0;
    loop {
        let request = match body {
            Some(body) => client
                .post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string()),
            None => client.get(&url),
        };
        let response = request.send().await;
        let (status, text) = match response {
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
                tracing::error!(err = %error, "Telegram API call failed: {method}");
                return Err(ApiError {
                    http_status: 400,
                    code: None,
                    message: error.to_string(),
                });
            }
        };
        if !(200..300).contains(&status) {
            if RETRY_STATUSES.contains(&status) && attempt < 3 {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
                continue;
            }
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            tracing::error!(status, "Telegram API call failed: {method}");
            return Err(ApiError {
                http_status: status,
                code: parsed["error_code"].as_i64().map(|c| c.to_string()),
                message: parsed["description"].as_str().unwrap_or(method).to_owned(),
            });
        }
        let parsed: Value = serde_json::from_str(&text).map_err(|e| ApiError {
            http_status: 400,
            code: None,
            message: e.to_string(),
        })?;
        return Ok(parsed["result"].clone());
    }
}

fn token_of(ctx: &ChannelCtx) -> ChannelResult<String> {
    ctx.auth["secretText"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ChannelError::new(
                "Telegram integration auth is incomplete",
                Category::AuthFailed,
            )
            .with_code("integration_auth_missing")
            .into()
        })
}

// ---- buttons (`send-button.ts`) -------------------------------------------

/// `buildInlineButton`: a flow step button.
pub fn build_inline_button(flow_id: &str, button: &Value) -> Value {
    let label = button["label"].clone();
    if button["buttonType"] == "openWebsite" {
        return json!({ "text": label, "url": button["beforeStep"]["url"] });
    }
    if sends_as_native_copy_button(Some("telegram"), button) {
        return json!({ "text": label, "copy_text": { "text": button["beforeStep"]["text"] } });
    }
    let payload = ButtonPayload {
        flow_id: flow_id.to_owned(),
        button_id: button["id"].as_str().map(str::to_owned),
        ..ButtonPayload::default()
    };
    json!({ "text": label, "callback_data": encode_button_payload(&payload) })
}

/// `buildCanonicalInlineButton`: a stored quick reply (`MessageButtonTemplate`).
pub fn build_canonical_inline_button(button: &Value) -> Value {
    match button["buttonType"].as_str() {
        Some("url") => json!({ "text": button["label"], "url": button["url"] }),
        Some("copyText") => {
            json!({ "text": button["label"], "copy_text": { "text": button["copyText"] } })
        }
        _ => json!({ "text": button["label"], "callback_data": button["postback"] }),
    }
}

/// `buildInlineKeyboardFromButtons`: up to 3 on a row, else one a row.
pub fn inline_keyboard(buttons: Vec<Value>) -> Value {
    let per_row = if buttons.len() > MAX_INLINE_BUTTONS_PER_ROW {
        1
    } else {
        MAX_INLINE_BUTTONS_PER_ROW
    };
    let rows: Vec<Value> = buttons
        .chunks(per_row)
        .map(|c| Value::Array(c.to_vec()))
        .collect();
    json!({ "inline_keyboard": rows })
}

fn quick_replies(data: Option<&Value>) -> Vec<Value> {
    data.and_then(Value::as_array)
        .map(|q| q.iter().map(build_canonical_inline_button).collect())
        .unwrap_or_default()
}

/// The step's buttons, then the node quick replies.
fn step_buttons(flow_id: &str, step: &Value, quick: Option<&Value>) -> Vec<Value> {
    let mut buttons: Vec<Value> = step["buttons"]
        .as_array()
        .map(|b| b.iter().map(|b| build_inline_button(flow_id, b)).collect())
        .unwrap_or_default();
    buttons.extend(quick_replies(quick));
    buttons
}

fn with_keyboard(mut payload: Value, buttons: Vec<Value>) -> Value {
    if !buttons.is_empty() {
        payload["reply_markup"] = inline_keyboard(buttons);
    }
    payload
}

/// One Bot API request (method, JSON body).
pub type Request = (&'static str, Value);

/// `sendFlowStep`'s `convertFlowStep*`: the requests of one step.
pub fn convert_flow_step(
    chat_id: &str,
    flow_id: &str,
    step: &Value,
    quick: Option<&Value>,
) -> Result<Vec<Request>, String> {
    let kind = step["stepType"].as_str().unwrap_or("");
    if chat_id.is_empty() && kind != "sendText" {
        return Ok(Vec::new());
    }
    let media = |method: &'static str, field: &str| {
        let payload = json!({ "chat_id": chat_id, field: step["url"] });
        vec![(
            method,
            with_keyboard(payload, step_buttons(flow_id, step, quick)),
        )]
    };
    Ok(match kind {
        "sendText" => vec![(
            "sendMessage",
            with_keyboard(
                json!({ "chat_id": chat_id, "text": step["text"] }),
                step_buttons(flow_id, step, quick),
            ),
        )],
        "sendImage" => media("sendPhoto", "photo"),
        "sendVideo" => media("sendVideo", "video"),
        "sendAudio" => media("sendAudio", "audio"),
        "sendFile" => media("sendDocument", "document"),
        // Telegram's sendMediaGroup takes no reply_markup.
        "sendMultipleImages" => vec![(
            "sendMediaGroup",
            json!({ "chat_id": chat_id, "media": step["images"].as_array().map(|images| images
                .iter()
                .map(|i| json!({ "type": "photo", "media": i["url"] }))
                .collect::<Vec<_>>()).unwrap_or_default() }),
        )],
        "sendGif" => {
            if step.get("url").is_none() {
                Vec::new()
            } else {
                vec![(
                    "sendDocument",
                    with_keyboard(
                        json!({ "chat_id": chat_id, "document": step["url"] }),
                        quick_replies(quick),
                    ),
                )]
            }
        }
        "sendQuickReply" => {
            let buttons: Vec<Value> = step["buttons"]
                .as_array()
                .map(|b| b.iter().map(|b| build_inline_button(flow_id, b)).collect())
                .unwrap_or_default();
            vec![(
                "sendMessage",
                json!({ "chat_id": chat_id, "text": step["message"], "reply_markup": inline_keyboard(buttons) }),
            )]
        }
        "sendCarousel" => carousel(chat_id, flow_id, step),
        other => return Err(format!("Unsupported Telegram flow step: {other}")),
    })
}

/// `convertFlowStepCarousel`: one photo (or text) message per card.
fn carousel(chat_id: &str, flow_id: &str, step: &Value) -> Vec<Request> {
    let mut out = Vec::new();
    for card in step["cards"].as_array().into_iter().flatten() {
        let text = [card["title"].as_str(), card["subtitle"].as_str()]
            .into_iter()
            .flatten()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let buttons: Vec<Value> = card["buttons"]
            .as_array()
            .map(|b| b.iter().map(|b| build_inline_button(flow_id, b)).collect())
            .unwrap_or_default();
        let markup = (!buttons.is_empty()).then(|| inline_keyboard(buttons));
        let mut payload = match card["image"]["url"].as_str().filter(|u| !u.is_empty()) {
            Some(url) => {
                let mut p = json!({ "chat_id": chat_id, "photo": url, "parse_mode": "HTML" });
                if !text.is_empty() {
                    p["caption"] = json!(text);
                }
                p
            }
            None if !text.is_empty() => {
                json!({ "chat_id": chat_id, "text": text, "parse_mode": "HTML" })
            }
            None => continue,
        };
        if let Some(markup) = markup {
            payload["reply_markup"] = markup;
        }
        let method = if payload.get("photo").is_some() {
            "sendPhoto"
        } else {
            "sendMessage"
        };
        out.push((method, payload));
    }
    out
}

/// `sendMessage`: text (+ quick replies), the images in one media group (or
/// one photo), every other attachment as its own message.
pub fn convert_message(chat_id: &str, message: &Value, quick: Option<&Value>) -> Vec<Request> {
    let mut out = Vec::new();
    if message["contentType"] != "text" {
        out.push((
            "sendMessage",
            json!({ "chat_id": chat_id, "text": message["text"].as_str().unwrap_or("not handled yet") }),
        ));
        return out;
    }
    if let Some(text) = message["text"].as_str().filter(|t| !t.is_empty()) {
        out.push((
            "sendMessage",
            with_keyboard(
                json!({ "chat_id": chat_id, "text": text }),
                quick_replies(quick),
            ),
        ));
    }
    let attachments: Vec<&Value> = message["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let (images, others): (Vec<&Value>, Vec<&Value>) = attachments
        .into_iter()
        .partition(|a| a["fileType"] == "image");
    match images.as_slice() {
        [] => {}
        [image] => out.push(("sendPhoto", json!({ "chat_id": chat_id, "photo": image["url"] }))),
        images => out.push((
            "sendMediaGroup",
            json!({ "chat_id": chat_id, "media": images.iter().map(|a| json!({ "type": "photo", "media": a["url"] })).collect::<Vec<_>>() }),
        )),
    }
    for attachment in others {
        let (method, field) = match attachment["fileType"].as_str() {
            Some("video") => ("sendVideo", "video"),
            Some("audio") => ("sendAudio", "audio"),
            _ => ("sendDocument", "document"),
        };
        out.push((
            method,
            json!({ "chat_id": chat_id, field: attachment["url"] }),
        ));
    }
    out
}

/// Runs the requests in order; a media group answers several message ids.
async fn send_all(
    client: reqwest::Client,
    base: String,
    token: ChannelResult<String>,
    requests: Result<Vec<Request>, String>,
) -> ChannelResult<SendResult> {
    let token = token?;
    let requests = requests.map_err(|m| {
        SendFailure::Channel(map_to_channel_error(&ApiError {
            http_status: 400,
            code: None,
            message: m,
        }))
    })?;
    let mut ids = Vec::new();
    for (method, body) in requests {
        let result = call(&client, &base, &token, method, Some(&body))
            .await
            .map_err(|e| {
                tracing::error!(error = %e.message, "An error occurred while sending the message");
                SendFailure::Channel(map_to_channel_error(&e))
            })?;
        match result.as_array() {
            Some(messages) => ids.extend(
                messages
                    .iter()
                    .filter_map(|m| m["message_id"].as_i64())
                    .map(|id| id.to_string()),
            ),
            None => {
                if let Some(id) = result["message_id"].as_i64() {
                    ids.push(id.to_string());
                }
            }
        }
    }
    Ok(SendResult {
        sent_count: ids.len() as u32,
        message_ids: ids,
    })
}

// ---- receive (`incoming-message.ts`) --------------------------------------

/// A file of a message to copy (`downloadAndUploadFile` arguments).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaRequest {
    pub file_id: String,
    pub mime_type: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_type: String,
    pub is_photo_message: bool,
}

fn guess_file_type(mime: &str) -> String {
    chatbotx_files::guess_file_type_from_mime_type(mime)
        .as_str()
        .to_owned()
}

fn file(
    id: &Value,
    mime: String,
    dims: Option<(&Value, &Value)>,
    kind: Option<&str>,
) -> MediaRequest {
    MediaRequest {
        file_id: id.as_str().unwrap_or_default().to_owned(),
        file_type: kind.map_or_else(|| guess_file_type(&mime), str::to_owned),
        mime_type: mime,
        width: dims.and_then(|d| d.0.as_i64()),
        height: dims.and_then(|d| d.1.as_i64()),
        is_photo_message: false,
    }
}

/// `getMessageAttachments` (the files, before any download).
pub fn media_requests(message: &Value) -> Vec<MediaRequest> {
    let mime = |v: &Value, default: &str| v["mime_type"].as_str().unwrap_or(default).to_owned();
    let mut out = Vec::new();
    if let Some(photos) = message["photo"].as_array() {
        // `getLargestPhoto`: the first one unless a later one is larger.
        let largest = photos
            .iter()
            .fold(None::<&Value>, |largest, photo| match largest {
                None => Some(photo),
                Some(l) => match (photo["file_size"].as_f64(), l["file_size"].as_f64()) {
                    (Some(p), Some(lv)) if p > lv => Some(photo),
                    _ => Some(l),
                },
            });
        if let Some(p) = largest {
            let mut request = file(
                &p["file_id"],
                "image/jpeg".into(),
                Some((&p["width"], &p["height"])),
                None,
            );
            request.is_photo_message = true;
            out.push(request);
        }
    }
    // `animation` comes with `document` set to the same file.
    let animation = &message["animation"];
    if animation.is_object() {
        out.push(file(
            &animation["file_id"],
            mime(animation, "video/mp4"),
            None,
            Some("gif"),
        ));
    } else if message["document"].is_object() {
        let d = &message["document"];
        out.push(file(
            &d["file_id"],
            mime(d, "application/octet-stream"),
            None,
            None,
        ));
    }
    let sticker = &message["sticker"];
    if sticker.is_object() {
        if sticker["is_video"] == true {
            out.push(file(
                &sticker["file_id"],
                "video/webm".into(),
                None,
                Some("gif"),
            ));
        } else if sticker["is_animated"] == true {
            let thumb = &sticker["thumbnail"];
            if thumb.is_object() {
                out.push(file(
                    &thumb["file_id"],
                    "image/webp".into(),
                    Some((&thumb["width"], &thumb["height"])),
                    None,
                ));
            }
        } else {
            out.push(file(
                &sticker["file_id"],
                "image/webp".into(),
                Some((&sticker["width"], &sticker["height"])),
                None,
            ));
        }
    }
    for (key, default) in [
        ("audio", "audio/mpeg"),
        ("video", "video/mp4"),
        ("voice", "audio/ogg"),
    ] {
        let v = &message[key];
        if v.is_object() {
            out.push(file(&v["file_id"], mime(v, default), None, None));
        }
    }
    out
}

/// `receiveMessage` without the downloads: the message, contact and ref of
/// an update; the attachments are the files still to copy.
pub fn parse_update(update: &Value) -> Result<(Received, Vec<MediaRequest>), String> {
    if let Some(query) = update.get("callback_query").filter(|q| q.is_object()) {
        let from = &query["from"];
        let chat_id = query["message"]["chat"]["id"]
            .as_i64()
            .or_else(|| from["id"].as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default();
        let payload = query["data"].as_str().unwrap_or("").to_owned();
        let mut message = IncomingMessage::incoming(Some(id_text(&query["id"])));
        message.text = Some(payload.clone());
        return Ok((
            Received {
                message: Some(message),
                contact: contact(chat_id, from),
                postback_action: Some(payload),
                ..Received::default()
            },
            Vec::new(),
        ));
    }
    let Some(message) = update.get("message").filter(|m| m.is_object()) else {
        return Err("No message or callback_query in update".into());
    };
    let from = &message["from"];
    let contact_id = from["id"]
        .as_i64()
        .or_else(|| message["chat"]["id"].as_i64())
        .map(|id| id.to_string())
        .unwrap_or_default();
    let mut incoming = IncomingMessage::incoming(Some(id_text(&message["message_id"])));
    incoming.text = message["text"]
        .as_str()
        .or_else(|| message["caption"].as_str())
        .map(str::to_owned);
    // `/start <ref>`.
    let r#ref = message["text"]
        .as_str()
        .filter(|t| t.starts_with("/start"))
        .and_then(|t| t.split(' ').nth(1))
        .map(str::to_owned);
    Ok((
        Received {
            message: Some(incoming),
            contact: contact(contact_id, from),
            r#ref,
            ..Received::default()
        },
        media_requests(message),
    ))
}

fn id_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn contact(source_id: String, from: &Value) -> IncomingContact {
    let text = |k: &str| from[k].as_str().map(str::to_owned);
    IncomingContact {
        source_id,
        first_name: text("first_name"),
        last_name: text("last_name"),
        locale: text("language_code"),
        ..IncomingContact::default()
    }
}

fn to_attachment(request: &MediaRequest) -> IncomingAttachment {
    IncomingAttachment {
        source_id: Some(chatbotx_core::utils::id::create_id()),
        file_type: request.file_type.clone(),
        mime_type: request.mime_type.clone(),
        origin_path: request.file_id.clone(),
        width: request.width,
        height: request.height,
        is_photo_message: request.is_photo_message,
        ..IncomingAttachment::default()
    }
}

impl Channel for Telegram {
    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let (client, base, token) = (self.client.clone(), self.base_url.clone(), token_of(ctx));
        let (media, media_http) = (self.media.clone(), self.media_client.clone());
        let loc = chatbotx_files::ChannelMediaLocation {
            channel: ctx.channel.clone(),
            workspace_id: ctx.workspace_id.to_string(),
            integration_id: ctx.integration_id.to_string(),
        };
        Box::pin(async move {
            let (mut received, files) = parse_update(&payload).map_err(SendFailure::Other)?;
            let mut attachments = Vec::new();
            for request in &files {
                let mut attachment = to_attachment(request);
                // ponytail: without a file store the Telegram file id is kept.
                if let Some(media) = &media {
                    let token = token.clone()?;
                    let result = call(
                        &client,
                        &base,
                        &token,
                        &format!("getFile?file_id={}", request.file_id),
                        None,
                    )
                    .await;
                    let path = match result {
                        Ok(file) => file["file_path"].as_str().map(str::to_owned),
                        Err(e) if e.http_status == 404 => None,
                        Err(e) => return Err(SendFailure::Channel(map_to_channel_error(&e))),
                    };
                    let Some(path) = path else { continue };
                    let url = format!("{base}/file/bot{token}/{path}");
                    copy_to_storage(media, &media_http, &url, None, Some(&request.mime_type), &loc, &mut attachment)
                        .await
                        .map_err(|error| {
                            tracing::error!(%error, file_id = request.file_id, "downloadAndUploadFile error");
                            SendFailure::Other(format!("Failed to download attachment: {error}"))
                        })?;
                }
                attachments.push(attachment);
            }
            if let Some(message) = received.message.as_mut() {
                message.attachments = attachments;
            }
            Ok(Some(received))
        })
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let requests = Ok(convert_message(
            &data.contact.contact_inbox.source_id,
            &data.message,
            data.quick_replies.as_ref(),
        ));
        Box::pin(send_all(
            self.client.clone(),
            self.base_url.clone(),
            token_of(ctx),
            requests,
        ))
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let requests = convert_flow_step(
            &data.contact.contact_inbox.source_id,
            &data.flow_id,
            &data.step,
            data.quick_replies.as_ref(),
        );
        Box::pin(send_all(
            self.client.clone(),
            self.base_url.clone(),
            token_of(ctx),
            requests,
        ))
    }

    fn send_typing(
        &self,
        ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let (client, base, token) = (self.client.clone(), self.base_url.clone(), token_of(ctx));
        Box::pin(async move {
            let chat_id = data.contact.contact_inbox.source_id;
            if chat_id.is_empty() {
                return Err(SendFailure::other("Missing chat ID in conversation"));
            }
            if data.typing {
                let body = json!({ "chat_id": chat_id, "action": "typing" });
                call(&client, &base, &token?, "sendChatAction", Some(&body))
                    .await
                    .map_err(|e| SendFailure::Channel(map_to_channel_error(&e)))?;
            }
            Ok(())
        })
    }
}

// ---- webhook (`handlers/webhook.ts`) --------------------------------------

/// `isTelegramMemberStatus`.
pub fn is_member_status(status: &str, is_member: Option<bool>) -> bool {
    matches!(status, "creator" | "administrator" | "member")
        || (status == "restricted" && is_member == Some(true))
}

fn audience_user(user: &Value) -> Value {
    let mut out = json!({ "sourceId": id_text(&user["id"]), "firstName": user["first_name"] });
    for (from, to) in [("last_name", "lastName"), ("username", "username")] {
        if let Some(v) = user[from].as_str().filter(|v| !v.is_empty()) {
            out[to] = json!(v);
        }
    }
    out
}

fn iso(unix_seconds: &Value) -> Value {
    let ms = unix_seconds.as_i64().unwrap_or(0) * 1000;
    json!(chatbotx_core::js::to_iso_string(ms).unwrap_or_default())
}

fn is_group(message: &Value) -> bool {
    matches!(
        message["chat"]["type"].as_str(),
        Some("group" | "supergroup")
    )
}

fn member(m: &Value) -> bool {
    is_member_status(m["status"].as_str().unwrap_or(""), m["is_member"].as_bool())
}

/// `parseTelegramAudienceEvent`: the event of a non-DM update.
pub fn parse_audience_event(update: &Value) -> Option<Value> {
    if let Some(m) = update.get("my_chat_member").filter(|m| m.is_object()) {
        let chat = &m["chat"];
        let mut chat_out = json!({ "id": id_text(&chat["id"]), "type": chat["type"] });
        if let Some(title) = chat["title"].as_str().filter(|t| !t.is_empty()) {
            chat_out["title"] = json!(title);
        }
        return Some(json!({
            "kind": "botChatStatus",
            "chat": chat_out,
            "status": m["new_chat_member"]["status"],
            "oldStatus": m["old_chat_member"]["status"],
            "user": audience_user(&m["from"]),
            "occurredAt": iso(&m["date"]),
        }));
    }
    if let Some(m) = update.get("chat_member").filter(|m| m.is_object()) {
        let user = &m["new_chat_member"]["user"];
        if user["is_bot"] == true {
            return None;
        }
        let (was, is) = (member(&m["old_chat_member"]), member(&m["new_chat_member"]));
        if was == is {
            return None;
        }
        return Some(json!({
            "kind": if is { "groupJoin" } else { "groupLeave" },
            "user": audience_user(user),
            "chatId": id_text(&m["chat"]["id"]),
            "occurredAt": iso(&m["date"]),
        }));
    }
    let message = update.get("message").filter(|m| m.is_object())?;
    if !is_group(message) {
        return None;
    }
    let from = message.get("from").filter(|f| f.is_object())?;
    if from["is_bot"] == true {
        return None;
    }
    let text = message["text"]
        .as_str()
        .or_else(|| message["caption"].as_str())
        .unwrap_or("");
    Some(json!({
        "kind": "groupMessage",
        "user": audience_user(from),
        "chatId": id_text(&message["chat"]["id"]),
        "messageId": id_text(&message["message_id"]),
        "textLength": text.encode_utf16().count(),
        "occurredAt": iso(&message["date"]),
    }))
}

/// `webhookHandler`: the job to queue for one update (`None` = only "ok").
pub fn route_update(
    bot_id: &str,
    stored_secret: Option<&str>,
    presented_secret: Option<&str>,
    body: &str,
) -> Result<Option<(&'static str, Value)>, String> {
    let stored_secret = stored_secret.filter(|s| !s.is_empty());
    let verified = match (stored_secret, presented_secret) {
        (Some(stored), Some(presented)) if !presented.is_empty() => {
            chatbotx_core::utils::crypto::timing_safe_string_equal(presented, stored)
        }
        _ => false,
    };
    if stored_secret.is_some() && !verified {
        return Err("Webhook error: Invalid webhook secret token".into());
    }
    if body.is_empty() {
        return Err("Webhook error: Empty webhook payload".into());
    }
    let update: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if !update.is_object() || !update["update_id"].is_number() {
        return Err("Invalid Telegram update".into());
    }
    let incoming = |update: Value| {
        json!({ "type": "incomingMessage", "data": {
            "integrationType": "telegram",
            "integrationIdentifier": bot_id,
            "payload": update,
        }})
    };
    if let Some(query) = update.get("callback_query").filter(|q| q.is_object()) {
        if query["message"]["chat"]["id"]
            .as_i64()
            .is_none_or(|id| id == 0)
        {
            return Ok(None);
        }
        return Ok(Some(("incomingMessage", incoming(update))));
    }
    if let Some(event) = parse_audience_event(&update) {
        // Group activity awards leaderboard points: only a verified bot.
        if !verified {
            return Ok(None);
        }
        return Ok(Some((
            CHANNEL_AUDIENCE_EVENT,
            json!({ "type": CHANNEL_AUDIENCE_EVENT, "data": {
                "integrationType": "telegram",
                "integrationIdentifier": bot_id,
                "event": event,
            }}),
        )));
    }
    match update.get("message").filter(|m| m.is_object()) {
        Some(message) if !is_group(message) => Ok(Some(("incomingMessage", incoming(update)))),
        _ => Ok(None),
    }
}

/// `handleTelegramWebhook` (`/integrations/telegram/webhook?botId=`).
pub async fn webhook(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    bot_id: Option<&str>,
    presented_secret: Option<&str>,
    body: &str,
) -> WebhookReply {
    let Some(bot_id) = bot_id.filter(|b| !b.is_empty()) else {
        return json_reply(400, "Missing botId query param");
    };
    let bot = bot_id.to_owned();
    let row = db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "botId", "auth" FROM "IntegrationTelegram" WHERE "botId" = ?1 LIMIT 1"#,
                    params![bot],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await;
    let (bot_id, auth) = match row {
        Ok(Some(row)) => row,
        Ok(None) => return json_reply(404, "Bot not found"),
        Err(e) => return json_reply(400, &e.to_string()),
    };
    let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
    let secret = auth["metadata"]["webhookSecretToken"].as_str();
    match route_update(&bot_id, secret, presented_secret, body) {
        Ok(None) => {}
        Ok(Some((name, data))) => {
            if let Err(e) = jobs
                .add(QueueName::Integration, name, data, JobOptions::default())
                .await
            {
                tracing::error!(err = %e, integration_type = "telegram", "Telegram handleRequest failed");
                return json_reply(400, &e.to_string());
            }
        }
        Err(message) => {
            tracing::error!(
                err = message,
                integration_type = "telegram",
                "Telegram handleRequest failed"
            );
            return json_reply(400, &message);
        }
    }
    WebhookReply {
        status: 200,
        content_type: "text/plain;charset=UTF-8",
        body: "ok".into(),
    }
}

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;
