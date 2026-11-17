//! `integrations/messenger/src/handlers/webhook.ts`: the Page webhook
//! (`GET` subscription handshake, `POST` events with the
//! `X-Hub-Signature-256` check) and the integration jobs of one delivery.

use std::sync::LazyLock;

use chatbotx_core::utils::crypto::{hmac_sha256_hex, timing_safe_string_equal};
use regex::Regex;
use serde_json::{Value, json};

use super::messenger::MESSAGE_METADATA;
use super::whatsapp_webhook::WebhookJob;

/// `MessengerWebhookException` text: `"Webhook error: " + message`.
pub fn webhook_error(message: &str) -> String {
    format!("Webhook error: {message}")
}

/// `quoteUnsafeMessageIds`: bare 16+ digit `message_id` /
/// `messenger_subscription_token` numbers become strings before parsing
/// (they exceed 2^53). `regex` has no look-ahead, so the terminator is
/// captured and written back.
pub fn quote_unsafe_message_ids(body: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"("(?:message_id|messenger_subscription_token)"\s*:\s*)(\d{16,})(\s*[,}\]])"#)
            .expect("static regex")
    });
    RE.replace_all(body, r#"$1"$2"$3"#).into_owned()
}

/// `verifyWebhookSignature`: `sha256=<hex>` (exactly one `=`) against the
/// HMAC-SHA256 of the raw body, compared in constant time.
pub fn verify_signature(body: &str, signature: &str, client_secret: &str) -> bool {
    let parts: Vec<&str> = signature.split('=').collect();
    let [_, hash] = parts.as_slice() else {
        return false;
    };
    timing_safe_string_equal(hash, &hmac_sha256_hex(client_secret, body))
}

/// `handleSubscriptionEvent`: the challenge, or the error text.
pub fn handshake(query: Option<&str>, verify_token: Option<&str>) -> Result<String, &'static str> {
    let param = |key: &str| {
        url::form_urlencoded::parse(query.unwrap_or("").as_bytes())
            .filter(|(k, _)| k == key)
            .last()
            .map(|(_, v)| v.into_owned())
    };
    let challenge = param("hub.challenge").filter(|c| !c.is_empty());
    if param("hub.mode").as_deref() == Some("subscribe")
        && param("hub.verify_token").as_deref() == verify_token
        && let Some(challenge) = challenge
    {
        return Ok(challenge);
    }
    Err("Invalid webhook verification parameters")
}

/// What a verified request answers: the handshake challenge, or `"ok"`
/// and the jobs to queue.
#[derive(Debug)]
pub enum Outcome {
    Challenge(String),
    Jobs(Vec<WebhookJob>),
}

/// `webhookHandler` error text of a failure inside `handleWebhookEvent`
/// (parse or enqueue).
pub fn event_error(message: &str) -> String {
    webhook_error(&format!(
        "Webhook processing failed: {}",
        webhook_error(&format!("Failed to process webhook event: {message}"))
    ))
}

/// `webhookHandler`: `GET` = handshake, `POST` = verified events. `Err` =
/// the nested `MessengerWebhookException` text (HTTP 400).
pub fn handle_request(
    is_get: bool,
    query: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
    verify_token: Option<&str>,
    client_secret: &str,
) -> Result<Outcome, String> {
    handle_request_with(
        is_get,
        query,
        signature,
        body,
        verify_token,
        client_secret,
        webhook_jobs,
    )
}

/// The `webhookHandler` shape the Meta channels share (handshake, body,
/// signature, then the channel's own job extraction).
pub fn handle_request_with(
    is_get: bool,
    query: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
    verify_token: Option<&str>,
    client_secret: &str,
    jobs_of: fn(&str) -> Result<Vec<WebhookJob>, String>,
) -> Result<Outcome, String> {
    if is_get {
        return handshake(query, verify_token)
            .map(Outcome::Challenge)
            .map_err(|e| {
                webhook_error(&format!("Webhook processing failed: {}", webhook_error(e)))
            });
    }
    let body = String::from_utf8_lossy(body);
    let fail = |m: &str| Err(event_error(&webhook_error(m)));
    if body.is_empty() {
        return fail("Empty webhook payload");
    }
    let signature = signature.unwrap_or("");
    if signature.is_empty() {
        return fail("Missing webhook signature");
    }
    if !verify_signature(&body, signature, client_secret) {
        return fail("Invalid webhook signature");
    }
    jobs_of(&body)
        .map(Outcome::Jobs)
        .map_err(|e| event_error(&e))
}

