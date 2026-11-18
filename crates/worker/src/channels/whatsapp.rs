//! `integrations/whatsapp`: the Cloud API channel (Q.I1a). The Graph client
//! (`client.ts`, `whatsapp-api-js` request shapes), `lib/error-mapper.ts`,
//! `sendMessage` / `sendFlowStep` (`handlers/message/outgoing-message`),
//! `receiveMessage` (`incomming-message.ts`, `incoming-reply.ts`),
//! `handleMessageStatus` and `sendTyping` (`handlers/conversation.ts`).

use chatbotx_core::flow_config::steps::get_button_link_url;
use chatbotx_core::flow_config::util::{
    ButtonPayload, append_code_to_magic_link, encode_button_payload, extract_metadata,
};
use chatbotx_core::utils::graph_error::{GraphErrorSource, format_graph_error_message};
use chatbotx_jobs::BoxFuture;
use std::sync::Arc;

use super::whatsapp_media::WhatsappMedia;
use super::whatsapp_steps as steps;
use serde_json::{Map, Value, json};

use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, IncomingAttachment,
    IncomingContact, IncomingMessage, MessageStatus, OutgoingContact, Received, SendFailure,
    SendFlowStep, SendMessage, SendResult, SendTyping,
};

/// `API_URL`.
pub const API_URL: &str = "https://graph.facebook.com";
/// `DEFAULT_API_VERSION`.
pub const DEFAULT_API_VERSION: &str = "v23.0";
/// `WHATSAPP_NATIVE_LOCATION_REQUEST`.
const NATIVE_LOCATION_REQUEST: &str = "whatsapp:native:location_request";

/// `messageLimits`.
pub(crate) mod limits {
    pub const BODY_TEXT: usize = 1024;
    pub const CAPTION: usize = 1024;
    pub const CAROUSEL_CARD_BODY: usize = 160;
    pub const FOOTER_TEXT: usize = 60;
    pub const TEXT: usize = 4096;
    pub const BUTTON_ID: usize = 256;
    pub const BUTTON_TITLE: usize = 20;
    pub const ROW_ID: usize = 200;
    pub const ROW_TITLE: usize = 24;
}
const MAX_BUTTONS: usize = 3;
const MAX_LIST_ROWS: usize = 10;
const DEFAULT_LIST_BUTTON_LABEL: &str = "Options";
const EMPTY_BODY_PLACEHOLDER: &str = ".";

pub struct Whatsapp {
    client: reqwest::Client,
    base_url: String,
    /// `ctx.uploader` + `storageUrl`; `None` keeps media links and ids as given.
    media: Option<Arc<WhatsappMedia>>,
    media_client: reqwest::Client,
}

