//! The WhatsApp webhooks (TRACKER P1.20): the manual per-integration route
//! (`apps/builder/src/app/integrations/whatsapp/webhook/[integrationId]/route.ts`)
//! and the shared part of `handlers/webhook.ts`: the `X-Hub-Signature-256`
//! policy, the split of a `messages` change into one item each, and the
//! `incomingMessage` / `messageStatus` / `messageReaction` jobs with their
//! deterministic job ids. The HTTP layer stays in the server crate.

use std::sync::Arc;

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_core::utils::crypto::{VerifyHmacSha256SignatureInput, verify_hmac_sha256_signature};
use chatbotx_db::Db;
use chatbotx_db::repo::integration_whatsapp as repo;
use chatbotx_db::repo::models::PlatformCredential;
use chatbotx_db::repo::partials::credential::credential_aad_for;
use chatbotx_db::repo::{Json, iso, now_ms};
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use serde_json::{Map, Value, json};

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

pub fn json_reply(status: u16, message: &str) -> WebhookReply {
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

/// Who a webhook route speaks for, for its `account_update` events.
#[derive(Debug, Clone, PartialEq)]
pub enum AccountScope {
    /// `ProviderAccountRouteScope` JSON.
    Scoped(Value),
    /// The route has no app id to scope the event.
    Unscoped,
    /// The delivery was accepted without a signature.
    Unverified,
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
    // Automatic events log-and-skip, so they are queued last.
    let mut automatic = Vec::new();
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
            jobs.extend(coexist_job(change));
            if change.get("field").and_then(Value::as_str) == Some("automatic_events") {
                automatic.extend(automatic_event_jobs(entry, change.get("value"), pinned));
                continue;
            }
            if change.get("field").and_then(Value::as_str) == Some("messaging_handovers") {
                jobs.extend(handover_job(change.get("value"), pinned));
                continue;
            }
            if change.get("field").and_then(Value::as_str) == Some("standby") {
                jobs.extend(standby_jobs(
                    object,
                    entry.get("id"),
                    change.get("value"),
                    pinned,
                ));
                continue;
            }
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
            jobs.extend(identity_change_jobs(value, phone_id));
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
                // The `continuesFlow` ingress stamp only picks the queue in the
                // original; here every status stays on `integration` (DECISIONS).
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
    jobs.extend(automatic);
    jobs
}

fn trimmed_text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `normalizeUserIdChanged` / `normalizePhoneChanged` of one system
/// message: the `change` of a `whatsappIdentityChange` job.
fn identity_change(message: &Value, phone_id: &str) -> Option<Value> {
    use chatbotx_business::contact_identity::{
        is_distinct_primary_identity, resolve_phone_transition,
    };
    let system = message.get("system")?;
    let system_type = trimmed_text(system, "type")?;
    let message_id = text(message, "id").unwrap_or_default();
    let skipped = || {
        tracing::warn!(
            message_id,
            phone_id,
            "Whatsapp identity change skipped: invalid or incomplete identity transition"
        );
        None
    };
    let from_user_id = text(message, "from_user_id");
    match system_type.as_str() {
        "user_changed_user_id" => {
            let Some(user_id) = trimmed_text(system, "user_id") else {
                tracing::warn!(message_id, phone_id, "Whatsapp identity change skipped: malformed system payload");
                return None;
            };
            // `parsePreviousUserIdFromBody`: "User <name> changed from <a> to <b>."
            let from_body = text(system, "body").and_then(|body| {
                let body = body.trim().trim_end_matches('.');
                let rest = body.strip_prefix("User ")?;
                let (head, to) = rest.rsplit_once(" to ")?;
                let (_, from) = head.rsplit_once(" changed from ")?;
                (to == user_id && !from.contains(' ')).then(|| from.to_owned())
            });
            let previous_user_id = [
                trimmed_text(system, "previous_user_id"),
                from_body,
                trimmed_text(message, "from_user_id"),
            ]
            .into_iter()
            .flatten()
            .find(|c| *c != user_id);
            let previous_parent_user_id = trimmed_text(system, "previous_parent_user_id")
                .or_else(|| trimmed_text(message, "from_parent_user_id"));
            let parent_user_id = trimmed_text(system, "parent_user_id");
            let previous_phone = trimmed_text(message, "from");
            let reported = trimmed_text(system, "wa_id");
            let new_phone = is_distinct_primary_identity(
                reported.as_deref(),
                &[Some(&user_id), from_user_id.as_deref()],
            )
            .then(|| {
                resolve_phone_transition(
                    previous_phone.as_deref(),
                    reported.as_deref(),
                    &[Some(&user_id), from_user_id.as_deref()],
                )
            })
            .flatten()
            .map(|t| t.new_phone);
            if previous_user_id.is_none()
                && previous_parent_user_id.is_none()
                && previous_phone.is_none()
            {
                return skipped();
            }
            Some(json!({
                "kind": "userIdChanged",
                "previousUserId": previous_user_id,
                "userId": user_id,
                "previousParentUserId": previous_parent_user_id,
                "parentUserId": parent_user_id,
                "previousPhone": previous_phone,
                "newPhone": new_phone,
            }))
        }
        "user_changed_number" | "customer_changed_number" => {
            let (Some(reported), Some(previous_phone)) = (
                trimmed_text(system, "wa_id"),
                trimmed_text(message, "from"),
            ) else {
                tracing::warn!(message_id, phone_id, "Whatsapp identity change skipped: malformed system payload");
                return None;
            };
            let system_user_id = text(system, "user_id");
            let user_id = trimmed_text(message, "from_user_id")
                .or_else(|| trimmed_text(system, "user_id"));
            let scoped = [from_user_id.as_deref(), system_user_id.as_deref()];
            let new_phone = is_distinct_primary_identity(Some(&reported), &scoped)
                .then(|| resolve_phone_transition(Some(&previous_phone), Some(&reported), &scoped))
                .flatten()
                .map(|t| t.new_phone);
            let Some(new_phone) = new_phone else {
                return skipped();
            };
            Some(json!({
                "kind": "phoneChanged",
                "previousPhone": previous_phone,
                "newPhone": new_phone,
                "userId": user_id,
            }))
        }
        other => {
            tracing::info!(message_id, phone_id, system_type = other, "Whatsapp system message skipped: unsupported type");
            None
        }
    }
}

/// `extractIdentityChangePayloads` + `enqueueIdentityChangePayloads` of one
/// `messages` value.
fn identity_change_jobs(value: &Value, phone_id: &str) -> Vec<WebhookJob> {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| m.get("type").and_then(Value::as_str) == Some("system"))
        .filter_map(|message| {
            let message_id = trimmed_text(message, "id")?;
            let change = identity_change(message, phone_id)?;
            Some(WebhookJob {
                name: WHATSAPP_IDENTITY_CHANGE,
                data: json!({ "type": WHATSAPP_IDENTITY_CHANGE, "data": {
                    "integrationType": "whatsapp",
                    "integrationIdentifier": phone_id,
                    "payload": {
                        "phoneNumberId": phone_id,
                        "messageId": message_id,
                        "timestamp": message.get("timestamp"),
                        "change": change,
                    },
                }}),
                job_id: format!(
                    "wa-sys-{}-{}",
                    safe_id_segment(phone_id),
                    safe_id_segment(&message_id)
                ),
            })
        })
        .collect()
}

