//! Meta's `calls` webhook field (`integrations/whatsapp/src/lib/calls.ts`
//! `extractCallEventPayloads`, `handlers/webhook.ts` `enqueueCallEventPayloads`
//! and `enqueueNativeCallCapture`): one `whatsappCallEvent` job per call event,
//! and the native recording / transcript fetch jobs.
//!
//! ponytail: the VoIP connect signalling (`captureConnectOffer`,
//! `rejectUnprocessableConnect`, `captureOutboundAnswer`) is not ported, so a
//! session SDP is never read past its validity check. Optional fields of the
//! wrong JSON type read as absent (zod fails the whole `calls` value). The
//! native fetch jobs carry no `whatsappCallId`; the job resolves the row by
//! wacid, as the original does for a not-yet-created row.
use chatbotx_jobs::JobOptions;
use chatbotx_jobs::options::{Backoff, Retention};
use chatbotx_jobs::queues::whatsapp_call::{
    whatsapp_call_native_recording_fetch_job_id, whatsapp_call_native_transcript_fetch_job_id,
};
use serde_json::{Map, Value, json};

use crate::channels::whatsapp_webhook::safe_id_segment;

pub const EVENT_JOB: &str = "whatsappCallEvent";
pub const NATIVE_RECORDING_FETCH_JOB: &str = "whatsappCallNativeRecordingFetch";
pub const NATIVE_TRANSCRIPT_FETCH_JOB: &str = "whatsappCallNativeTranscriptFetch";
const MAX_SDP_CHARS: usize = 100_000;
const MAX_TERMINATE_ERRORS: usize = 20;
/// `TERMINATE_JOB_DELAY_MS`: interim statuses commit first.
const TERMINATE_JOB_DELAY_MS: u64 = 2000;

/// One queued job of a `calls` delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct CallJob {
    pub name: &'static str,
    pub data: Value,
    pub opts: JobOptions,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// `z.union([z.string(), z.number()])` → `String(value)`.
fn str_or_num(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn direction(v: &Value) -> Option<&'static str> {
    match v.get("direction").and_then(Value::as_str) {
        Some("USER_INITIATED") => Some("userInitiated"),
        Some("BUSINESS_INITIATED") => Some("businessInitiated"),
        _ => None,
    }
}

/// Builds an object without its `None` fields (`undefined` drops on JSON).
fn object(fields: Vec<(&str, Option<Value>)>) -> Value {
    let map: Map<String, Value> = fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
        .collect();
    Value::Object(map)
}

fn opt(v: Option<String>) -> Option<Value> {
    v.map(Value::String)
}

/// `toContactPayload`.
fn contact_payload(contact: &Value) -> Value {
    object(vec![
        ("waId", opt(s(contact, "wa_id"))),
        ("userId", opt(s(contact, "user_id"))),
        ("parentUserId", opt(s(contact, "parent_user_id"))),
        (
            "username",
            opt(contact
                .pointer("/profile/username")
                .and_then(Value::as_str)
                .map(str::to_owned)),
        ),
        (
            "name",
            opt(contact
                .pointer("/profile/name")
                .and_then(Value::as_str)
                .map(str::to_owned)),
        ),
    ])
}

/// `pickContactForCallItem`: by identity, else the one contact for an item
/// with no identity of its own.
fn pick_contact(contacts: &[Value], from: Option<&str>, from_user: Option<&str>) -> Option<Value> {
    if contacts.is_empty() {
        return None;
    }
    let matched = contacts.iter().find(|c| {
        (from.is_some() && c.get("wa_id").and_then(Value::as_str) == from)
            || (from_user.is_some() && c.get("user_id").and_then(Value::as_str) == from_user)
    });
    if let Some(c) = matched {
        return Some(contact_payload(c));
    }
    (from.is_none() && from_user.is_none() && contacts.len() == 1)
        .then(|| contact_payload(&contacts[0]))
}

/// `parseCallSession`: `None` = no session, `Some(false)` = invalid.
fn session_valid(raw: Option<&Value>, expected: &str) -> Option<bool> {
    let raw = raw?;
    let sdp = raw.get("sdp").and_then(Value::as_str);
    let valid = sdp.is_some_and(|sdp| !sdp.is_empty() && sdp.chars().count() <= MAX_SDP_CHARS)
        && raw.get("sdp_type").and_then(Value::as_str) == Some(expected);
    if !valid {
        tracing::warn!("Whatsapp call session invalid: malformed session");
    }
    Some(valid)
}

/// The bounded `errors[]` (`callTerminateErrorSchema`), or `None`.
fn terminate_errors(v: Option<&Value>) -> Option<Vec<Value>> {
    let items = v?.as_array()?;
    if items.len() > MAX_TERMINATE_ERRORS {
        return None;
    }
    Some(
        items
            .iter()
            .map(|e| {
                object(vec![
                    ("code", e.get("code").filter(|c| c.is_number()).cloned()),
                    ("title", opt(s(e, "title"))),
                    ("message", opt(s(e, "message"))),
                ])
            })
            .collect(),
    )
}