impl Whatsapp {
    pub fn new() -> Whatsapp {
        Whatsapp::with_base_url(API_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Whatsapp {
        Whatsapp {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            media: None,
            media_client: super::whatsapp_media::media_client(),
        }
    }

    /// Copies received media to the file store and re-hosts outbound links.
    pub fn with_media(mut self, media: WhatsappMedia) -> Whatsapp {
        self.media = Some(Arc::new(media));
        self
    }
}

impl Default for Whatsapp {
    fn default() -> Whatsapp {
        Whatsapp::new()
    }
}

/// The `WhatsappAuthValue` fields a send needs.
#[derive(Debug, Clone)]
struct Auth {
    access_token: String,
    phone_number_id: String,
}

fn auth_of(ctx: &ChannelCtx) -> ChannelResult<Auth> {
    let token = ctx
        .auth
        .pointer("/tokens/accessToken")
        .and_then(Value::as_str);
    let phone = ctx
        .auth
        .pointer("/metadata/phoneNumber/id")
        .and_then(Value::as_str);
    match (token, phone) {
        (Some(token), Some(phone)) => Ok(Auth {
            access_token: token.to_owned(),
            phone_number_id: phone.to_owned(),
        }),
        _ => Err(ChannelError::new(
            "WhatsApp integration auth is incomplete",
            Category::AuthFailed,
        )
        .with_code("integration_auth_missing")
        .into()),
    }
}

// ---- error mapping (`lib/error-mapper.ts`) --------------------------------

/// `categorize(code, type)`.
pub fn categorize(code: Option<i64>, kind: Option<&str>) -> Category {
    const AUTH_FAILED: &[i64] = &[0, 190, 133_005, 133_008, 133_009];
    const THREAD_CONTROL_NOT_ESCALATION: i64 = 2_494_191;
    const PERMISSION_DENIED: &[i64] = &[
        3,
        10,
        131_005,
        368,
        130_497,
        131_031,
        131_042,
        131_057,
        131_064,
        133_000,
        133_006,
        133_010,
        133_015,
        THREAD_CONTROL_NOT_ESCALATION,
    ];
    const CALLING_INELIGIBLE: &[i64] = &[138_013, 138_014, 138_018];
    const THREAD_CONTROL_REJECTION: &[i64] = &[131_070];
    const RATE_LIMITED: &[i64] = &[4, 80_007, 130_429, 131_048, 131_056, 133_016];
    const QUOTA_EXCEEDED: &[i64] = &[131_047];
    const USER_BLOCKED: &[i64] = &[130_403, 131_026, 131_049, 131_050];
    const INVALID_RECIPIENT: &[i64] = &[33, 131_021, 130_472];
    const PAYLOAD_INVALID: &[i64] = &[
        100, 131_008, 131_009, 131_037, 131_052, 131_053, 131_055, 132_000, 132_001, 132_005,
        132_007, 132_012, 132_015, 132_016, 132_018, 131_062, 132_068, 132_069, 134_100, 134_101,
        134_102, 135_000,
    ];
    const NETWORK_ERROR: &[i64] = &[1, 2, 131_000, 131_016, 133_004];
    let oauth = kind == Some("OAuthException");
    let Some(code) = code else {
        return if oauth {
            Category::AuthFailed
        } else {
            Category::Unknown
        };
    };
    if CALLING_INELIGIBLE.contains(&code)
        || THREAD_CONTROL_REJECTION.contains(&code)
        || code == THREAD_CONTROL_NOT_ESCALATION
    {
        Category::PermissionDenied
    } else if AUTH_FAILED.contains(&code) || oauth {
        Category::AuthFailed
    } else if RATE_LIMITED.contains(&code) {
        Category::RateLimited
    } else if QUOTA_EXCEEDED.contains(&code) {
        Category::QuotaExceeded
    } else if USER_BLOCKED.contains(&code) {
        Category::UserBlocked
    } else if INVALID_RECIPIENT.contains(&code) {
        Category::InvalidRecipient
    } else if PERMISSION_DENIED.contains(&code) || (200..=299).contains(&code) {
        Category::PermissionDenied
    } else if NETWORK_ERROR.contains(&code) {
        Category::NetworkError
    } else if PAYLOAD_INVALID.contains(&code) {
        Category::PayloadInvalid
    } else {
        Category::Unknown
    }
}

/// `mapToChannelError({ httpStatus, errorBody })`: Meta's `{ error: {..} }`
/// body as a categorized [`ChannelError`].
pub fn map_graph_error(http_status: u16, body: &Value) -> ChannelError {
    let error = body.get("error").unwrap_or(&Value::Null);
    let text = |k: &str| error.get(k).and_then(Value::as_str).map(str::to_owned);
    let code = error.get("code").filter(|c| !c.is_null()).cloned();
    let numeric = code.as_ref().and_then(Value::as_i64);
    let kind = text("type");
    let category = categorize(numeric, kind.as_deref());
    let message = format_graph_error_message(&GraphErrorSource {
        code: code.clone(),
        sub_code: error.get("error_subcode").filter(|c| !c.is_null()).cloned(),
        message: text("message"),
        user_message: text("error_user_msg").or_else(|| text("error_user_title")),
    })
    .unwrap_or_else(|| "WhatsApp API call failed".into());
    let mut mapped = ChannelError::new(message, category);
    mapped.code = Some(match &code {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "-1".into(),
    });
    mapped.http_status_code = error
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or(http_status);
    mapped
}

// ---- outgoing shapes (`whatsapp-api-js/messages`) -------------------------

/// One Cloud API message: `type` and its payload. `raw` = posted with the
/// raw-path envelope (`recipient_type: individual`).
#[derive(Debug, Clone, PartialEq)]
pub struct WaMessage {
    pub kind: String,
    pub payload: Value,
    pub raw: bool,
}

impl WaMessage {
    pub(crate) fn lib(kind: &str, payload: Value) -> WaMessage {
        WaMessage {
            kind: kind.into(),
            payload,
            raw: false,
        }
    }

    pub(crate) fn raw(kind: &str, payload: Value) -> WaMessage {
        WaMessage {
            kind: kind.into(),
            payload,
            raw: true,
        }
    }
}

pub(crate) fn text_message(body: &str) -> WaMessage {
    WaMessage::lib("text", json!({ "body": body }))
}

pub(crate) fn media_message(kind: &str, link: &str) -> WaMessage {
    WaMessage::lib(kind, json!({ "link": link }))
}

/// `clampText`: trim, cut at `max` UTF-16 units, no lone high surrogate.
pub fn clamp_text(value: &str, max: usize) -> String {
    let mut out = String::new();
    let mut units = 0;
    for ch in value.trim().chars() {
        units += ch.len_utf16();
        if units > max {
            break;
        }
        out.push(ch);
    }
    out.trim().to_owned()
}

/// `splitText`: chunks of at most `max` units, cut at the last word break.
pub fn split_text(value: &str, max: usize) -> Vec<String> {
    let mut rest: Vec<char> = value.trim().chars().collect();
    let mut chunks = Vec::new();
    let len16 = |c: &[char]| c.iter().map(|ch| ch.len_utf16()).sum::<usize>();
    while len16(&rest) > max {
        let mut taken = 0;
        let mut cut_at = 0;
        for (i, ch) in rest.iter().enumerate() {
            if taken + ch.len_utf16() > max {
                break;
            }
            taken += ch.len_utf16();
            cut_at = i + 1;
        }
        let head = &rest[..cut_at];
        // `/\s+\S*$/` on the head: the start of its trailing whitespace run.
        let word_break = head.iter().rposition(|c| c.is_whitespace()).map(|end| {
            let mut start = end;
            while start > 0 && head[start - 1].is_whitespace() {
                start -= 1;
            }
            start
        });
        let cut = word_break.filter(|&b| b > 0).unwrap_or(cut_at);
        chunks.push(rest[..cut].iter().collect::<String>().trim().to_owned());
        rest = rest[cut..]
            .iter()
            .collect::<String>()
            .trim()
            .chars()
            .collect();
    }
    if !rest.is_empty() {
        chunks.push(rest.into_iter().collect());
    }
    chunks
}

pub(crate) fn interactive(
    kind: &str,
    action: Value,
    body: &str,
    header: Option<Value>,
) -> WaMessage {
    let mut payload = json!({
        "type": kind,
        "action": action,
        "body": { "text": clamp_text(body, limits::BODY_TEXT) },
    });
    if let Some(header) = header {
        payload["header"] = header;
    }
    WaMessage::lib("interactive", payload)
}

struct Reply {
    id: String,
    label: String,
}

/// `normalizeRawButton`.
pub(crate) fn button_payload(data: &SendFlowStep, button_id: &str, with_contact: bool) -> String {
    let metadata = data.metadata.as_ref();
    encode_button_payload(&ButtonPayload {
        flow_id: data.flow_id.clone(),
        flow_version_id: data.flow_version_id.clone(),
        button_id: Some(button_id.to_owned()),
        broadcast_id: extract_metadata("broadcastId", metadata),
        sequence_step_id: extract_metadata("sequenceStepId", metadata),
        contact_inbox_id: with_contact.then(|| data.contact.contact_inbox.id.to_string()),
        ..ButtonPayload::default()
    })
}

/// `getCanonicalReplyPayload`.
fn canonical_reply_payload(button: &Value) -> Option<String> {
    let postback = button.get("postback").and_then(Value::as_str);
    if button.get("buttonType").and_then(Value::as_str) != Some("url") {
        return postback.map(str::to_owned);
    }
    postback
        .or_else(|| button.get("url").and_then(Value::as_str))
        .map(str::to_owned)
}

pub(crate) fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `generateFooter`.
fn with_footer(mut message: WaMessage, footer: Option<&str>) -> WaMessage {
    if let Some(footer) = footer {
        message.payload["footer"] = json!({ "text": clamp_text(footer, limits::FOOTER_TEXT) });
    }
    message
}

/// `buildWhatsappButtonMessages`.
pub(crate) fn button_messages(
    data: &SendFlowStep,
    buttons: &[Value],
    quick: &[Value],
    body_text: &str,
    media: Option<WaMessage>,
    footer: Option<&str>,
) -> Vec<WaMessage> {
    let links: Vec<(Reply, String)> = buttons
        .iter()
        .filter_map(|b| {
            let url = get_button_link_url(b)?;
            Some((
                Reply {
                    id: button_payload(data, str_field(b, "id"), true),
                    label: str_field(b, "label").to_owned(),
                },
                url,
            ))
        })
        .collect();
    let candidates = buttons
        .iter()
        .filter(|b| get_button_link_url(b).is_none())
        .map(|b| Reply {
            id: button_payload(data, str_field(b, "id"), false),
            label: str_field(b, "label").to_owned(),
        })
        .chain(quick.iter().map(|q| Reply {
            id: canonical_reply_payload(q).unwrap_or_default(),
            label: str_field(q, "label").to_owned(),
        }));
    // `dedupeReplyButtons`.
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_labels = std::collections::HashSet::new();
    let replies: Vec<Reply> = candidates
        .filter(|r| {
            let id = clamp_text(&r.id, limits::BUTTON_ID.min(limits::ROW_ID));
            let label = clamp_text(&r.label, limits::BUTTON_TITLE.min(limits::ROW_TITLE));
            !label.is_empty() && seen_ids.insert(id) && seen_labels.insert(label)
        })
        .collect();
    if replies.is_empty() && links.is_empty() {
        return Vec::new();
    }
    let chunks = split_text(body_text, limits::BODY_TEXT);
    let body = chunks.last().cloned().unwrap_or_default();
    let mut out: Vec<WaMessage> = chunks[..chunks.len().saturating_sub(1)]
        .iter()
        .map(|c| text_message(c))
        .collect();
    let cta = |reply: &Reply, url: &str, body: &str| {
        interactive(
            "cta_url",
            json!({ "name": "cta_url", "parameters": {
                "display_text": clamp_text(&reply.label, limits::BUTTON_TITLE),
                "url": append_code_to_magic_link(url, &reply.id),
            }}),
            if body.trim().is_empty() {
                EMPTY_BODY_PLACEHOLDER
            } else {
                body.trim()
            },
            None,
        )
    };
    if !replies.is_empty() {
        if replies.len() <= MAX_BUTTONS {
            let header = media
                .as_ref()
                .map(|m| json!({ "type": m.kind, m.kind.clone(): m.payload }));
            out.push(with_footer(
                interactive(
                    "button",
                    json!({ "buttons": replies.iter().map(|r| json!({ "type": "reply", "reply": {
                    "id": clamp_text(&r.id, limits::BUTTON_ID),
                    "title": clamp_text(&r.label, limits::BUTTON_TITLE),
                }})).collect::<Vec<_>>() }),
                    &body,
                    header,
                ),
                footer,
            ));
        } else {
            out.extend(media);
            for chunk in replies.chunks(MAX_LIST_ROWS) {
                out.push(with_footer(interactive(
                    "list",
                    json!({ "button": DEFAULT_LIST_BUTTON_LABEL, "sections": [{ "rows": chunk.iter().map(|r| json!({
                        "id": clamp_text(&r.id, limits::ROW_ID),
                        "title": clamp_text(&r.label, limits::ROW_TITLE),
                    })).collect::<Vec<_>>() }] }),
                    &body,
                    None,
                ), footer));
            }
        }
        out.extend(links.iter().map(|(r, url)| cta(r, url, &r.label)));
        return out;
    }
    out.extend(media);
    let mut links = links.iter();
    if let Some((primary, url)) = links.next() {
        out.push(with_footer(cta(primary, url, &body), footer));
    }
    out.extend(links.map(|(r, url)| cta(r, url, &r.label)));
    out
}

/// `convertMessageToWhatsappMessage`.
pub fn convert_message(message: &Value, quick_replies: Option<&Value>) -> Vec<WaMessage> {
    let text = message.get("text").and_then(Value::as_str);
    let is_location_request = quick_replies.and_then(Value::as_array).is_some_and(|b| {
        b.iter()
            .any(|b| canonical_reply_payload(b).as_deref() == Some(NATIVE_LOCATION_REQUEST))
    });
    if is_location_request {
        return text
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|body| {
                WaMessage::raw(
                    "interactive",
                    json!({ "type": "location_request_message",
                        "body": { "text": clamp_text(body, limits::BODY_TEXT) },
                        "action": { "name": "send_location" } }),
                )
            })
            .into_iter()
            .collect();
    }
    let attributes = message.get("contentAttributes").unwrap_or(&Value::Null);
    let entity = attributes.get("type").and_then(Value::as_str);
    if entity == Some("whatsapp_call_permission_request")
        && let Some(text) = text.filter(|t| !t.is_empty())
    {
        return vec![WaMessage::raw(
            "interactive",
            json!({ "type": "call_permission_request", "body": { "text": text },
                "action": { "name": "call_permission_request" } }),
        )];
    }
    // A product row carries its resolved products; the text is only a preview.
    if entity == Some("product_message")
        && let Some(payload) = attributes.get("payload").filter(|p| p.is_object())
    {
        return super::whatsapp_steps::product_message(payload)
            .map(|m| vec![m])
            .unwrap_or_else(|error| {
                tracing::warn!(error = error.message(), "WhatsApp product message not sent");
                Vec::new()
            });
    }
    if message
        .get("contentType")
        .and_then(Value::as_str)
        .unwrap_or("text")
        != "text"
    {
        return vec![text_message(text.unwrap_or("not handled yet"))];
    }
    let mut out: Vec<WaMessage> = text
        .filter(|t| !t.is_empty())
        .map(text_message)
        .into_iter()
        .collect();
    for attachment in message
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let url = attachment.get("url").and_then(Value::as_str).unwrap_or("");
        let kind = match attachment.get("fileType").and_then(Value::as_str) {
            Some("image") => "image",
            Some("video") => "video",
            Some("audio") => "audio",
            _ => "document",
        };
        out.push(media_message(kind, url));
    }
    out
}

