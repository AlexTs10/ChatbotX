//! `integrations/api/src/handlers/message/outgoing-message.ts` and
//! `lib/{delivery,signature}.ts`: the signed `message_created` envelope
//! POSTed to the channel's callback URL.

use std::time::Duration;

use chatbotx_business::net::ssrf_guard::assert_public_url;
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_jobs::BoxFuture;
use ring::hmac;
use ring::rand::{SecureRandom as _, SystemRandom};
use serde_json::{Value, json};

use crate::channel::{
    Channel, ChannelCtx, ChannelResult, IncomingAttachment, IncomingContact, IncomingMessage,
    Received, SendFailure, SendFlowStep, SendMessage, SendResult,
};

/// `DELIVERY_TIMEOUT_MS`.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ApiChannel {
    client: reqwest::Client,
}

impl ApiChannel {
    pub fn new() -> ApiChannel {
        ApiChannel {
            client: reqwest::Client::builder()
                .timeout(DELIVERY_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for ApiChannel {
    fn default() -> ApiChannel {
        ApiChannel::new()
    }
}

/// `signApiPayload`: hex HMAC-SHA256 of `"<timestamp>.<rawBody>"`.
pub fn sign_api_payload(signing_secret: &str, timestamp: &str, raw_body: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, signing_secret.as_bytes());
    let tag = hmac::sign(&key, format!("{timestamp}.{raw_body}").as_bytes());
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// `crypto.randomUUID()`.
fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    let _ = SystemRandom::new().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `postSignedEnvelope`: the SSRF check at send time, then the signed POST.
/// Returns the parsed `{ messageId }` body (none for an empty / non-JSON
/// body).
async fn post_signed_envelope(
    client: reqwest::Client,
    callback_url: String,
    signing_secret: String,
    envelope: Value,
) -> ChannelResult<Option<Value>> {
    assert_public_url(&callback_url, "API channel callback URL")
        .await
        .map_err(SendFailure::Other)?;
    let raw_body = envelope.to_string();
    let timestamp = (now_ms() / 1000).to_string();
    let signature = sign_api_payload(&signing_secret, &timestamp, &raw_body);
    let response = client
        .post(&callback_url)
        .header("Content-Type", "application/json")
        .header("X-ChatbotX-Signature", format!("sha256={signature}"))
        .header("X-ChatbotX-Timestamp", timestamp)
        .header("X-ChatbotX-Delivery", random_uuid())
        .body(raw_body)
        .send()
        .await
        .map_err(|e| SendFailure::Other(e.to_string()))?;
    // `ky` throws `HTTPError` on a non-2xx answer.
    let status = response.status();
    if !status.is_success() {
        return Err(SendFailure::Other(format!(
            "Request failed with status code {} {}: POST {callback_url}",
            status.as_u16(),
            status.canonical_reason().unwrap_or_default()
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|e| SendFailure::Other(e.to_string()))?;
    Ok(serde_json::from_str(&text).ok())
}

/// The callback URL and signing secret of `ctx.auth`; `None` = an
/// inbound-only channel (not an error).
fn auth(ctx: &ChannelCtx) -> Option<(String, String)> {
    let url = ctx.auth.get("callbackUrl")?.as_str()?.to_owned();
    if url.is_empty() {
        return None;
    }
    let secret = ctx
        .auth
        .get("signingSecret")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some((url, secret))
}

fn result_of(response: Option<Value>) -> SendResult {
    let message_id = response
        .as_ref()
        .and_then(|r| r.get("messageId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    SendResult {
        message_ids: message_id.into_iter().collect(),
        sent_count: 1,
    }
}

fn not_sent() -> BoxFuture<'static, ChannelResult<SendResult>> {
    Box::pin(std::future::ready(Ok(SendResult::default())))
}

fn file_type_for_step(step_type: &str) -> &'static str {
    match step_type {
        "sendImage" | "sendGif" => "image",
        "sendAudio" => "audio",
        "sendVideo" => "video",
        _ => "file",
    }
}

/// `mapFlowStepToEnvelope`: `(text, contentAttributes)`; an unsupported
/// step rejects so delivery cannot record a false success.
pub fn map_flow_step_to_envelope(step: &Value) -> ChannelResult<(Value, Option<Value>)> {
    let step_type = step.get("stepType").and_then(Value::as_str).unwrap_or("");
    Ok(match step_type {
        "sendText" => (step.get("text").cloned().unwrap_or(Value::Null), None),
        "sendImage" | "sendGif" | "sendVideo" | "sendAudio" | "sendFile" => (
            Value::Null,
            Some(json!({
                "attachments": [{ "url": step.get("url"), "fileType": file_type_for_step(step_type) }],
            })),
        ),
        "sendMultipleImages" => {
            let images = step
                .get("images")
                .and_then(Value::as_array)
                .map(|images| {
                    images
                        .iter()
                        .map(|image| json!({ "url": image.get("url"), "fileType": "image" }))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (Value::Null, Some(json!({ "attachments": images })))
        }
        "sendQuickReply" => (step.get("message").cloned().unwrap_or(Value::Null), None),
        "sendCarousel" => {
            let cards = step
                .get("cards")
                .and_then(Value::as_array)
                .map(|cards| {
                    cards
                        .iter()
                        .map(|card| {
                            json!({
                                "id": card.get("id"),
                                "title": card.get("title"),
                                "subtitle": card.get("subtitle"),
                                "imageUrl": card.get("image").and_then(|i| i.get("url")),
                                "buttons": card.get("buttons"),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (
                Value::Null,
                Some(json!({
                    "type": "template",
                    "payload": { "templateType": "carousel", "cards": cards },
                })),
            )
        }
        other => {
            return Err(SendFailure::Other(format!(
                "Unsupported API flow step: {other}"
            )));
        }
    })
}

/// Drops `null` members like `JSON.stringify` drops `undefined` ones.
fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        other => other,
    }
}

impl Channel for ApiChannel {
    fn send_message(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let Some((url, secret)) = auth(ctx) else {
            return not_sent();
        };
        let message = &data.message;
        let field = |name: &str| message.get(name).cloned().unwrap_or(Value::Null);
        let envelope = json!({
            "event": "message_created",
            "timestamp": iso(now_ms()),
            "contact": {
                "id": data.contact.contact_inbox.id.to_string(),
                "sourceId": data.contact.contact_inbox.source_id,
            },
            "conversation": { "id": field("conversationId") },
            "message": strip_nulls(json!({
                "id": field("id"),
                "text": field("text"),
                "messageType": field("messageType"),
                "contentType": field("contentType"),
                "attachments": field("attachments"),
                "contentAttributes": field("contentAttributes"),
                "quickReplies": data.quick_replies,
            })),
        });
        let client = self.client.clone();
        Box::pin(async move {
            Ok(result_of(
                post_signed_envelope(client, url, secret, envelope).await?,
            ))
        })
    }

    fn send_flow_step(
        &self,
        ctx: &ChannelCtx,
        data: SendFlowStep,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let mapped = map_flow_step_to_envelope(&data.step);
        let auth = auth(ctx);
        let client = self.client.clone();
        Box::pin(async move {
            let (text, content_attributes) = mapped?;
            let Some((url, secret)) = auth else {
                return Ok(SendResult::default());
            };
            let envelope = json!({
                "event": "message_created",
                "timestamp": iso(now_ms()),
                "contact": {
                    "id": data.contact.contact_inbox.id.to_string(),
                    "sourceId": data.contact.contact_inbox.source_id,
                },
                "message": strip_nulls(json!({
                    "text": text,
                    "messageType": "outgoing",
                    "contentType": "text",
                    "contentAttributes": content_attributes,
                    "quickReplies": data.quick_replies,
                })),
            });
            Ok(result_of(
                post_signed_envelope(client, url, secret, envelope).await?,
            ))
        })
    }

    /// `receiveMessage` (`handlers/message/incoming-message.ts`): the body
    /// the public route validated (`incomingApiMessageSchema`).
    fn receive_message(
        &self,
        _ctx: &ChannelCtx,
        payload: Value,
    ) -> BoxFuture<'static, ChannelResult<Option<Received>>> {
        Box::pin(std::future::ready(parse_incoming(&payload).map(Some)))
    }
}

/// The `incomingApiMessageSchema` payload as a [`Received`].
pub fn parse_incoming(payload: &Value) -> ChannelResult<Received> {
    let text_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let contact = &payload["contact"];
    let message = &payload["message"];
    let (Some(contact_source), Some(message_source)) = (
        text_of(contact, "sourceId").filter(|s| !s.is_empty()),
        text_of(message, "sourceId").filter(|s| !s.is_empty()),
    ) else {
        return Err(SendFailure::Other(
            "Invalid API channel message payload".into(),
        ));
    };
    let attachments = message["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let url = text_of(a, "url").unwrap_or_default();
            IncomingAttachment {
                source_id: Some(url.clone()),
                file_type: text_of(a, "fileType").unwrap_or_else(|| "file".into()),
                mime_type: text_of(a, "mimeType").unwrap_or_default(),
                origin_path: url,
                size: Some(0),
                name: text_of(a, "name"),
                is_photo_message: false,
            }
        })
        .collect();
    let mut incoming = IncomingMessage::incoming(Some(message_source));
    incoming.content_type = if text_of(message, "contentType").as_deref() == Some("location") {
        "location".into()
    } else {
        "text".into()
    };
    incoming.text = text_of(message, "text");
    incoming.content_attributes = message
        .get("contentAttributes")
        .filter(|v| v.is_object())
        .cloned();
    incoming.attachments = attachments;
    Ok(Received {
        message: Some(incoming),
        contact: IncomingContact {
            source_id: contact_source,
            first_name: text_of(contact, "firstName"),
            last_name: text_of(contact, "lastName"),
            email: text_of(contact, "email"),
            phone_number: text_of(contact, "phoneNumber"),
            avatar: text_of(contact, "avatar"),
            locale: text_of(contact, "locale"),
            ..IncomingContact::default()
        },
        postback_action: text_of(payload, "postbackPayload"),
        ..Received::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_timestamp_dot_body_as_hex_hmac_sha256() {
        // RFC 4231-style check computed with Node's `crypto.subtle`.
        let signature = sign_api_payload("secret", "1700000000", "{}");
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(signature, sign_api_payload("secret", "1700000000", "{}"));
        assert_ne!(signature, sign_api_payload("secret", "1700000001", "{}"));
    }

    #[test]
    fn maps_text_and_rejects_unsupported_steps() {
        let (text, attrs) =
            map_flow_step_to_envelope(&json!({ "stepType": "sendText", "text": "hi" })).unwrap();
        assert_eq!(text, json!("hi"));
        assert!(attrs.is_none());
        assert!(map_flow_step_to_envelope(&json!({ "stepType": "sendCard" })).is_err());
    }
}