fn identity_fields(item: &Value) -> Vec<(&'static str, Option<Value>)> {
    vec![
        ("from", opt(s(item, "from"))),
        ("to", opt(s(item, "to"))),
        ("fromUserId", opt(s(item, "from_user_id"))),
        ("toUserId", opt(s(item, "to_user_id"))),
        ("fromParentUserId", opt(s(item, "from_parent_user_id"))),
        ("toParentUserId", opt(s(item, "to_parent_user_id"))),
        ("timestamp", opt(str_or_num(item, "timestamp"))),
    ]
}

fn media_ref(media: &Value) -> Option<Value> {
    let id = s(media, "id")?;
    Some(object(vec![
        ("mediaId", Some(Value::String(id))),
        ("sha256", opt(s(media, "sha256"))),
        ("mimeType", opt(s(media, "mime_type"))),
        ("url", opt(s(media, "url"))),
    ]))
}

/// `normalizeCallItem`.
fn normalize_call_item(item: &Value) -> Option<Value> {
    let wacid = s(item, "id")?;
    let event = item.get("event").and_then(Value::as_str)?;
    let mut fields: Vec<(&str, Option<Value>)> = vec![("wacid", Some(json!(wacid)))];
    match event {
        "connect" | "call_created" => {
            let Some(dir) = direction(item) else {
                tracing::warn!(wacid, "Whatsapp call connect skipped: missing direction");
                return None;
            };
            let expected = if dir == "businessInitiated" {
                "answer"
            } else {
                "offer"
            };
            let session = session_valid(item.get("session"), expected);
            fields.push(("kind", Some(json!("connect"))));
            fields.push(("direction", Some(json!(dir))));
            fields.extend(identity_fields(item));
            fields.push(("sessionInvalid", Some(json!(session == Some(false)))));
            fields.push((
                "bizOpaqueCallbackData",
                opt(s(item, "biz_opaque_callback_data")),
            ));
        }
        "terminate" => {
            let raw = s(item, "status").unwrap_or_default().to_uppercase();
            if raw != "COMPLETED" && raw != "FAILED" {
                tracing::warn!(
                    wacid,
                    status = raw,
                    "Whatsapp call terminate status unknown; defaulting to FAILED"
                );
            }
            let status = if raw == "COMPLETED" {
                "COMPLETED"
            } else {
                "FAILED"
            };
            let duration = match item.get("duration") {
                Some(Value::Number(n)) => n.as_f64(),
                Some(Value::String(t)) => t
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .or(t.trim().is_empty().then_some(0.0)),
                _ => None,
            };
            fields.push(("kind", Some(json!("terminate"))));
            fields.push(("direction", direction(item).map(|d| json!(d))));
            fields.push(("status", Some(json!(status))));
            fields.extend(identity_fields(item));
            fields.push(("startTime", opt(str_or_num(item, "start_time"))));
            fields.push(("endTime", opt(str_or_num(item, "end_time"))));
            fields.push((
                "durationSeconds",
                duration.filter(|d| d.is_finite()).map(|d| json!(d)),
            ));
            fields.push((
                "errors",
                terminate_errors(item.get("errors")).map(Value::from),
            ));
        }
        "call_recording_available" => {
            let Some(audio) = item.pointer("/call_recording/audio").and_then(media_ref) else {
                tracing::warn!(
                    wacid,
                    event,
                    "Whatsapp call recording-available skipped: missing call_recording"
                );
                return None;
            };
            fields.push(("kind", Some(json!("recordingAvailable"))));
            fields.push(("audio", Some(audio)));
            fields.push(("direction", direction(item).map(|d| json!(d))));
            fields.push((
                "bizOpaqueCallbackData",
                opt(s(item, "biz_opaque_callback_data")),
            ));
        }
        "call_transcript_available" | "call_transcription_available" => {
            let Some(document) = item
                .pointer("/call_transcript/document")
                .and_then(media_ref)
            else {
                tracing::warn!(
                    wacid,
                    event,
                    "Whatsapp call transcription-available skipped: missing call_transcript"
                );
                return None;
            };
            fields.push(("kind", Some(json!("transcriptionAvailable"))));
            fields.push(("document", Some(document)));
        }
        _ => {
            tracing::warn!(wacid, event, "Whatsapp call event skipped: unknown event");
            return None;
        }
    }
    Some(object(fields))
}