/// `IntegrationJobAction.whatsappIdentityChange`.
pub const WHATSAPP_IDENTITY_CHANGE: &str = "whatsappIdentityChange";
/// `IntegrationJobAction.providerAccountEvent`.
pub const PROVIDER_ACCOUNT_EVENT: &str = "providerAccountEvent";

/// `boundedEnumValue`: a Meta enum value, else `"unknown"`.
fn bounded_enum(value: Option<&Value>) -> String {
    let ok = |s: &str| {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && s.len() <= 64
            && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    };
    value
        .and_then(Value::as_str)
        .filter(|s| ok(s))
        .unwrap_or("unknown")
        .to_owned()
}

/// `routeScopeKey`.
fn route_scope_key(scope: &Value) -> String {
    match scope["kind"].as_str() {
        Some("credentialOwner") => format!("owner-{}", scope["ownerId"].as_str().unwrap_or("")),
        Some("manualIntegration") => {
            format!("manual-{}", scope["integrationId"].as_str().unwrap_or(""))
        }
        _ => "platform".into(),
    }
}

/// `extractAccountUpdatePayloads` + `enqueueAccountUpdatePayloads` +
/// `addRouteScopedJob`: one `providerAccountEvent` per recognised
/// `account_update` (`entry.id` is the WABA id), scoped to the route.
pub fn account_update_jobs(body: &Value, route_scope: &Value, now: i64) -> Vec<WebhookJob> {
    let mut jobs = Vec::new();
    for entry in body.get("entry").and_then(Value::as_array).into_iter().flatten() {
        let (Some(waba_id), Some(changes)) = (
            entry.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()),
            entry.get("changes").and_then(Value::as_array),
        ) else {
            continue;
        };
        if !changes
            .iter()
            .any(|c| c.get("field").and_then(Value::as_str) == Some("account_update"))
        {
            continue;
        }
        let occurred_at = entry
            .get("time")
            .and_then(Value::as_f64)
            .filter(|t| t.is_finite() && *t > 0.0)
            .map(|t| (t * 1000.0) as i64);
        let Some(occurred_at) = occurred_at else {
            tracing::warn!(waba_id, "Whatsapp account_update skipped: no entry time");
            continue;
        };
        if occurred_at > now + 5 * 60_000 {
            tracing::warn!(waba_id, "Whatsapp account_update skipped: time is in the future");
            continue;
        }
        for change in changes {
            if change.get("field").and_then(Value::as_str) != Some("account_update") {
                continue;
            }
            let Some(meta_event) = change.pointer("/value/event").and_then(Value::as_str) else {
                tracing::warn!(waba_id, "Whatsapp account_update skipped: malformed payload");
                continue;
            };
            let event = match meta_event {
                "PARTNER_REMOVED" => "revoked",
                "ACCOUNT_OFFBOARDED" => "offboarded",
                "ACCOUNT_RECONNECTED" => "reconnected",
                other => {
                    tracing::info!(waba_id, meta_event = other, "Whatsapp account_update ignored: event not handled");
                    continue;
                }
            };
            let info = change
                .pointer("/value/disconnection_info")
                .filter(|v| v.is_object());
            let reason = bounded_enum(info.and_then(|i| i.get("reason")));
            let initiated_by = bounded_enum(info.and_then(|i| i.get("initiated_by")));
            // `JSON.stringify(payload)` key order.
            let hashed = format!(
                r#"{{"integrationType":"whatsapp","account":{{"kind":"businessAccount","id":{}}},"event":"{event}","detail":{{"reason":"{reason}","initiatedBy":"{initiated_by}"}},"occurredAt":{occurred_at}}}"#,
                Value::from(waba_id)
            );
            let job_id = format!(
                "provider-account-whatsapp-{}-{event}-{occurred_at}-{}-{}",
                safe_id_segment(waba_id),
                chatbotx_core::utils::crypto::sha256_hex(&hashed),
                safe_id_segment(&route_scope_key(route_scope))
            );
            jobs.push(WebhookJob {
                name: PROVIDER_ACCOUNT_EVENT,
                data: json!({ "type": PROVIDER_ACCOUNT_EVENT, "data": {
                    "integrationType": "whatsapp",
                    "account": { "kind": "businessAccount", "id": waba_id },
                    "event": event,
                    "detail": { "reason": reason, "initiatedBy": initiated_by },
                    "occurredAt": occurred_at,
                    "routeScope": route_scope,
                }}),
                job_id,
            });
        }
    }
    jobs
}