fn id_of(v: &Value, pointer: &str) -> Option<String> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `classifyMessagingRoutingItem` (handover / request / app roles / the
/// AI hand-back notice).
pub fn is_routing_item(item: &Value) -> bool {
    let Some(map) = item.as_object() else {
        return false;
    };
    if [
        "pass_thread_control",
        "take_thread_control",
        "request_thread_control",
        "app_roles",
    ]
    .iter()
    .any(|k| map.contains_key(*k))
    {
        return true;
    }
    is_ai_handback_notice(item)
}

/// `isAiHandbackNotice`: a `message` with only an `admin_text` naming the AI.
fn is_ai_handback_notice(item: &Value) -> bool {
    static AI_WORD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bAI\b").expect("static regex"));
    let Some(message) = item.get("message").filter(|m| m.is_object()) else {
        return false;
    };
    message
        .get("admin_text")
        .and_then(Value::as_str)
        .is_some_and(|t| AI_WORD.is_match(t))
        && message.get("mid").is_none()
}

/// `messengerMessagingEventSchema`: sender / recipient ids and a numeric
/// timestamp; an invalid `reaction.action` fails the item.
fn is_valid_event(item: &Value) -> bool {
    id_of(item, "/sender/id").is_some()
        && id_of(item, "/recipient/id").is_some()
        && item.get("timestamp").is_some_and(Value::is_number)
        && item.get("reaction").is_none_or(|r| {
            matches!(
                r.get("action").and_then(Value::as_str),
                Some("react" | "unreact")
            ) && r.get("mid").is_some_and(Value::is_string)
        })
        && item
            .get("message")
            .is_none_or(|m| m.get("mid").is_some_and(Value::is_string))
        && item.get("postback").is_none_or(|p| {
            p.get("mid").is_some_and(Value::is_string)
                && p.get("title").is_some_and(Value::is_string)
                && p.get("payload").is_some_and(Value::is_string)
        })
}

fn job(name: &'static str, data: Value) -> WebhookJob {
    WebhookJob {
        name,
        data: json!({ "type": name, "data": data }),
        job_id: String::new(),
    }
}

/// The jobs of one verified delivery (`handleWebhookEvent` after the
/// signature check). `Err` = the body is not JSON.
pub fn webhook_jobs(body: &str) -> Result<Vec<WebhookJob>, String> {
    let parsed: Value =
        serde_json::from_str(&quote_unsafe_message_ids(body)).map_err(|e| e.to_string())?;
    let mut jobs = Vec::new();
    // `incomingWebhookEventSchema`: `object: "page"` and an `entry` array.
    let (Some("page"), Some(entries)) = (
        parsed.get("object").and_then(Value::as_str),
        parsed.get("entry").and_then(Value::as_array),
    ) else {
        tracing::warn!("messenger webhook payload unrecognized — skipping");
        return Ok(jobs);
    };
    for entry in entries {
        let (Some(page_id), Some(time)) = (
            entry.get("id").and_then(Value::as_str),
            entry.get("time").filter(|t| t.is_number()),
        ) else {
            tracing::warn!("messenger webhook entry skipped: malformed entry");
            continue;
        };
        let changes: Vec<&Value> = entry
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        // ponytail: Conversation Routing items (`threadControlEvent`:
        // handover, requests, app roles, standby) wait for the thread
        // control port (Q.D12); they are dropped with a log.
        let routing = entry
            .get("messaging")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|i| is_routing_item(i))
            .count()
            + entry
                .get("standby")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
        if routing > 0 {
            tracing::info!(
                page_id,
                routing,
                "Messenger conversation routing items not processed (thread control not ported)"
            );
        }
        if changes
            .iter()
            .any(|c| c.get("field").and_then(Value::as_str) == Some("inbox_labels"))
        {
            jobs.push(job(
                "channelLabelChange",
                json!({
                    "integrationType": "messenger",
                    "integrationIdentifier": page_id,
                    "payload": { "object": "page", "entry": [entry] },
                }),
            ));
            continue;
        }
        let field_changes = |field: &str| -> Vec<&Value> {
            changes
                .iter()
                .filter(|c| c.get("field").and_then(Value::as_str) == Some(field))
                .filter_map(|c| c.get("value"))
                .collect()
        };
        let feed = field_changes("feed");
        if !feed.is_empty() {
            for value in feed {
                feed_job(page_id, value)
                    .into_iter()
                    .for_each(|j| jobs.push(j));
            }
            continue;
        }
        let leadgen = field_changes("leadgen");
        if !leadgen.is_empty() {
            for value in leadgen {
                let (Some(leadgen_id), Some(form_id), Some(_)) = (
                    value.get("leadgen_id").and_then(Value::as_str),
                    value.get("form_id").and_then(Value::as_str),
                    value.get("page_id").and_then(Value::as_str),
                ) else {
                    tracing::warn!("Unrecognized leadgen webhook payload");
                    continue;
                };
                jobs.push(job(
                    "processLeadgen",
                    json!({
                        "integrationType": "messenger",
                        "integrationIdentifier": page_id,
                        "leadgenId": leadgen_id,
                        "formId": form_id,
                        "adgroupId": value.get("adgroup_id").or_else(|| value.get("ad_id")),
                    }),
                ));
            }
            continue;
        }
        for item in entry
            .get("messaging")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if is_routing_item(item) {
                continue;
            }
            if let Some(field) = item
                .get("field")
                .and_then(Value::as_str)
                .filter(|f| f.starts_with("marketing_message_"))
            {
                jobs.push(job(
                    "processMarketingMessageWebhook",
                    json!({
                        "pageId": page_id, "field": field, "value": item,
                    }),
                ));
                continue;
            }
            if !is_valid_event(item) {
                tracing::warn!("messenger messaging event skipped: malformed item");
                continue;
            }
            if let Some(j) = messaging_job(page_id, time, item) {
                jobs.push(j);
            }
        }
    }
    Ok(jobs)
}