/// `convertFlowStepToWhatsappMessage` for the ported step types.
pub fn convert_flow_step(data: &SendFlowStep, pix_redirect: bool) -> ChannelResult<Vec<WaMessage>> {
    let step = &data.step;
    let buttons: Vec<Value> = step
        .get("buttons")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let quick: &[Value] = data
        .quick_replies
        .as_ref()
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let quick_count = quick.len();
    match str_field(step, "stepType") {
        "sendText" => {
            let text = str_field(step, "text");
            Ok(if buttons.len() + quick_count == 0 {
                vec![text_message(text)]
            } else {
                button_messages(data, &buttons, quick, text, None, None)
            })
        }
        "sendImage" => {
            let image = media_message("image", str_field(step, "url"));
            Ok(if buttons.len() + quick_count == 0 {
                vec![image]
            } else {
                button_messages(data, &buttons, quick, "", Some(image), None)
            })
        }
        "sendMultipleImages" => Ok(step
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|i| media_message("image", str_field(i, "url")))
            .collect()),
        "sendWaTemplateMessage" => Ok(vec![super::whatsapp_template::convert_wa_template(
            data,
            pix_redirect,
        )?]),
        "sendCarousel" => Ok(steps::carousel(data, quick)),
        "sendViewCatalog" => Ok(steps::view_catalog(step)),
        "whatsappOptionList" => steps::option_list(data),
        "whatsappCallButton" => Ok(steps::call_button(step)),
        "whatsappLocation" => Ok(steps::location(step)),
        "whatsappFlow" => steps::whatsapp_flow(data),
        "sendProducts" => steps::send_products(data),
        other => Err(SendFailure::Other(format!(
            "Unsupported WhatsApp flow step: {other}"
        ))),
    }
}