/// `automaticEventFieldExtractors.automatic_events` +
/// `enqueueAutomaticEventPayloads`: one `adsAutomaticEvent` per valid event
/// (`entry.id` is the WABA id).
fn automatic_event_jobs(
    entry: &Value,
    value: Option<&Value>,
    pinned: Option<&str>,
) -> Vec<WebhookJob> {
    let (Some(waba_id), Some(value)) = (entry.get("id").and_then(Value::as_str), value) else {
        return Vec::new();
    };
    let phone = value
        .pointer("/metadata/phone_number_id")
        .and_then(Value::as_str)
        .map(|p| chatbotx_core::js::trim(p).to_owned())
        .filter(|p| !p.is_empty());
    let (Some(phone), Some(events)) = (
        phone,
        value.get("automatic_events").and_then(Value::as_array),
    ) else {
        tracing::warn!("Whatsapp automatic event skipped: malformed payload");
        return Vec::new();
    };
    if pinned.is_some_and(|p| p != phone) {
        tracing::warn!(
            pinned,
            received = phone,
            "Whatsapp webhook change dropped: phone_number_id does not match the route-loaded integration"
        );
        return Vec::new();
    }
    let mut jobs = Vec::new();
    for event in events {
        let parsed = match chatbotx_business::ads::conversion::AutomaticPayload::parse(event) {
            Ok(parsed) => parsed,
            Err(reason) => {
                tracing::warn!(reason, "Whatsapp automatic event skipped");
                continue;
            }
        };
        jobs.push(WebhookJob {
            name: "adsAutomaticEvent",
            data: json!({ "type": "adsAutomaticEvent", "data": {
                "integrationType": "whatsapp",
                "integrationIdentifier": phone,
                "phoneNumberId": phone,
                "wabaId": waba_id,
                "payload": event,
            }}),
            job_id: format!(
                "ads-auto-{}-{}",
                safe_id_segment(&phone),
                safe_id_segment(&parsed.id)
            ),
        });
    }
    jobs
}