/// One `feed` change: a new / edited / removed comment of another user.
fn feed_job(page_id: &str, value: &Value) -> Option<WebhookJob> {
    let text = |k: &str| value.get(k).and_then(Value::as_str);
    let valid = text("item") == Some("comment")
        && matches!(text("verb"), Some("add" | "remove" | "edited"))
        && text("comment_id").is_some()
        && text("post_id").is_some()
        && value.pointer("/from/id").is_some_and(Value::is_string)
        && value.get("created_time").is_some_and(Value::is_number);
    if !valid {
        tracing::warn!("Unrecognized feed webhook payload");
        return None;
    }
    let from = value.pointer("/from/id").and_then(Value::as_str);
    let comment_id = text("comment_id");
    match text("verb") {
        Some("add") if from != Some(page_id) => Some(job(
            "incomingComment",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "commentData": {
                    "commentId": comment_id,
                    "postId": text("post_id"),
                    "parentId": text("parent_id"),
                    "fromId": from,
                    "fromName": value.pointer("/from/name"),
                    "message": text("message"),
                    "videoUrl": text("video"),
                    "tags": value.get("message_tags").and_then(Value::as_array).map(|tags| tags.iter()
                        .map(|t| json!({ "id": t.get("id"), "name": t.get("name") })).collect::<Vec<_>>()),
                    "createdTime": value.get("created_time"),
                },
            }),
        )),
        Some("edited") if from != Some(page_id) => Some(job(
            "updateIncomingComment",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "commentId": comment_id,
                "newText": text("message").unwrap_or(""),
            }),
        )),
        Some("remove") => Some(job(
            "deleteIncomingComment",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "commentId": comment_id,
            }),
        )),
        _ => None,
    }
}