/// `resolveRecipientParams`: the BSUID when the contact is keyed by it.
fn recipient_params(contact: &OutgoingContact) -> (&'static str, String) {
    let ci = &contact.contact_inbox;
    let user_id = ci.source_user_id.as_deref().filter(|u| !u.is_empty());
    match user_id {
        Some(user_id) if ci.source_id == user_id || ci.source_id.is_empty() => {
            ("recipient", user_id.to_owned())
        }
        _ => ("to", ci.source_id.clone()),
    }
}

/// The POST body of one message: the lib sender's `{ messaging_product,
/// type, to, <type> }`, or the raw path's envelope (raw shapes and BSUID
/// recipients).
pub fn message_body(message: &WaMessage, recipient: (&str, &str)) -> Value {
    let mut body = Map::new();
    body.insert("messaging_product".into(), json!("whatsapp"));
    if message.raw || recipient.0 == "recipient" {
        body.insert("recipient_type".into(), json!("individual"));
    }
    body.insert(recipient.0.into(), json!(recipient.1));
    body.insert("type".into(), json!(message.kind));
    body.insert(message.kind.clone(), message.payload.clone());
    Value::Object(body)
}

async fn post_json(
    client: &reqwest::Client,
    url: String,
    token: &str,
    body: &Value,
) -> ChannelResult<Value> {
    let response = client
        .post(url)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| {
            SendFailure::Channel(ChannelError::new(e.to_string(), Category::NetworkError))
        })?;
    let status = response.status().as_u16();
    let parsed: Value = response
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if !(200..300).contains(&status) || parsed.get("error").is_some() {
        tracing::error!(status, body = %parsed, "WhatsApp Graph call failed");
        return Err(map_graph_error(status, &parsed).into());
    }
    Ok(parsed)
}