/// `normalizeStatusItem`.
fn normalize_status_item(item: &Value) -> Option<Value> {
    let wacid = s(item, "id")?;
    let status = s(item, "status")?;
    if !matches!(status.as_str(), "RINGING" | "ACCEPTED" | "REJECTED") {
        tracing::warn!(
            wacid,
            status,
            "Whatsapp call status skipped: unknown status"
        );
        return None;
    }
    Some(object(vec![
        ("kind", Some(json!("status"))),
        ("wacid", Some(json!(wacid))),
        ("status", Some(json!(status))),
        ("recipientId", opt(s(item, "recipient_id"))),
        ("recipientUserId", opt(s(item, "recipient_user_id"))),
        ("timestamp", opt(str_or_num(item, "timestamp"))),
        (
            "bizOpaqueCallbackData",
            opt(s(item, "biz_opaque_callback_data")),
        ),
    ]))
}

/// `extractCallEventPayloads`: `{ phoneNumberId, contact?, event }` per
/// item, statuses before call events.
pub fn extract_call_events(body: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let changes = body
        .get("entry")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("changes").and_then(Value::as_array))
        .flatten()
        .filter(|c| c.get("field").and_then(Value::as_str) == Some("calls"));
    for change in changes {
        let value = &change["value"];
        let Some(phone) = value
            .pointer("/metadata/phone_number_id")
            .and_then(Value::as_str)
        else {
            tracing::warn!("Whatsapp call webhook skipped: malformed value");
            continue;
        };
        let list = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let (contacts, calls, statuses) = (list("contacts"), list("calls"), list("statuses"));
        let terminates = calls
            .iter()
            .filter(|c| c.get("event").and_then(Value::as_str) == Some("terminate"))
            .count();
        let shared_errors = (terminates == 1)
            .then(|| terminate_errors(value.get("errors")))
            .flatten();
        for item in &statuses {
            if let Some(event) = normalize_status_item(item) {
                let contact = pick_contact(
                    &contacts,
                    item.get("recipient_id").and_then(Value::as_str),
                    item.get("recipient_user_id").and_then(Value::as_str),
                );
                out.push(object(vec![
                    ("phoneNumberId", Some(json!(phone))),
                    ("contact", contact),
                    ("event", Some(event)),
                ]));
            }
        }
        for item in &calls {
            let Some(mut event) = normalize_call_item(item) else {
                continue;
            };
            let outbound =
                item.get("direction").and_then(Value::as_str) == Some("BUSINESS_INITIATED");
            let (from, from_user) = if outbound {
                ("to", "to_user_id")
            } else {
                ("from", "from_user_id")
            };
            let contact = pick_contact(
                &contacts,
                item.get(from).and_then(Value::as_str),
                item.get(from_user).and_then(Value::as_str),
            );
            if event["kind"] == "terminate"
                && event.get("errors").is_none()
                && let Some(errors) = &shared_errors
            {
                event["errors"] = Value::from(errors.clone());
            }
            out.push(object(vec![
                ("phoneNumberId", Some(json!(phone))),
                ("contact", contact),
                ("event", Some(event)),
            ]));
        }
    }
    out
}

/// `callEventJobIdSuffix`.
fn job_id_suffix(event: &Value) -> String {
    let kind = event["kind"].as_str().unwrap_or_default();
    if kind == "status" {
        return format!("status-{}", event["status"].as_str().unwrap_or_default());
    }
    kind.to_owned()
}

/// `NATIVE_CALL_CAPTURE_RETRY_OPTIONS`.
fn native_capture_options(job_id: String) -> JobOptions {
    JobOptions {
        job_id: Some(job_id),
        attempts: Some(13),
        backoff: Some(Backoff::Fixed(300_000)),
        ..JobOptions::default()
    }
}