/// One `messaging[]` event (reshaped to a single-event payload).
fn messaging_job(page_id: &str, time: &Value, item: &Value) -> Option<WebhookJob> {
    let single = json!({ "object": "page", "entry": [{ "id": page_id, "time": time, "messaging": [item] }] });
    let sender = id_of(item, "/sender/id")?;
    if item.get("read").is_some_and(|r| !r.is_null()) {
        return Some(job(
            "contactMarkAsRead",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "sourceConversationId": sender,
                "payload": single,
            }),
        ));
    }
    if let Some(reaction) = item.get("reaction").filter(|r| r.is_object()) {
        return Some(job(
            "messageReaction",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "messageId": reaction.get("mid"),
                "action": reaction.get("action"),
                "emoji": reaction.get("emoji"),
                "contactSourceId": sender,
            }),
        ));
    }
    let message = item.get("message").filter(|m| m.is_object());
    if let Some(m) = message
        && m.get("is_deleted") == Some(&json!(true))
    {
        return Some(job(
            "deleteIncomingMessage",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": page_id,
                "messageId": m.get("mid"),
            }),
        ));
    }
    let postback = item.get("postback").filter(|p| p.is_object());
    if message.is_none() && postback.is_none() && item.get("referral").is_none_or(Value::is_null) {
        return None;
    }
    let is_echo = message.is_some_and(|m| m.get("is_echo") == Some(&json!(true)));
    let identifier = if is_echo {
        sender
    } else {
        id_of(item, "/recipient/id")?
    };
    if let Some(p) = postback {
        return Some(job(
            "incomingMessage",
            json!({
                "integrationType": "messenger",
                "integrationIdentifier": identifier,
                "payload": single,
                "action": p.get("payload"),
            }),
        ));
    }
    if is_echo
        && message
            .and_then(|m| m.get("metadata"))
            .and_then(Value::as_str)
            == Some(MESSAGE_METADATA)
    {
        return None;
    }
    let name = if is_echo {
        "incomingMessageEcho"
    } else {
        "incomingMessage"
    };
    Some(job(
        name,
        json!({
            "integrationType": "messenger",
            "integrationIdentifier": identifier,
            "payload": single,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_needs_one_equals_sign_and_the_exact_body() {
        let body = r#"{"object":"page"}"#;
        let good = format!("sha256={}", hmac_sha256_hex("secret", body));
        assert!(verify_signature(body, &good, "secret"));
        assert!(!verify_signature(body, &good, "other"));
        assert!(!verify_signature(
            r#"{ "object": "page" }"#,
            &good,
            "secret"
        ));
        assert!(!verify_signature(body, &format!("{good}=x"), "secret"));
        assert!(!verify_signature(body, "", "secret"));
    }

    #[test]
    fn handshake_needs_mode_token_and_challenge() {
        let ok = handshake(
            Some("hub.mode=subscribe&hub.verify_token=t&hub.challenge=abc"),
            Some("t"),
        );
        assert_eq!(ok, Ok("abc".into()));
        assert!(
            handshake(
                Some("hub.mode=subscribe&hub.verify_token=x&hub.challenge=abc"),
                Some("t")
            )
            .is_err()
        );
        assert!(
            handshake(
                Some("hub.mode=subscribe&hub.verify_token=t&hub.challenge="),
                Some("t")
            )
            .is_err()
        );
        assert!(
            handshake(
                Some("hub.mode=unsubscribe&hub.verify_token=t&hub.challenge=a"),
                Some("t")
            )
            .is_err()
        );
    }

    #[test]
    fn unsafe_ids_are_quoted() {
        let body = r#"{"message_id":120254191797760180,"x":[{"messenger_subscription_token": 1234567890123456789}]}"#;
        let quoted = quote_unsafe_message_ids(body);
        let v: Value = serde_json::from_str(&quoted).unwrap();
        assert_eq!(v["message_id"], "120254191797760180");
        assert_eq!(
            v["x"][0]["messenger_subscription_token"],
            "1234567890123456789"
        );
        assert_eq!(
            quote_unsafe_message_ids(r#"{"message_id":12}"#),
            r#"{"message_id":12}"#
        );
    }

    fn delivery(messaging: Value) -> String {
        json!({ "object": "page", "entry": [{ "id": "page-1", "time": 1, "messaging": messaging }] }).to_string()
    }

    fn names(jobs: &[WebhookJob]) -> Vec<&'static str> {
        jobs.iter().map(|j| j.name).collect()
    }

    #[test]
    fn every_messaging_event_becomes_one_job() {
        let ev = |extra: Value| {
            let mut e = json!({ "sender": { "id": "psid" }, "recipient": { "id": "page-1" }, "timestamp": 5 });
            e.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            e
        };
        let body = delivery(json!([
            ev(json!({ "message": { "mid": "m1", "text": "a" } })),
            ev(json!({ "message": { "mid": "m2", "text": "b" } })),
            ev(json!({ "read": { "watermark": 1 } })),
            ev(json!({ "reaction": { "mid": "m1", "action": "react", "emoji": "x" } })),
            ev(json!({ "message": { "mid": "m3", "is_deleted": true } })),
            ev(json!({ "postback": { "mid": "p", "title": "T", "payload": "P" } })),
            ev(json!({ "delivery": { "mids": ["m"], "watermark": 1 } })),
            { "sender": { "id": "page-1" }, "recipient": { "id": "psid" }, "timestamp": 5, "message": { "mid": "e1", "is_echo": true, "metadata": MESSAGE_METADATA } },
            { "sender": { "id": "page-1" }, "recipient": { "id": "psid" }, "timestamp": 5, "message": { "mid": "e2", "is_echo": true } },
            { "sender": { "id": "psid" }, "recipient": { "id": "page-1" }, "pass_thread_control": {} },
            { "field": "marketing_message_delivery", "value": {} },
        ]));
        let jobs = webhook_jobs(&body).unwrap();
        assert_eq!(
            names(&jobs),
            vec![
                "incomingMessage",
                "incomingMessage",
                "contactMarkAsRead",
                "messageReaction",
                "deleteIncomingMessage",
                "incomingMessage",
                "incomingMessageEcho",
                "processMarketingMessageWebhook"
            ]
        );
        assert_eq!(
            jobs[0].data["data"]["payload"]["entry"][0]["messaging"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(jobs[5].data["data"]["action"], "P");
        // An echo is routed by its sender (the page).
        assert_eq!(jobs[6].data["data"]["integrationIdentifier"], "page-1");
    }

    #[test]
    fn feed_labels_and_leadgen_changes() {
        let body = json!({ "object": "page", "entry": [
            { "id": "page-1", "time": 1, "changes": [
                { "field": "feed", "value": { "item": "comment", "verb": "add", "comment_id": "c1", "post_id": "p1", "from": { "id": "u1", "name": "U" }, "message": "hi", "created_time": 1 } },
                { "field": "feed", "value": { "item": "comment", "verb": "add", "comment_id": "c2", "post_id": "p1", "from": { "id": "page-1" }, "created_time": 1 } },
                { "field": "feed", "value": { "item": "comment", "verb": "edited", "comment_id": "c1", "post_id": "p1", "from": { "id": "u1" }, "message": "new", "created_time": 2 } },
                { "field": "feed", "value": { "item": "comment", "verb": "remove", "comment_id": "c1", "post_id": "p1", "from": { "id": "u1" }, "created_time": 3 } },
            ] },
            { "id": "page-1", "time": 1, "changes": [{ "field": "inbox_labels", "value": { "action": "add", "label": { "id": "l" } } }] },
            { "id": "page-1", "time": 1, "changes": [{ "field": "leadgen", "value": { "leadgen_id": "lg", "form_id": "f", "page_id": "page-1", "ad_id": "ad" } }] },
            { "time": 1 },
        ]})
        .to_string();
        let jobs = webhook_jobs(&body).unwrap();
        assert_eq!(
            names(&jobs),
            vec![
                "incomingComment",
                "updateIncomingComment",
                "deleteIncomingComment",
                "channelLabelChange",
                "processLeadgen"
            ]
        );
        assert_eq!(jobs[0].data["data"]["commentData"]["commentId"], "c1");
        assert_eq!(jobs[4].data["data"]["adgroupId"], "ad");
    }

    #[test]
    fn a_non_page_object_queues_nothing() {
        assert!(
            webhook_jobs(r#"{"object":"instagram","entry":[]}"#)
                .unwrap()
                .is_empty()
        );
        assert!(webhook_jobs("not json").is_err());
    }

    #[test]
    fn errors_carry_the_nested_original_text() {
        let missing = handle_request(false, None, None, b"{}", None, "s").unwrap_err();
        assert_eq!(
            missing,
            "Webhook error: Webhook processing failed: Webhook error: Failed to process webhook event: Webhook error: Missing webhook signature"
        );
        let get = handle_request(true, Some("hub.mode=x"), None, b"", Some("t"), "s").unwrap_err();
        assert_eq!(
            get,
            "Webhook error: Webhook processing failed: Webhook error: Invalid webhook verification parameters"
        );
        let body = br#"{"object":"page","entry":[]}"#;
        let sig = format!(
            "sha256={}",
            hmac_sha256_hex("s", std::str::from_utf8(body).unwrap())
        );
        assert!(
            matches!(handle_request(false, None, Some(&sig), body, None, "s"), Ok(Outcome::Jobs(j)) if j.is_empty())
        );
    }
}