/// The Graph sends of one `sendMessage` / `sendFlowStep` call.
async fn send_messages(
    client: reqwest::Client,
    base: String,
    auth: ChannelResult<Auth>,
    recipient: (&'static str, String),
    messages: ChannelResult<Vec<WaMessage>>,
    media: Option<(Arc<WhatsappMedia>, i64)>,
) -> ChannelResult<SendResult> {
    let auth = auth?;
    let url = format!(
        "{base}/{DEFAULT_API_VERSION}/{}/messages",
        auth.phone_number_id
    );
    let mut result = SendResult::default();
    for message in messages? {
        let mut body = message_body(&message, (recipient.0, &recipient.1));
        if let Some((media, workspace_id)) = &media {
            super::whatsapp_media::prepare_outbound_media(
                media,
                &super::whatsapp_media::media_client(),
                *workspace_id,
                &mut body,
            )
            .await;
        }
        let response = post_json(&client, url.clone(), &auth.access_token, &body).await?;
        result.sent_count += 1;
        match response.pointer("/messages/0/id").and_then(Value::as_str) {
            Some(id) => result.message_ids.push(id.to_owned()),
            None => {
                tracing::warn!(kind = %message.kind, "WhatsApp message could not be sent")
            }
        }
    }
    Ok(result)
}

/// `ctx.integrationDetail.pixRedirectEnabled`.
async fn pix_redirect_enabled(db: &chatbotx_db::Db, integration_id: i64) -> bool {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "pixRedirectEnabled" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
                [integration_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false))
    })
    .await
    .unwrap_or(false)
}

// ---- inbound (`incomming-message.ts`, `incoming-reply.ts`) ----------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| match x {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// `readInteractiveReply`: `(text, postbackAction, buttonTitle, contentAttributes)`.
fn interactive_reply(
    reply: &Value,
    context_id: Option<String>,
) -> (String, Option<String>, Option<String>, Option<Value>) {
    const UNHANDLED: &str = "Received interactive (coming soon)";
    match str_field(reply, "type") {
        "call_permission_reply" => {
            let p = &reply["call_permission_reply"];
            let response = str_field(p, "response");
            if response == "accept" || response == "reject" {
                let expiration = p.get("expiration_timestamp").and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                });
                let mut entity = json!({
                    "type": "whatsapp_call_permission_reply",
                    "response": response,
                    "isPermanent": p["is_permanent"] == json!(true),
                });
                if let Some(e) = expiration {
                    entity["expirationTimestamp"] = json!(e);
                }
                if let Some(src) = s(p, "response_source") {
                    entity["responseSource"] = json!(src);
                }
                let text = if response == "accept" {
                    "Accepted call permission request"
                } else {
                    "Declined call permission request"
                };
                return (text.into(), None, None, Some(entity));
            }
            (UNHANDLED.into(), None, None, None)
        }
        "button_reply" => {
            let r = &reply["button_reply"];
            let title = str_field(r, "title").to_owned();
            (title.clone(), s(r, "id"), Some(title), None)
        }
        "list_reply" => {
            let r = &reply["list_reply"];
            let title = str_field(r, "title").to_owned();
            (title.clone(), s(r, "id"), Some(title), Some(r.clone()))
        }
        "nfm_reply" => {
            let r = &reply["nfm_reply"];
            let flow_response: Value = r
                .get("response_json")
                .and_then(Value::as_str)
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_else(|| json!({}));
            let token = s(&flow_response, "flow_token");
            let decoded = token
                .as_deref()
                .and_then(|t| chatbotx_core::flow_config::util::decode_button_payload(t, false));
            let mut entity = json!({
                "type": "whatsapp_flow_response",
                "name": r.get("name").cloned().unwrap_or(Value::Null),
                "flowResponse": flow_response,
                "flowToken": token,
                "decoded": decoded.as_ref().map(|d| json!({ "flowId": d.flow_id, "buttonId": d.button_id })),
            });
            if let Some(id) = context_id {
                entity["contextMessageId"] = json!(id);
            }
            let postback = token
                .clone()
                .filter(|_| decoded.as_ref().is_some_and(|d| d.button_id.is_some()));
            let body = s(r, "body");
            // ponytail: `templateFlowToken` (template-launched Flows) and the
            // Flow response media download are not ported.
            (
                body.clone().unwrap_or_default(),
                postback,
                body,
                Some(entity),
            )
        }
        _ => (UNHANDLED.into(), None, None, None),
    }
}

