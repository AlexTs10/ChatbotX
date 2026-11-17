//! The WhatsApp webhooks (TRACKER P1.20): the manual per-integration route
//! (`apps/builder/src/app/integrations/whatsapp/webhook/[integrationId]/route.ts`)
//! and the shared part of `handlers/webhook.ts`: the `X-Hub-Signature-256`
//! policy, the split of a `messages` change into one item each, and the
//! `incomingMessage` / `messageStatus` / `messageReaction` jobs with their
//! deterministic job ids. The HTTP layer stays in the server crate.

use std::sync::Arc;

use chatbotx_core::utils::crypto::{VerifyHmacSha256SignatureInput, verify_hmac_sha256_signature};
use chatbotx_db::Db;
use chatbotx_db::repo::integration_whatsapp as repo;
use chatbotx_db::repo::{Json, iso, now_ms};
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use serde_json::{Value, json};

/// `HUB_SIGNATURE_HEADER`.
pub const HUB_SIGNATURE_HEADER: &str = "x-hub-signature-256";
/// `MAX_CHALLENGE_LENGTH`.
const MAX_CHALLENGE_LENGTH: usize = 256;

/// One HTTP answer of the webhook route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookReply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

fn json_reply(status: u16, message: &str) -> WebhookReply {
    WebhookReply {
        status,
        content_type: "application/json",
        body: json!({ "message": message }).to_string(),
    }
}

/// `toBullMqSafeIdSegment`.
pub fn safe_id_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `resolveSignaturePolicy` + `verifyPostSignature`: `Ok(())` = accepted.
/// A manual integration without an app secret is accepted unverified; any
/// other integration needs a valid `sha256=` HMAC of the exact raw body.
pub fn verify_signature(
    client_secret: Option<&str>,
    manual_integration: bool,
    signature_header: Option<&str>,
    raw_body: &[u8],
) -> Result<(), &'static str> {
    let secret = client_secret.filter(|s| !s.is_empty());
    let Some(secret) = secret else {
        if manual_integration {
            tracing::warn!(
                "Whatsapp webhook accepted unverified: manual integration has no app secret configured"
            );
            return Ok(());
        }
        return Err("missing-secret");
    };
    let Some(header) = signature_header else {
        return Err("missing-signature");
    };
    if verify_hmac_sha256_signature(VerifyHmacSha256SignatureInput {
        raw_body,
        secret,
        signature_header: Some(header),
        prefix: None,
    }) {
        Ok(())
    } else {
        Err("invalid-signature")
    }
}

/// One queued job of a webhook delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookJob {
    pub name: &'static str,
    pub data: Value,
    pub job_id: String,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// `pickContactsForMessage`: the contact of one message, by identity.
fn contact_for(contacts: Option<&Vec<Value>>, message: &Value, index: usize) -> Option<Value> {
    let contacts = contacts.filter(|c| !c.is_empty())?;
    let from = text(message, "from");
    let from_user = text(message, "from_user_id");
    let matched = contacts.iter().find(|c| {
        (from.is_some() && text(c, "wa_id") == from)
            || (from_user.is_some() && text(c, "user_id") == from_user)
    });
    if let Some(found) = matched {
        return Some(found.clone());
    }
    if from.is_none() && from_user.is_none() {
        return if contacts.len() == 1 {
            contacts.first().cloned()
        } else {
            contacts.get(index).cloned()
        };
    }
    None
}