/// `extractCoexistPayloads` + `enqueueCoexistPayloads`: a history,
/// state-sync or echo slice goes whole to `coexistWhatsappBuffer` (not
/// pinned: the buffer checks the integration).
///
/// ponytail: the hash is of serde_json's key order, as in [`handover_job`].
fn coexist_job(change: &Value) -> Option<WebhookJob> {
    let value = change.get("value").filter(|v| v.is_object())?;
    let field_key = match change.get("field").and_then(Value::as_str) {
        Some("history") => Some("history"),
        Some("smb_app_state_sync") => Some("state_sync"),
        Some("smb_message_echoes") => Some("message_echoes"),
        _ => None,
    };
    let is_array = |key: &str| value.get(key).is_some_and(Value::is_array);
    if !(field_key.is_some_and(is_array) || is_array("history") || is_array("smb_app_state_sync")) {
        return None;
    }
    let phone = value
        .pointer("/metadata/phone_number_id")
        .and_then(Value::as_str)?;
    Some(WebhookJob {
        name: crate::coexist_whatsapp::BUFFER_JOB,
        data: json!({ "type": crate::coexist_whatsapp::BUFFER_JOB, "data": { "phoneNumberId": phone, "payload": value } }),
        job_id: format!(
            "wa-coexist-{}-{}",
            safe_id_segment(phone),
            chatbotx_core::utils::crypto::sha256_hex(&value.to_string())
        ),
    })
}

/// `splitHandoverChange` + `enqueueConversationRoutingPayloads`: one
/// `threadControlEvent` job per `messaging_handovers` value, keyed by its body hash.
///
/// ponytail: the hash is of serde_json's key order (not `JSON.stringify`'s), so a job
/// id differs from the TS one; it is still stable across Meta redeliveries.
fn handover_job(value: Option<&Value>, pinned: Option<&str>) -> Option<WebhookJob> {
    let value = value.filter(|v| v.is_object())?;
    let Some(phone_id) = value
        .pointer("/recipient/phone_number_id")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    else {
        tracing::warn!("Whatsapp messaging_handovers dropped: recipient.phone_number_id missing");
        return None;
    };
    if pinned.is_some_and(|pinned| pinned != phone_id) {
        tracing::warn!(
            pinned,
            received = phone_id,
            "Whatsapp conversationRouting item dropped: phone_number_id does not match the route-loaded integration"
        );
        return None;
    }
    let hash = chatbotx_core::utils::crypto::sha256_hex(&value.to_string());
    Some(WebhookJob {
        name: "threadControlEvent",
        data: json!({ "type": "threadControlEvent", "data": {
            "integrationType": "whatsapp",
            "integrationIdentifier": phone_id,
            "payload": { "kind": "handover", "body": value },
        }}),
        job_id: format!("wa-tc-{}-{hash}", safe_id_segment(phone_id)),
    })
}