/// `receiveMessage`: one `whatsapp-api-js` `OnMessageArgs` payload.
fn non_empty(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `readOrderCommerce`: one malformed item does not drop the others; a cart
/// with no catalog or no usable item reports nothing.
fn order_commerce(order: &Value) -> Option<Value> {
    let catalog_id = non_empty(order, "catalog_id")?;
    let note = match &order["text"] {
        Value::Null => None,
        Value::String(text) => Some(text.clone()).filter(|t| !t.is_empty()),
        _ => return None,
    };
    let items: Vec<Value> = order["product_items"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            let number = |key: &str| item[key].is_number().then(|| item[key].clone());
            Some(json!({
                "catalogId": catalog_id,
                "retailerId": non_empty(item, "product_retailer_id")?,
                "quantity": number("quantity")?,
                "itemPrice": number("item_price")?,
                "currency": item["currency"].as_str()?,
            }))
        })
        .collect();
    (!items.is_empty()).then(
        || json!({ "kind": "cartSent", "catalogId": catalog_id, "note": note, "items": items }),
    )
}

/// `readReferredProductCommerce`: "Message business" on a catalog message.
fn referred_product_commerce(referred: &Value) -> Option<Value> {
    Some(json!({
        "kind": "productInquiry",
        "product": {
            "catalogId": non_empty(referred, "catalog_id")?,
            "retailerId": non_empty(referred, "product_retailer_id")?,
        },
    }))
}

pub fn parse_incoming(payload: &Value) -> Received {
    let info = crate::thread_control::whatsapp_receive_info(&payload["raw"]);
    // A standby echo is a partner's outgoing message (its Send API body), never a
    // customer message.
    if info["delivery"] == "standby"
        && crate::thread_control::is_standby_echo_item(&payload["message"])
    {
        return crate::thread_control::parse_standby_echo(&payload["message"], info);
    }
    let message = &payload["message"];
    let mut incoming = IncomingMessage::incoming(s(message, "id"));
    // `extractWhatsappUserIdentity(raw)`: the BSUID fields of the contact.
    let raw_contact = payload
        .pointer("/raw/entry/0/changes/0/value/contacts/0")
        .unwrap_or(&Value::Null);
    let source_user_id = s(raw_contact, "user_id").or_else(|| s(message, "from_user_id"));
    let contact = IncomingContact {
        source_id: s(payload, "from")
            .or_else(|| source_user_id.clone())
            .unwrap_or_default(),
        first_name: s(payload, "name"),
        source_user_id: source_user_id.clone(),
        source_parent_user_id: s(raw_contact, "parent_user_id"),
        source_username: s(raw_contact, "username"),
        ..IncomingContact::default()
    };
    let mut received = Received::default();
    let kind = str_field(message, "type");
    let media = |key: &str, file_type: &str| {
        let m = &message[key];
        IncomingAttachment {
            source_id: s(m, "id"),
            file_type: file_type.into(),
            mime_type: str_field(m, "mime_type").to_owned(),
            // The Graph media id until `receive_message` stores the bytes.
            origin_path: str_field(m, "id").to_owned(),
            name: s(m, "filename"),
            is_photo_message: key == "image",
            ..IncomingAttachment::default()
        }
    };
    match kind {
        "text" => {
            // ponytail: `extractInvisibleGoogleClick` is not ported.
            let body = str_field(&message["text"], "body");
            match body.strip_prefix("/ref-") {
                Some(r) => received.r#ref = Some(r.to_owned()),
                None => incoming.text = Some(body.to_owned()),
            }
        }
        "location" => {
            let loc = &message["location"];
            let label = [s(loc, "name"), s(loc, "address")]
                .into_iter()
                .flatten()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(": ");
            incoming.content_type = "location".into();
            incoming.text = Some(if label.is_empty() {
                "Received location".into()
            } else {
                label
            });
            incoming.content_attributes = Some(loc.clone());
        }
        "contacts" => {
            incoming.text = Some("Received contacts".into());
            incoming.content_attributes = Some(json!({ "contacts": message["contacts"] }));
        }
        "order" => {
            let order = &message["order"];
            let mut lines = vec!["Received order".to_owned()];
            for item in order["product_items"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "{} × {} ({} {})",
                    s(item, "product_retailer_id").unwrap_or_default(),
                    item["quantity"],
                    item["item_price"],
                    s(item, "currency").unwrap_or_default()
                ));
            }
            if let Some(note) = s(order, "text").filter(|n| !n.is_empty()) {
                lines.push(note);
            }
            incoming.text = Some(lines.join("\n"));
            incoming.content_attributes = Some(order.clone());
            received.commerce = order_commerce(order);
        }
        "interactive" => {
            let context = message
                .pointer("/context/id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let (text, postback, title, attributes) =
                interactive_reply(&message["interactive"], context);
            incoming.text = Some(text);
            received.postback_action = postback;
            received.button_title = title;
            incoming.content_attributes = attributes;
        }
        "button" => {
            let b = &message["button"];
            incoming.text = s(b, "text");
            received.postback_action = s(b, "payload");
            received.button_title = s(b, "text");
        }
        "audio" => incoming.attachments = vec![media("audio", "audio")],
        "video" | "image" | "document" | "sticker" => {
            let file_type = match kind {
                "document" => "file",
                "sticker" => "image",
                other => other,
            };
            incoming.attachments = vec![media(kind, file_type)];
            // Only `document` and `image` captions become the text (a video
            // caption is ignored, as in the original).
            if matches!(kind, "document" | "image") {
                incoming.text = s(&message[kind], "caption");
            }
        }
        other => incoming.text = Some(format!("Received {other}")),
    }
    // `getWhatsappReferral` (Click-to-WhatsApp ads).
    if let Some(r) = message.get("referral").filter(|r| r.is_object()) {
        received.r#ref = received.r#ref.take().or_else(|| s(r, "source_id"));
        received.referral_source = s(r, "source_type");
        received.referral = Some(json!({
            "ref": s(r, "source_id"), "source": s(r, "source_type"), "type": s(r, "source_type"),
            "adId": s(r, "source_id"), "adTitle": s(r, "headline"), "adBody": s(r, "body"),
            "sourceUrl": s(r, "source_url"), "ctwaClid": s(r, "ctwa_clid"),
            "photoUrl": s(r, "image_url").or_else(|| s(r, "thumbnail_url")),
            "videoUrl": s(r, "video_url"), "raw": r,
        }));
    }
    if kind == "text" {
        received.commerce = referred_product_commerce(&message["context"]["referred_product"]);
    }
    // ponytail: the `deriveAdSourcePlatform` field is not ported.
    received.message = Some(incoming);
    received.contact = contact;
    received.thread_control = Some(info);
    received
}