/// `buildMessagesChangeBuffers` + `splitMessagesChangeValue` +
/// `whatsapp-api-js` `post()` + `dispatchWebhookResult`: the jobs of one
/// delivery. Items for another number than `pinned` are dropped.
pub fn webhook_jobs(body: &Value, pinned: Option<&str>) -> Vec<WebhookJob> {
    let mut jobs = Vec::new();
    let Some(object) = body.get("object").filter(|o| !o.is_null()) else {
        return jobs;
    };
    for entry in body
        .get("entry")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for change in entry
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if change.get("field").and_then(Value::as_str) != Some("messages") {
                continue;
            }
            let Some(value) = change.get("value").filter(|v| v.is_object()) else {
                continue;
            };
            let phone_id = value
                .pointer("/metadata/phone_number_id")
                .and_then(Value::as_str);
            if let Some(pinned) = pinned
                && phone_id != Some(pinned)
            {
                tracing::warn!(pinned, received = ?phone_id, "Whatsapp webhook change dropped: phone_number_id does not match the route-loaded integration");
                continue;
            }
            let Some(phone_id) = phone_id else { continue };
            let single_body = |single: Value| json!({ "object": object, "entry": [{ "id": entry.get("id"), "changes": [{ "field": "messages", "value": single }] }] });
            let contacts = value.get("contacts").and_then(Value::as_array);
            let messages: Vec<&Value> = value
                .get("messages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|m| m.get("type").and_then(Value::as_str) != Some("system"))
                .collect();
            for (index, message) in messages.into_iter().enumerate() {
                let contact = contact_for(contacts, message, index).map(|mut c| {
                    if !c.get("profile").is_some_and(Value::is_object) {
                        c["profile"] = json!({});
                    }
                    c
                });
                let mut single = value.clone();
                single["messages"] = json!([message]);
                if let Some(map) = single.as_object_mut() {
                    map.remove("statuses");
                    match &contact {
                        Some(c) => {
                            map.insert("contacts".into(), json!([c]));
                        }
                        None => {
                            map.remove("contacts");
                        }
                    }
                }
                let from = contact
                    .as_ref()
                    .and_then(|c| text(c, "wa_id"))
                    .or_else(|| text(message, "from"));
                let name = contact.as_ref().and_then(|c| {
                    c.pointer("/profile/name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
                let message_id = text(message, "id").unwrap_or_default();
                if message.get("type").and_then(Value::as_str) == Some("reaction") {
                    let reaction = &message["reaction"];
                    let (Some(target), Some(contact_source)) = (
                        text(reaction, "message_id").filter(|t| !t.is_empty()),
                        from.clone()
                            .or_else(|| contact.as_ref().and_then(|c| text(c, "user_id"))),
                    ) else {
                        continue;
                    };
                    let emoji = text(reaction, "emoji").filter(|e| !e.is_empty());
                    let mut data = json!({
                        "integrationType": "whatsapp",
                        "integrationIdentifier": phone_id,
                        "messageId": target,
                        "action": if emoji.is_some() { "react" } else { "unreact" },
                        "contactSourceId": contact_source,
                    });
                    if let Some(emoji) = emoji {
                        data["emoji"] = json!(emoji);
                    }
                    jobs.push(WebhookJob {
                        name: "messageReaction",
                        data: json!({ "type": "messageReaction", "data": data }),
                        job_id: format!(
                            "wa-reaction-{}-{}",
                            safe_id_segment(phone_id),
                            safe_id_segment(&message_id)
                        ),
                    });
                    continue;
                }
                let payload = json!({
                    "phoneID": phone_id,
                    "from": from,
                    "message": message,
                    "name": name,
                    "raw": single_body(single),
                });
                jobs.push(WebhookJob {
                    name: "incomingMessage",
                    data: json!({ "type": "incomingMessage", "data": {
                        "integrationType": "whatsapp",
                        "integrationIdentifier": phone_id,
                        "payload": payload,
                    }}),
                    job_id: format!(
                        "wa-msg-{}-{}",
                        safe_id_segment(phone_id),
                        safe_id_segment(&message_id)
                    ),
                });
            }
            for status in value
                .get("statuses")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = text(status, "status").unwrap_or_default();
                if !matches!(kind.as_str(), "delivered" | "failed" | "read") {
                    continue;
                }
                let id = text(status, "id").unwrap_or_default();
                // ponytail: the `continuesFlow` ingress stamp
                // (`messageStatusBranchService`) is not ported.
                jobs.push(WebhookJob {
                    name: "messageStatus",
                    data: json!({ "type": "messageStatus", "data": {
                        "integrationIdentifier": phone_id,
                        "integrationType": "whatsapp",
                        "payload": {
                            "phoneID": phone_id,
                            "phone": status.get("recipient_id"),
                            "recipientUserId": status.get("recipient_user_id"),
                            "messageId": id,
                            "status": kind,
                            "timestamp": status.get("timestamp"),
                            "error": status.pointer("/errors/0"),
                        },
                    }}),
                    job_id: format!(
                        "wa-status-{}-{}-{}",
                        safe_id_segment(phone_id),
                        safe_id_segment(&id),
                        safe_id_segment(&kind)
                    ),
                });
            }
        }
    }
    // ponytail: coexist history, automatic events, calls, identity changes,
    // account updates and thread-control routing are not ported yet.
    jobs
}

/// Queues the jobs of one verified delivery (`REDELIVERABLE_JOB_OPTIONS`).
pub async fn enqueue_webhook_jobs(
    jobs: &dyn Enqueuer,
    body: &[u8],
    pinned: Option<&str>,
) -> Result<(), String> {
    let Ok(parsed) = serde_json::from_slice::<Value>(body) else {
        tracing::error!("Whatsapp webhook body is not JSON");
        return Ok(());
    };
    for job in webhook_jobs(&parsed, pinned) {
        jobs.add(
            QueueName::Integration,
            job.name,
            job.data,
            JobOptions {
                job_id: Some(job.job_id),
                remove_on_fail: Some(Retention::Remove),
                ..JobOptions::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// `GET|POST /integrations/whatsapp/webhook/{integrationId}` (the manual
/// integration route).
pub async fn manual_webhook(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    is_get: bool,
    integration_id: &str,
    query: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
) -> WebhookReply {
    if integration_id.is_empty() {
        return json_reply(400, "Missing integrationId");
    }
    let Ok(id) = integration_id.parse::<i64>() else {
        return json_reply(404, "Integration not found");
    };
    let row = db
        .read(move |conn| repo::find_by_id_unscoped(conn, id).map_err(crate::repo_err))
        .await;
    let Ok(Some(row)) = row else {
        return json_reply(404, "Integration not found");
    };
    let auth = row.auth.0.clone();
    if auth.pointer("/metadata/isManual") != Some(&json!(true)) {
        return json_reply(404, "Integration not found");
    }
    if is_get {
        let mode = query_param(query, "hub.mode");
        let token = query_param(query, "hub.verify_token");
        let Some(challenge) = query_param(query, "hub.challenge")
            .filter(|c| !c.is_empty() && mode.as_deref() == Some("subscribe"))
        else {
            return json_reply(400, "Invalid handshake");
        };
        if challenge.encode_utf16().count() > MAX_CHALLENGE_LENGTH {
            return json_reply(400, "Challenge too long");
        }
        if token.as_deref() != auth.get("verifyToken").and_then(Value::as_str) {
            tracing::warn!(
                integration_id,
                "Whatsapp manual webhook verify_token mismatch"
            );
            return json_reply(403, "Forbidden");
        }
        // `markWebhookVerified`.
        let mut next = auth.clone();
        if !next.get("metadata").is_some_and(Value::is_object) {
            next["metadata"] = json!({});
        }
        next["metadata"]["webhookVerifiedAt"] = json!(iso(now_ms()));
        let saved = db
            .write(move |tx| {
                repo::update_auth_unscoped(tx, id, &Json(next)).map_err(crate::repo_err)
            })
            .await;
        if let Err(err) = saved {
            tracing::error!(err = %err, "Whatsapp webhook verification could not be saved");
            return json_reply(400, &err.to_string());
        }
        return WebhookReply {
            status: 200,
            content_type: "text/plain",
            body: challenge,
        };
    }
    let verified = auth
        .pointer("/metadata/webhookVerifiedAt")
        .is_some_and(|v| !v.is_null() && v != &json!(""))
        || auth.pointer("/metadata/subscribeOverrideOk") == Some(&json!(true));
    if !verified {
        return json_reply(403, "Webhook not verified");
    }
    let secret = auth.get("clientSecret").and_then(Value::as_str);
    if let Err(reason) = verify_signature(secret, true, signature_header, body) {
        tracing::warn!(
            reason,
            "Whatsapp webhook rejected: signature verification failed"
        );
        return json_reply(401, "Whatsapp webhook signature verification failed");
    }
    match enqueue_webhook_jobs(jobs, body, Some(&row.phone_number_id)).await {
        Ok(()) => WebhookReply {
            status: 200,
            content_type: "text/plain;charset=UTF-8",
            body: "ok".into(),
        },
        Err(err) => {
            tracing::error!(err, "Whatsapp webhook handler failed");
            // `SdkException("Failed to handle webhook")` (default status 400).
            json_reply(400, "Failed to handle webhook")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, body);
        format!(
            "sha256={}",
            tag.as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }

    #[test]
    fn signature_policy_matches_the_table() {
        let body = br#"{"object":"whatsapp_business_account"}"#;
        assert_eq!(
            verify_signature(Some("s"), true, Some(&sign("s", body)), body),
            Ok(())
        );
        assert_eq!(
            verify_signature(Some("s"), false, Some(&sign("x", body)), body),
            Err("invalid-signature")
        );
        assert_eq!(
            verify_signature(Some("s"), true, None, body),
            Err("missing-signature")
        );
        assert_eq!(verify_signature(None, true, None, body), Ok(()));
        assert_eq!(
            verify_signature(None, false, None, body),
            Err("missing-secret")
        );
        // The exact bytes are signed: re-encoded JSON fails.
        let reencoded = br#"{ "object": "whatsapp_business_account" }"#;
        assert_eq!(
            verify_signature(Some("s"), true, Some(&sign("s", body)), reencoded),
            Err("invalid-signature")
        );
    }

    #[test]
    fn a_batched_change_becomes_one_job_per_item() {
        let body = json!({ "object": "whatsapp_business_account", "entry": [{ "id": "waba", "changes": [{ "field": "messages", "value": {
            "messaging_product": "whatsapp",
            "metadata": { "phone_number_id": "111", "display_phone_number": "1555" },
            "contacts": [{ "wa_id": "849", "profile": { "name": "Ada" } }, { "wa_id": "850" }],
            "messages": [
                { "id": "wamid.A", "from": "849", "type": "text", "text": { "body": "hi" } },
                { "id": "wamid.B", "from": "850", "type": "reaction", "reaction": { "message_id": "wamid.X", "emoji": "👍" } },
                { "id": "wamid.S", "type": "system", "system": {} },
            ],
            "statuses": [
                { "id": "wamid.O", "status": "read", "recipient_id": "849", "timestamp": "1700000000" },
                { "id": "wamid.P", "status": "sent", "recipient_id": "849" },
            ],
        }}]}]});
        let jobs = webhook_jobs(&body, None);
        let names: Vec<_> = jobs.iter().map(|j| (j.name, j.job_id.as_str())).collect();
        assert_eq!(
            names,
            vec![
                ("incomingMessage", "wa-msg-111-wamid.A"),
                ("messageReaction", "wa-reaction-111-wamid.B"),
                ("messageStatus", "wa-status-111-wamid.O-read"),
            ]
        );
        let payload = &jobs[0].data["data"]["payload"];
        assert_eq!(
            (payload["from"].as_str(), payload["name"].as_str()),
            (Some("849"), Some("Ada"))
        );
        assert_eq!(
            payload["raw"]["entry"][0]["changes"][0]["value"]["contacts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(jobs[1].data["data"]["action"], "react");
        // A pinned route drops another number's change.
        assert!(webhook_jobs(&body, Some("222")).is_empty());
    }
}