/// `splitStandbyChange`: one `threadControlEvent` job per standby message
/// (`wa-sb-`) and echo (`wa-sbe-`), each a single-item `receiveMessage` body that
/// keeps `field: "standby"`. Standby statuses are dropped.
fn standby_jobs(
    object: &Value,
    entry_id: Option<&Value>,
    value: Option<&Value>,
    pinned: Option<&str>,
) -> Vec<WebhookJob> {
    let mut jobs = Vec::new();
    let value = value.filter(|v| v.is_object());
    let phone_id = value
        .and_then(|v| v.pointer("/metadata/phone_number_id"))
        .and_then(Value::as_str);
    let standby = value
        .and_then(|v| v.get("standby"))
        .filter(|s| s.is_object());
    let (Some(value), Some(phone_id), Some(standby)) = (value, phone_id, standby) else {
        tracing::warn!("Whatsapp standby change dropped: malformed payload");
        return jobs;
    };
    if pinned.is_some_and(|pinned| pinned != phone_id) {
        tracing::warn!(
            pinned,
            received = phone_id,
            "Whatsapp conversationRouting item dropped: phone_number_id does not match the route-loaded integration"
        );
        return jobs;
    }
    // `buildStandbyRawBody`.
    let raw = |part: Value, contacts: Option<&Value>| {
        let mut single = value.clone();
        let mut standby_part = part;
        if let Some(c) = contacts {
            single["contacts"] = c.clone();
            standby_part["contacts"] = c.clone();
        }
        single["standby"] = standby_part;
        json!({ "object": object, "entry": [{ "id": entry_id, "changes": [{ "field": "standby", "value": single }] }] })
    };
    let job = |kind: &str, prefix: &str, id: &str, payload: Value| WebhookJob {
        name: "threadControlEvent",
        data: json!({ "type": "threadControlEvent", "data": {
            "integrationType": "whatsapp",
            "integrationIdentifier": phone_id,
            "payload": { "kind": kind, "body": payload },
        }}),
        job_id: format!(
            "{prefix}-{}-{}",
            safe_id_segment(phone_id),
            safe_id_segment(id)
        ),
    };
    let contact_list = standby.get("contacts").and_then(Value::as_array);
    for (index, message) in standby
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(id) = text(message, "id") else {
            tracing::warn!("Whatsapp standby message dropped: message id missing");
            continue;
        };
        let contacts = contact_for(contact_list, message, index).map(|c| json!([c]));
        let from = text(message, "from")
            .or_else(|| contacts.as_ref().and_then(|c| text(&c[0], "wa_id")))
            .unwrap_or_default();
        let name = contacts
            .as_ref()
            .and_then(|c| c.pointer("/0/profile/name"))
            .and_then(Value::as_str);
        jobs.push(job(
            "standbyMessage",
            "wa-sb",
            &id,
            json!({
                "phoneID": phone_id,
                "from": from,
                "message": message,
                "name": name,
                "raw": raw(json!({ "messages": [message] }), contacts.as_ref()),
            }),
        ));
    }
    for echo in standby
        .get("message_echoes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(sent)) = (
            text(echo, "id"),
            echo.get("message").filter(|m| m.is_object()),
        ) else {
            tracing::warn!("Whatsapp standby echo dropped: id or message missing");
            continue;
        };
        let from = text(sent, "to")
            .or_else(|| text(sent, "recipient"))
            .unwrap_or_default();
        jobs.push(job(
            "standbyEcho",
            "wa-sbe",
            &id,
            json!({
                "phoneID": phone_id,
                "from": from,
                "message": echo,
                "raw": raw(json!({ "message_echoes": [echo] }), None),
            }),
        ));
    }
    jobs
}