impl Channel for Whatsapp {
    fn delivery_receipts(&self) -> bool {
        true
    }

    fn receive_message(
        &self,
        ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        let mut received = parse_incoming(&payload);
        let media = self.media.clone();
        let client = self.media_client.clone();
        let base = self.base_url.clone();
        let token = ctx
            .auth
            .pointer("/tokens/accessToken")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let loc = chatbotx_files::ChannelMediaLocation {
            channel: ctx.channel.clone(),
            workspace_id: ctx.workspace_id.to_string(),
            integration_id: ctx.integration_id.to_string(),
        };
        Box::pin(async move {
            // ponytail: without a file store the Graph media id is kept.
            if let (Some(media), Some(message)) = (media, received.message.as_mut()) {
                for attachment in &mut message.attachments {
                    super::whatsapp_media::store_inbound_media(
                        &media, &client, &base, &token, &loc, attachment,
                    )
                    .await?;
                }
            }
            Ok(Some(received))
        })
    }

    fn message_status(
        &self,
        _ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<MessageStatus>>> {
        let status = MessageStatus {
            message_id: s(&payload, "messageId").unwrap_or_default(),
            status: s(&payload, "status").unwrap_or_default(),
            timestamp: s(&payload, "timestamp")
                .and_then(|t| t.parse::<i64>().ok())
                .map(|t| t * 1000),
            error: payload.get("error").filter(|e| !e.is_null()).map(|e| {
                s(e, "message")
                    .or_else(|| s(e, "title"))
                    .unwrap_or_else(|| e.to_string())
            }),
            recipient: s(&payload, "phone").or_else(|| s(&payload, "recipientUserId")),
        };
        Box::pin(std::future::ready(Ok(Some(status))))
    }

    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let messages = Ok(convert_message(&data.message, data.quick_replies.as_ref()));
        Box::pin(send_messages(
            self.client.clone(),
            self.base_url.clone(),
            auth_of(ctx),
            recipient_params(&data.contact),
            messages,
            self.media.clone().map(|m| (m, ctx.workspace_id)),
        ))
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let (client, base, auth) = (self.client.clone(), self.base_url.clone(), auth_of(ctx));
        let recipient = recipient_params(&data.contact);
        let (db, integration_id) = (ctx.db.clone(), ctx.integration_id);
        let media = self.media.clone().map(|m| (m, ctx.workspace_id));
        Box::pin(async move {
            let pix_redirect = str_field(&data.step, "stepType") == "sendWaTemplateMessage"
                && pix_redirect_enabled(&db, integration_id).await;
            let messages = convert_flow_step(&data, pix_redirect);
            send_messages(client, base, auth, recipient, messages, media).await
        })
    }

    /// `sendTyping`: `markAsRead(phoneId, wamid, "text")` (typing on only).
    fn send_typing(
        &self,
        ctx: &ChannelCtx,
        data: SendTyping,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let client = self.client.clone();
        let base = self.base_url.clone();
        let auth = auth_of(ctx);
        Box::pin(async move {
            let Some(wamid) = data.message_source_id.filter(|_| data.typing) else {
                return Ok(());
            };
            let auth = auth?;
            post_json(
                &client,
                format!(
                    "{base}/{DEFAULT_API_VERSION}/{}/messages",
                    auth.phone_number_id
                ),
                &auth.access_token,
                &json!({ "messaging_product": "whatsapp", "status": "read", "message_id": wamid,
                    "typing_indicator": { "type": "text" } }),
            )
            .await
            .map(|_| ())
        })
    }
}