/// The `integration` jobs of one delivery, for the route-pinned number only.
pub fn call_jobs(body: &Value, pinned: Option<&str>) -> Vec<CallJob> {
    let payloads: Vec<Value> = extract_call_events(body)
        .into_iter()
        .filter(|p| pinned.is_none_or(|pinned| p["phoneNumberId"] == pinned))
        .collect();
    let mut jobs = Vec::new();
    for payload in &payloads {
        let event = &payload["event"];
        let wacid = event["wacid"].as_str().unwrap_or_default();
        let kind = event["kind"].as_str().unwrap_or_default();
        jobs.push(CallJob {
            name: EVENT_JOB,
            data: json!({ "type": EVENT_JOB, "data": {
                "integrationType": "whatsapp",
                "integrationIdentifier": payload["phoneNumberId"],
                "payload": payload,
            }}),
            opts: JobOptions {
                job_id: Some(format!(
                    "wa-call-{}-{}",
                    safe_id_segment(wacid),
                    job_id_suffix(event)
                )),
                attempts: Some(5),
                backoff: Some(Backoff::Exponential(30_000)),
                remove_on_fail: Some(Retention::KeepAge {
                    age_secs: 6 * 60 * 60,
                    count: None,
                }),
                delay_ms: (kind == "terminate").then_some(TERMINATE_JOB_DELAY_MS),
                ..JobOptions::default()
            },
        });
    }
    for payload in &payloads {
        let event = &payload["event"];
        let wacid = event["wacid"].as_str().unwrap_or_default();
        match event["kind"].as_str() {
            Some("recordingAvailable") => {
                let (Some(url), Some(mime)) = (
                    event.pointer("/audio/url"),
                    event.pointer("/audio/mimeType"),
                ) else {
                    tracing::warn!(
                        wacid,
                        "Whatsapp native call recording skipped: missing audio url/mimeType"
                    );
                    continue;
                };
                jobs.push(CallJob {
                    name: NATIVE_RECORDING_FETCH_JOB,
                    data: json!({ "type": NATIVE_RECORDING_FETCH_JOB, "data": {
                        "wacid": wacid,
                        "audioMediaId": event.pointer("/audio/mediaId"),
                        "audioUrl": url,
                        "mimeType": mime,
                    }}),
                    opts: native_capture_options(whatsapp_call_native_recording_fetch_job_id(
                        wacid,
                    )),
                });
            }
            Some("transcriptionAvailable") => {
                let Some(url) = event.pointer("/document/url") else {
                    tracing::warn!(
                        wacid,
                        "Whatsapp native call transcript skipped: missing document url"
                    );
                    continue;
                };
                jobs.push(CallJob {
                    name: NATIVE_TRANSCRIPT_FETCH_JOB,
                    data: json!({ "type": NATIVE_TRANSCRIPT_FETCH_JOB, "data": {
                        "wacid": wacid,
                        "documentMediaId": event.pointer("/document/mediaId"),
                        "documentUrl": url,
                    }}),
                    opts: native_capture_options(whatsapp_call_native_transcript_fetch_job_id(
                        wacid,
                    )),
                });
            }
            _ => {}
        }
    }
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(value: Value) -> Value {
        json!({ "object": "whatsapp_business_account", "entry": [{ "id": "w", "changes": [{ "field": "calls", "value": value }] }] })
    }

    #[test]
    fn statuses_come_first_and_a_single_terminate_takes_the_value_level_errors() {
        let b = body(json!({
            "metadata": { "phone_number_id": "p1" },
            "contacts": [{ "wa_id": "49", "profile": { "name": "Ann" } }],
            "calls": [{ "id": "wacid.1", "event": "terminate", "direction": "USER_INITIATED", "from": "49", "status": "completed", "duration": "12", "start_time": 1755700000 }],
            "statuses": [{ "id": "wacid.1", "status": "REJECTED", "recipient_id": "49" }],
            "errors": [{ "code": 138021, "title": "media" }],
        }));
        let events = extract_call_events(&b);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event"]["kind"], "status");
        assert_eq!(events[0]["contact"]["name"], "Ann");
        let terminate = &events[1]["event"];
        assert_eq!(terminate["status"], "COMPLETED");
        assert_eq!(terminate["durationSeconds"], 12.0);
        assert_eq!(terminate["startTime"], "1755700000");
        assert_eq!(terminate["errors"][0]["code"], 138021);
    }

    #[test]
    fn connect_jobs_drop_the_session_and_terminate_jobs_wait() {
        let b = body(json!({
            "metadata": { "phone_number_id": "p1" },
            "calls": [
                { "id": "w:1", "event": "connect", "direction": "USER_INITIATED", "from": "49", "session": { "sdp_type": "answer", "sdp": "v=0" } },
                { "id": "w:1", "event": "terminate", "direction": "USER_INITIATED", "status": "FAILED" },
                { "id": "w:1", "event": "connect" },
                { "id": "w:1", "event": "call_recording_available", "call_recording": { "audio": { "id": "m1", "url": "https://x/a", "mime_type": "audio/ogg" } } },
            ],
        }));
        let jobs = call_jobs(&b, Some("p1"));
        let names: Vec<_> = jobs.iter().map(|j| j.name).collect();
        assert_eq!(
            names,
            vec![EVENT_JOB, EVENT_JOB, EVENT_JOB, NATIVE_RECORDING_FETCH_JOB]
        );
        let connect = &jobs[0].data["data"]["payload"]["event"];
        assert_eq!(
            connect["sessionInvalid"], true,
            "an offer was expected inbound"
        );
        assert!(connect.get("session").is_none());
        assert!(!jobs[0].opts.job_id.as_deref().unwrap().contains(':'));
        assert_eq!(jobs[1].opts.delay_ms, Some(2000));
        assert_eq!(jobs[3].opts.job_id.as_deref(), Some("native-rec-fetch-w:1"));
        assert!(
            call_jobs(&b, Some("other")).is_empty(),
            "another number's change is dropped"
        );
    }
}