/// Queues the jobs of one verified delivery (`REDELIVERABLE_JOB_OPTIONS`).
pub async fn enqueue_webhook_jobs(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    body: &[u8],
    pinned: Option<&str>,
    account_scope: AccountScope,
) -> Result<(), String> {
    let Ok(parsed) = serde_json::from_slice::<Value>(body) else {
        tracing::error!("Whatsapp webhook body is not JSON");
        return Ok(());
    };
    // Account events change connection state: only from a verified delivery
    // whose route can say who it speaks for. Failures propagate (redelivery).
    let account_jobs = match &account_scope {
        AccountScope::Scoped(scope) => account_update_jobs(&parsed, scope, now_ms()),
        _ => Vec::new(),
    };
    if account_jobs.is_empty()
        && !matches!(account_scope, AccountScope::Scoped(_))
        && !account_update_jobs(&parsed, &Value::Null, now_ms()).is_empty()
    {
        match account_scope {
            AccountScope::Unverified => tracing::warn!(
                "Whatsapp account_update ignored: the webhook is not signature-verified"
            ),
            _ => tracing::error!(
                "Provider account event dropped: the webhook credential has no app id to scope it"
            ),
        }
    }
    for job in account_jobs {
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
    // Call events first (they propagate and are jobId-deduped).
    for job in crate::whatsapp_call_webhook::call_jobs(&parsed, pinned) {
        jobs.add(QueueName::Integration, job.name, job.data, job.opts)
            .await
            .map_err(|e| e.to_string())?;
    }
    // Then the VoIP connect signalling (`enqueueVoipConnectSignaling`).
    crate::whatsapp_call_webhook::capture_voip_signals(db, jobs, &parsed, pinned).await?;
    for job in webhook_jobs(&parsed, pinned) {
        let (name, event_id) = (job.name, job.data.pointer("/data/payload/id").cloned());
        let added = jobs
            .add(
                QueueName::Integration,
                job.name,
                job.data,
                JobOptions {
                    job_id: Some(job.job_id),
                    remove_on_fail: Some(Retention::Remove),
                    ..JobOptions::default()
                },
            )
            .await;
        match added {
            Ok(_) => {}
            // An automatic event is logged and skipped; the webhook still acknowledges.
            Err(err) if name == "adsAutomaticEvent" => tracing::error!(
                %err,
                event_id = ?event_id,
                "Whatsapp automatic event enqueue failed; webhook will still acknowledge"
            ),
            Err(err) => return Err(err.to_string()),
        }
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
    let secret = auth
        .get("clientSecret")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let account_scope = if secret.is_some() {
        AccountScope::Scoped(json!({
            "kind": "manualIntegration",
            "integrationId": row.id.to_string(),
            "workspaceId": row.workspace_id.to_string(),
        }))
    } else {
        AccountScope::Unverified
    };
    if let Err(reason) = verify_signature(secret, true, signature_header, body) {
        tracing::warn!(
            reason,
            "Whatsapp webhook rejected: signature verification failed"
        );
        return json_reply(401, "Whatsapp webhook signature verification failed");
    }
    match enqueue_webhook_jobs(db, jobs, body, Some(&row.phone_number_id), account_scope).await {
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

/// `platformCredentialService.findDecryptedPlatform({ type })`: the
/// decrypted config of a platform app (`whatsapp`, `zalo`).
// ponytail: the minimum of Q.D11's `platform_credential` service (no config
// schema parse, no tenant / credential-scoped lookups).
pub async fn find_platform_config(
    db: &Arc<Db>,
    encryption: &Encryption,
    kind: &'static str,
) -> Option<Map<String, Value>> {
    let row = db
        .read(move |conn| {
            chatbotx_db::repo::find_one::<PlatformCredential>(
                conn,
                r#""userId" IS NULL AND "type" = ?1"#,
                &[&kind],
            )
            .map_err(crate::repo_err)
        })
        .await
        .ok()??;
    let blob: EncryptedData = serde_json::from_value(row.value.0).ok()?;
    let aad = credential_aad_for(None, kind);
    match encryption.decrypt_object::<Map<String, Value>>(&blob, Some(&aad)) {
        Ok(config) => Some(config),
        Err(error) => {
            tracing::error!(%error, kind, "Platform credential could not be decrypted");
            None
        }
    }
}

/// The request facts the shared route needs.
pub struct PlatformRequest<'a> {
    pub is_get: bool,
    pub query: Option<&'a str>,
    pub signature_header: Option<&'a str>,
    /// `x-domain` (cloud only).
    pub domain: Option<&'a str>,
    pub body: &'a [u8],
}

/// `GET|POST /integrations/whatsapp/webhook` (`[...integration]/webhook.ts`
/// for `whatsapp` + `handlers/webhook.ts`): the platform app's webhook. It
/// pins no phone number and always enforces the signature.
pub async fn platform_webhook(
    db: &Arc<Db>,
    jobs: &dyn Enqueuer,
    encryption: Option<&Encryption>,
    is_cloud: bool,
    broker_host: &str,
    request: PlatformRequest<'_>,
) -> WebhookReply {
    let not_configured = || json_reply(404, "Integration is not configured");
    // ponytail: a tenant custom domain (`customDomainService` +
    // `findDecryptedForUser`) and the credential-scoped `/c/{id}` route are
    // not ported; only the broker host speaks for the platform in cloud.
    if is_cloud && request.domain.unwrap_or("") != broker_host {
        tracing::debug!(domain = ?request.domain, "No active custom domain for integration webhook");
        return not_configured();
    }
    let Some(encryption) = encryption else {
        return not_configured();
    };
    let Some(config) = find_platform_config(db, encryption, "whatsapp").await else {
        tracing::debug!("Integration whatsapp is not configured");
        return not_configured();
    };
    let text = |key: &str| {
        config
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    if request.is_get {
        // `whatsapp-api-js` `get()`; its errors are not `SdkException`s, so
        // the route answers 400 with their message.
        let Some(verify_token) = text("verifyToken") else {
            return json_reply(400, "Missing verify token");
        };
        let mode = query_param(request.query, "hub.mode").filter(|m| !m.is_empty());
        let token = query_param(request.query, "hub.verify_token").filter(|t| !t.is_empty());
        let (Some(mode), Some(token)) = (mode, token) else {
            return json_reply(400, "Missing search params");
        };
        if mode != "subscribe" || token != verify_token {
            return json_reply(400, "Invalid token verification");
        }
        return WebhookReply {
            status: 200,
            content_type: "text/plain;charset=UTF-8",
            body: query_param(request.query, "hub.challenge").unwrap_or_default(),
        };
    }
    if let Err(reason) = verify_signature(
        text("clientSecret"),
        false,
        request.signature_header,
        request.body,
    ) {
        tracing::warn!(
            reason,
            "Whatsapp webhook rejected: signature verification failed"
        );
        return json_reply(401, "Whatsapp webhook signature verification failed");
    }
    let account_scope = match text("clientId") {
        Some(app_id) => AccountScope::Scoped(json!({ "kind": "platform", "appId": app_id })),
        None => AccountScope::Unscoped,
    };
    match enqueue_webhook_jobs(db, jobs, request.body, None, account_scope).await {
        Ok(()) => WebhookReply {
            status: 200,
            content_type: "text/plain;charset=UTF-8",
            body: "ok".into(),
        },
        Err(err) => {
            tracing::error!(err, "Whatsapp webhook handler failed");
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

    #[tokio::test]
    async fn platform_route_reads_the_platform_credential() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        db.migrate().await.unwrap();
        let jobs = crate::steps::test_support::Jobs::default();
        let encryption =
            Encryption::new(chatbotx_core::encryption::Keys::new(&"ab".repeat(32)).unwrap());
        let request = |is_get, query, signature, body| PlatformRequest {
            is_get,
            query,
            signature_header: signature,
            domain: None,
            body,
        };
        // No credential yet.
        let reply = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            false,
            "",
            request(true, None, None, b""),
        )
        .await;
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (404, r#"{"message":"Integration is not configured"}"#)
        );

        let blob = encryption
            .encrypt_object(
                &json!({ "clientId": "app", "clientSecret": "s", "verifyToken": "vt" }),
                Some(&credential_aad_for(None, "whatsapp")),
            )
            .unwrap();
        let value = serde_json::to_string(&blob).unwrap();
        db.write(move |tx| {
            tx.execute(
                r#"INSERT INTO "PlatformCredential" ("id", "type", "publicConfig", "value") VALUES (1, 'whatsapp', '{}', ?1)"#,
                [value],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let ok = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            false,
            "",
            request(
                true,
                Some("hub.mode=subscribe&hub.verify_token=vt&hub.challenge=42"),
                None,
                b"",
            ),
        )
        .await;
        assert_eq!((ok.status, ok.body.as_str()), (200, "42"));
        let bad = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            false,
            "",
            request(
                true,
                Some("hub.mode=subscribe&hub.verify_token=x"),
                None,
                b"",
            ),
        )
        .await;
        assert_eq!(
            (bad.status, bad.body.as_str()),
            (400, r#"{"message":"Invalid token verification"}"#)
        );

        let body = br#"{"object":"whatsapp_business_account","entry":[{"id":"w","changes":[{"field":"messages","value":{"metadata":{"phone_number_id":"111"},"messages":[{"id":"wamid.A","from":"849","type":"text","text":{"body":"hi"}}]}}]}]}"#;
        let unsigned = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            false,
            "",
            request(false, None, None, body),
        )
        .await;
        assert_eq!(unsigned.status, 401);
        let signature = sign("s", body);
        let signed = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            false,
            "",
            request(false, None, Some(&signature), body),
        )
        .await;
        assert_eq!((signed.status, signed.body.as_str()), (200, "ok"));
        assert_eq!(jobs.named("incomingMessage").len(), 1);

        // Cloud: only the broker host speaks for the platform.
        let other = platform_webhook(
            &db,
            &jobs,
            Some(&encryption),
            true,
            "app.test",
            PlatformRequest {
                domain: Some("tenant.test"),
                ..request(true, None, None, b"")
            },
        )
        .await;
        assert_eq!(other.status, 404);
    }
}