#[cfg(test)]
mod commerce_tests {
    use super::*;

    #[test]
    fn order_keeps_usable_items_only() {
        let order = json!({ "catalog_id": "c1", "text": "", "product_items": [
            { "product_retailer_id": "sku", "quantity": 2, "item_price": 9.5, "currency": "EUR" },
            { "product_retailer_id": "", "quantity": 1, "item_price": 1, "currency": "EUR" },
        ] });
        let event = order_commerce(&order).expect("cart");
        assert_eq!(event["note"], Value::Null);
        assert_eq!(event["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(event["items"][0]["quantity"], json!(2));
        assert_eq!(
            order_commerce(&json!({ "catalog_id": "", "product_items": [] })),
            None
        );
        assert_eq!(referred_product_commerce(&Value::Null), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_mapper_categories_follow_the_code_table() {
        assert_eq!(categorize(Some(131_047), None), Category::QuotaExceeded);
        assert_eq!(categorize(Some(190), None), Category::AuthFailed);
        assert_eq!(
            categorize(Some(138_014), Some("OAuthException")),
            Category::PermissionDenied
        );
        assert_eq!(categorize(Some(250), None), Category::PermissionDenied);
        assert_eq!(
            categorize(None, Some("OAuthException")),
            Category::AuthFailed
        );
        assert_eq!(categorize(Some(999_999), None), Category::Unknown);
        let mapped = map_graph_error(
            400,
            &json!({ "error": { "message": "Re-engagement", "code": 131_047 } }),
        );
        assert_eq!(mapped.category, Category::QuotaExceeded);
        assert_eq!(mapped.code.as_deref(), Some("131047"));
        assert_eq!(mapped.http_status_code, 400);
    }

    #[test]
    fn plain_text_uses_the_lib_body_and_a_bsuid_the_raw_one() {
        let message = &convert_message(&json!({ "text": "hi", "contentType": "text" }), None)[0];
        assert_eq!(
            message_body(message, ("to", "84900")),
            json!({ "messaging_product": "whatsapp", "to": "84900", "type": "text", "text": { "body": "hi" } })
        );
        assert_eq!(
            message_body(message, ("recipient", "US.1")),
            json!({ "messaging_product": "whatsapp", "recipient_type": "individual", "recipient": "US.1",
                "type": "text", "text": { "body": "hi" } })
        );
    }

    #[test]
    fn a_call_permission_request_is_the_interactive_shape() {
        let out = convert_message(
            &json!({ "text": "May we call?", "contentType": "text",
                "contentAttributes": { "type": "whatsapp_call_permission_request" } }),
            None,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].raw);
        assert_eq!(out[0].payload["type"], "call_permission_request");
        assert_eq!(out[0].payload["body"]["text"], "May we call?");
    }

    #[test]
    fn split_text_cuts_at_word_breaks() {
        // `/\s+\S*$/` on the 7-unit head "aaa bbb" cuts before "bbb".
        assert_eq!(split_text("aaa bbb ccc", 7), vec!["aaa", "bbb ccc"]);
        assert_eq!(split_text("abcdefgh", 4), vec!["abcd", "efgh"]);
        assert_eq!(clamp_text("  hello world  ", 5), "hello");
    }

    #[test]
    fn parses_text_ref_button_and_list_replies() {
        let wrap = |message: Value| json!({ "phoneID": "1", "from": "849", "name": "Ada", "message": message });
        let r = parse_incoming(&wrap(
            json!({ "id": "wamid.1", "type": "text", "text": { "body": "hello" } }),
        ));
        assert_eq!(r.message.as_ref().unwrap().text.as_deref(), Some("hello"));
        assert_eq!(r.contact.source_id, "849");
        assert_eq!(r.contact.first_name.as_deref(), Some("Ada"));
        let r = parse_incoming(&wrap(
            json!({ "id": "w", "type": "text", "text": { "body": "/ref-promo" } }),
        ));
        assert_eq!(r.r#ref.as_deref(), Some("promo"));
        assert_eq!(r.message.unwrap().text, None);
        let r = parse_incoming(&wrap(
            json!({ "id": "w", "type": "interactive", "interactive": {
            "type": "button_reply", "button_reply": { "id": "f:1::b1", "title": "Yes" } } }),
        ));
        assert_eq!(r.postback_action.as_deref(), Some("f:1::b1"));
        assert_eq!(r.button_title.as_deref(), Some("Yes"));
        let r = parse_incoming(&wrap(json!({ "id": "w", "type": "location", "location": {
            "latitude": 1, "longitude": 2, "name": "Home", "address": "Street" } })));
        let m = r.message.unwrap();
        assert_eq!(
            (m.content_type.as_str(), m.text.as_deref()),
            ("location", Some("Home: Street"))
        );
        let r = parse_incoming(&wrap(json!({ "id": "w", "type": "reaction" })));
        assert_eq!(
            r.message.unwrap().text.as_deref(),
            Some("Received reaction")
        );
    }
}
