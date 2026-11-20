//! `coexist/whatsapp-history-payload.ts` + `whatsapp-flush-reduce.ts`: one
//! staged WhatsApp Coexistence `changes[].value` slice to contacts, historical
//! messages, post-batch patches and history metadata. Pure (no database).
use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::channel::{IncomingAttachment, IncomingContact};

/// History-decline error code per Meta docs.
pub const HISTORY_DECLINED_ERROR_CODE: i64 = 2_593_109;
const MEDIA_KEYS: [&str; 5] = ["image", "video", "audio", "document", "sticker"];

/// `HistoryMetadata`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HistoryMetadata {
    pub phase: f64,
    pub chunk_order: f64,
    pub progress: f64,
}

/// `isWhatsappHistoryTerminal`: the last phase (2) at 100 %.
pub fn is_terminal(phase: Option<f64>, progress: Option<f64>) -> bool {
    phase.is_some_and(|p| p >= 2.0) && progress.is_some_and(|p| p >= 100.0)
}

/// `reduceMetadata`: lexicographic by `(phase, progress, chunkOrder)`.
pub fn reduce_metadata(current: Option<HistoryMetadata>, next: HistoryMetadata) -> HistoryMetadata {
    let Some(current) = current else {
        return next;
    };
    let key = |m: &HistoryMetadata| (m.phase, m.progress, m.chunk_order);
    if key(&next) > key(&current) {
        next
    } else {
        current
    }
}

/// One historical message (`IncomingMessage & { createdAt?: Date }`).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoricalMessage {
    pub source_id: String,
    /// `incoming` | `outgoing`.
    pub message_type: &'static str,
    pub text: String,
    /// Seconds-based Meta timestamp in ms; `None` = missing or not a number.
    pub created_at: Option<i64>,
    pub attachments: Vec<IncomingAttachment>,
    /// A template card (`contentAttributes`; Messenger button templates).
    pub content_attributes: Option<Value>,
}

/// `ContactWithMessage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactWithMessage {
    pub contact: IncomingContact,
    pub message: Option<HistoricalMessage>,
}

/// `PendingCoexistPatch` (`kind` = `media` | `edit` | `revoke`), stored on
/// `CoexistSyncRun.pendingPatches.entries` in the TS JSON shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub kind: &'static str,
    pub source_id: String,
    pub contact_wa_id: String,
    /// `edit` only (`None` = the text did not change).
    pub text: Option<String>,
    pub attachment: Option<IncomingAttachment>,
    /// ISO time of the first staging (`stagedAt`); set when carried.
    pub staged_at: Option<String>,
}

impl Patch {
    /// `pendingPatchKey`.
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.kind, self.contact_wa_id, self.source_id)
    }

    pub fn to_json(&self, now: &str) -> Value {
        let mut out = json!({
            "kind": self.kind,
            "contactWaId": self.contact_wa_id,
            "sourceId": self.source_id,
            "stagedAt": self.staged_at.as_deref().unwrap_or(now),
        });
        match self.kind {
            "media" => {
                out["attachment"] = self
                    .attachment
                    .as_ref()
                    .map_or(Value::Null, attachment_json)
            }
            "edit" => {
                out["text"] = self.text.clone().map_or(Value::Null, Value::String);
                out["attachment"] = self
                    .attachment
                    .as_ref()
                    .map_or(Value::Null, attachment_json);
            }
            _ => {}
        }
        out
    }

    pub fn from_json(v: &Value) -> Option<Patch> {
        let kind = match v["kind"].as_str()? {
            "media" => "media",
            "edit" => "edit",
            "revoke" => "revoke",
            _ => return None,
        };
        let attachment = attachment_from_json(&v["attachment"]);
        if kind == "media" && attachment.is_none() {
            return None;
        }
        Some(Patch {
            kind,
            source_id: v["sourceId"].as_str()?.to_owned(),
            contact_wa_id: v["contactWaId"].as_str()?.to_owned(),
            text: v["text"].as_str().map(str::to_owned),
            attachment,
            staged_at: v["stagedAt"].as_str().map(str::to_owned),
        })
    }
}

fn attachment_json(a: &IncomingAttachment) -> Value {
    json!({
        "sourceId": a.source_id, "fileType": a.file_type, "mimeType": a.mime_type,
        "originPath": a.origin_path, "size": a.size.unwrap_or(0), "width": a.width,
        "height": a.height, "name": a.name, "isPhotoMessage": a.is_photo_message,
    })
}

fn attachment_from_json(v: &Value) -> Option<IncomingAttachment> {
    Some(IncomingAttachment {
        source_id: v["sourceId"].as_str().map(str::to_owned),
        file_type: v["fileType"].as_str()?.to_owned(),
        mime_type: v["mimeType"].as_str()?.to_owned(),
        origin_path: v["originPath"].as_str()?.to_owned(),
        size: v["size"].as_i64(),
        width: v["width"].as_i64(),
        height: v["height"].as_i64(),
        name: v["name"].as_str().map(str::to_owned),
        is_photo_message: v["isPhotoMessage"].as_bool().unwrap_or(false),
        optimized: false,
    })
}

/// `ExtractResult`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extracted {
    pub entries: Vec<ContactWithMessage>,
    pub patches: Vec<Patch>,
    pub declined: bool,
    pub metadata: Option<HistoryMetadata>,
    pub metadata_entries: Vec<HistoryMetadata>,
    pub parse_failed: bool,
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The zod `waValueSchema` checks that would reject the payload: a known key
/// of the wrong type, or an item without its required string id.
fn schema_ok(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    // zod `.optional()`: absent is fine, `null` is not.
    let arrays_ok = |o: &Map<String, Value>, keys: &[&str]| {
        keys.iter().all(|k| o.get(*k).is_none_or(Value::is_array))
    };
    let opt_str = |v: &Value, k: &str| v.get(k).is_none_or(Value::is_string);
    let message_ok = |m: &Value| {
        m.is_object()
            && m.get("id").is_some_and(Value::is_string)
            && ["from", "to", "from_user_id", "to_user_id", "type"]
                .iter()
                .all(|k| opt_str(m, k))
            && m.get("timestamp")
                .is_none_or(|t| t.is_string() || t.is_number())
            && m.get("text")
                .is_none_or(|t| t.get("body").is_some_and(Value::is_string))
    };
    if !arrays_ok(
        obj,
        &[
            "contacts",
            "history",
            "state_sync",
            "message_echoes",
            "smb_app_state_sync",
            "smb_message_echoes",
            "messages",
        ],
    ) {
        return false;
    }
    arr(value, "contacts")
        .iter()
        .all(|c| c.get("wa_id").is_some_and(Value::is_string) && opt_str(c, "user_id"))
        && arr(value, "history").iter().all(|h| {
            h.is_object()
                && arr(h, "threads").iter().all(|t| {
                    t.get("id").is_some_and(Value::is_string)
                        && opt_str(t, "user_id")
                        && arr(t, "messages").iter().all(message_ok)
                })
                && arr(h, "errors")
                    .iter()
                    .all(|e| e.get("code").is_some_and(Value::is_number))
        })
        && arr(value, "messages").iter().all(message_ok)
        && [
            arr(value, "smb_message_echoes"),
            arr(value, "message_echoes"),
        ]
        .concat()
        .iter()
        .all(|e| {
            message_ok(e)
                && e.get("from").is_some_and(Value::is_string)
                && e.get("to").is_some_and(Value::is_string)
        })
}

fn to_ms(timestamp: Option<&Value>) -> Option<i64> {
    let seconds = match timestamp? {
        Value::Number(n) => n.as_f64()?,
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .or_else(|| text.trim().is_empty().then_some(0.0))?,
        _ => return None,
    };
    seconds.is_finite().then_some((seconds * 1000.0) as i64)
}

/// `extractMedia` + `buildWaIncomingAttachment`: a `wa-media:{id}` stub, or
/// `None` when the media object carries no id.
fn media_of(message: &Value) -> Option<IncomingAttachment> {
    let (key, media) = MEDIA_KEYS
        .iter()
        .find_map(|k| message.get(*k).filter(|m| m.is_object()).map(|m| (*k, m)))?;
    let id = s(media, "id")?;
    let mime_type = s(media, "mime_type").unwrap_or("application/octet-stream");
    Some(IncomingAttachment {
        source_id: Some(id.to_owned()),
        file_type: chatbotx_files::guess_file_type_from_mime_type(mime_type)
            .as_str()
            .to_owned(),
        mime_type: mime_type.to_owned(),
        origin_path: format!("wa-media:{id}"),
        size: Some(0),
        width: None,
        height: None,
        name: s(media, "caption").map(str::to_owned),
        is_photo_message: key == "image",
        optimized: false,
    })
}

#[derive(Default)]
struct Lookups<'a> {
    name: HashMap<&'a str, &'a str>,
    user_id: HashMap<&'a str, &'a str>,
    username: HashMap<&'a str, &'a str>,
}

/// `buildCoexistIncomingContact` → `(contact, customerWaId, customerUserId)`.
fn contact_of(
    raw_wa_id: &str,
    raw_user_id: Option<&str>,
    l: &Lookups<'_>,
) -> (IncomingContact, String, Option<String>) {
    let user_id = raw_user_id
        .or_else(|| l.user_id.get(raw_wa_id).copied())
        .map(str::to_owned);
    let wa_id = if raw_wa_id.is_empty() {
        user_id.clone().unwrap_or_default()
    } else {
        raw_wa_id.to_owned()
    };
    let contact = IncomingContact {
        source_id: wa_id.clone(),
        phone_number: (!raw_wa_id.is_empty()).then(|| raw_wa_id.to_owned()),
        first_name: l.name.get(raw_wa_id).map(|n| (*n).to_owned()),
        source_user_id: user_id.clone(),
        source_username: l.username.get(raw_wa_id).map(|n| (*n).to_owned()),
        ..IncomingContact::default()
    };
    (contact, wa_id, user_id)
}

/// `resolveIsOutgoingMessage`.
fn is_outgoing(
    from: Option<&str>,
    from_user_id: Option<&str>,
    wa_id: &str,
    user_id: Option<&str>,
) -> bool {
    if let Some(from) = from.filter(|f| !f.is_empty()) {
        return from != wa_id;
    }
    match (from_user_id.filter(|f| !f.is_empty()), user_id) {
        (Some(from), Some(user)) => from != user,
        _ => false,
    }
}

fn text_of(message: &Value) -> String {
    message
        .pointer("/text/body")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| s(message, "type").map(|t| format!("[{t}]")))
        .unwrap_or_default()
}

fn historical(message: &Value, message_type: &'static str) -> HistoricalMessage {
    HistoricalMessage {
        source_id: s(message, "id").unwrap_or_default().to_owned(),
        message_type,
        text: text_of(message),
        created_at: to_ms(message.get("timestamp")),
        attachments: media_of(message).into_iter().collect(),
        content_attributes: None,
    }
}

/// `extractFromValue`.
pub fn extract_from_value(payload: &Value) -> Extracted {
    if !schema_ok(payload) {
        return Extracted {
            parse_failed: true,
            ..Extracted::default()
        };
    }
    let mut lookups = Lookups::default();
    for contact in arr(payload, "contacts") {
        let wa_id = s(contact, "wa_id").unwrap_or_default();
        if let Some(name) = contact.pointer("/profile/name").and_then(Value::as_str) {
            lookups.name.insert(wa_id, name);
        }
        if let Some(user_id) = s(contact, "user_id") {
            lookups.user_id.insert(wa_id, user_id);
        }
        if let Some(username) = contact.pointer("/profile/username").and_then(Value::as_str) {
            lookups.username.insert(wa_id, username);
        }
    }
    let mut out = Extracted::default();

    // `collectHistory`.
    for entry in arr(payload, "history") {
        if arr(entry, "errors")
            .iter()
            .any(|e| e["code"].as_i64() == Some(HISTORY_DECLINED_ERROR_CODE))
        {
            out.declined = true;
        }
        if let Some(meta) = entry.get("metadata").filter(|m| m.is_object()) {
            let num = |k: &str| meta.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let parsed = HistoryMetadata {
                phase: num("phase"),
                chunk_order: num("chunk_order"),
                progress: num("progress"),
            };
            out.metadata_entries.push(parsed);
            out.metadata = Some(reduce_metadata(out.metadata, parsed));
        }
        for thread in arr(entry, "threads") {
            let (contact, wa_id, user_id) = contact_of(
                s(thread, "id").unwrap_or_default(),
                s(thread, "user_id"),
                &lookups,
            );
            let raw = arr(thread, "messages");
            let messages: Vec<&Value> = raw
                .iter()
                .filter(|m| s(m, "type") != Some("errors"))
                .collect();
            if !raw.is_empty() && messages.is_empty() {
                continue;
            }
            if messages.is_empty() {
                out.entries.push(ContactWithMessage {
                    contact,
                    message: None,
                });
                continue;
            }
            for message in messages {
                let outgoing = is_outgoing(
                    s(message, "from"),
                    s(message, "from_user_id"),
                    &wa_id,
                    user_id.as_deref(),
                );
                out.entries.push(ContactWithMessage {
                    contact: contact.clone(),
                    message: Some(historical(
                        message,
                        if outgoing { "outgoing" } else { "incoming" },
                    )),
                });
            }
        }
    }

    // `collectStateSync`.
    for entry in [
        arr(payload, "smb_app_state_sync"),
        arr(payload, "state_sync"),
    ]
    .concat()
    {
        let phone = entry
            .pointer("/contact/phone_number")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty());
        let Some(phone) = phone.filter(|_| s(&entry, "action") != Some("remove")) else {
            continue;
        };
        let name = entry
            .pointer("/contact/first_name")
            .or_else(|| entry.pointer("/contact/full_name"))
            .and_then(Value::as_str);
        out.entries.push(ContactWithMessage {
            contact: IncomingContact {
                source_id: phone.to_owned(),
                phone_number: Some(phone.to_owned()),
                first_name: name.map(str::to_owned),
                ..IncomingContact::default()
            },
            message: None,
        });
    }

    // `collectEchoes`.
    for echo in [
        arr(payload, "smb_message_echoes"),
        arr(payload, "message_echoes"),
    ]
    .concat()
    {
        let (contact, _, _) = contact_of(
            s(&echo, "to").unwrap_or_default(),
            s(&echo, "to_user_id"),
            &lookups,
        );
        out.entries.push(ContactWithMessage {
            contact,
            message: Some(historical(&echo, "outgoing")),
        });
    }

    // `collectPatches`.
    for message in arr(payload, "messages") {
        let Some(wa_id) = s(message, "from")
            .filter(|f| !f.is_empty())
            .or_else(|| s(message, "from_user_id"))
        else {
            continue;
        };
        let patch = |kind, source_id: &str, text, attachment| Patch {
            kind,
            source_id: source_id.to_owned(),
            contact_wa_id: wa_id.to_owned(),
            text,
            attachment,
            staged_at: None,
        };
        if s(message, "type") == Some("revoke") || message.get("revoke").is_some() {
            if let Some(original) = message
                .pointer("/revoke/original_message_id")
                .and_then(Value::as_str)
            {
                out.patches.push(patch("revoke", original, None, None));
            }
            continue;
        }
        if s(message, "type") == Some("edit") || message.get("edit").is_some() {
            if let Some(original) = message
                .pointer("/edit/original_message_id")
                .and_then(Value::as_str)
            {
                let edited = message
                    .pointer("/edit/message")
                    .cloned()
                    .unwrap_or(Value::Null);
                let text = edited
                    .pointer("/text/body")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                out.patches
                    .push(patch("edit", original, text, media_of(&edited)));
            }
            continue;
        }
        if let Some(attachment) = media_of(message) {
            out.patches.push(patch(
                "media",
                s(message, "id").unwrap_or_default(),
                None,
                Some(attachment),
            ));
        }
    }
    out
}

/// One contact and its messages (`HistoricalContactMessages`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContactMessages {
    pub contact: IncomingContact,
    pub messages: Vec<HistoricalMessage>,
}

/// `ReducedBatch`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reduced {
    pub batch: Vec<ContactMessages>,
    pub patches: Vec<Patch>,
    pub parse_failed_row_ids: Vec<i64>,
    pub declined: bool,
    pub metadata: Option<HistoryMetadata>,
    pub terminal_seen: bool,
}

/// `mergeContact`: the first non-null field wins.
fn merge_contact(acc: &mut IncomingContact, next: &IncomingContact) {
    for (field, value) in [
        (&mut acc.phone_number, &next.phone_number),
        (&mut acc.first_name, &next.first_name),
        (&mut acc.last_name, &next.last_name),
        (&mut acc.email, &next.email),
        (&mut acc.avatar, &next.avatar),
        (&mut acc.source_user_id, &next.source_user_id),
        (&mut acc.source_username, &next.source_username),
    ] {
        if field.is_none() {
            field.clone_from(value);
        }
    }
}

/// `reduceStagedRows`: rows are `(stagingId, payload)` in staging order.
pub fn reduce_staged_rows(rows: &[(i64, Value)]) -> Reduced {
    let mut out = Reduced::default();
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, ContactMessages> = HashMap::new();
    for (id, payload) in rows {
        let extracted = extract_from_value(payload);
        if extracted.parse_failed {
            tracing::error!(
                staging_id = id,
                "[coexist] Unparseable WhatsApp payload — parked as parse-failed"
            );
            out.parse_failed_row_ids.push(*id);
            continue;
        }
        for entry in extracted.entries {
            if entry.contact.source_id.is_empty() {
                continue;
            }
            match groups.get_mut(&entry.contact.source_id) {
                Some(group) => merge_contact(&mut group.contact, &entry.contact),
                None => {
                    order.push(entry.contact.source_id.clone());
                    groups.insert(
                        entry.contact.source_id.clone(),
                        ContactMessages {
                            contact: entry.contact.clone(),
                            messages: Vec::new(),
                        },
                    );
                }
            }
            if let Some(message) = entry.message {
                groups
                    .get_mut(&entry.contact.source_id)
                    .expect("inserted")
                    .messages
                    .push(message);
            }
        }
        out.patches.extend(extracted.patches);
        out.declined |= extracted.declined;
        out.terminal_seen |= extracted
            .metadata_entries
            .iter()
            .any(|m| is_terminal(Some(m.phase), Some(m.progress)));
        if let Some(meta) = extracted.metadata {
            out.metadata = Some(reduce_metadata(out.metadata, meta));
        }
    }
    out.batch = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect();
    out
}

#[cfg(test)]
mod tests {
    //! `coexist-whatsapp-flush.test.ts` payload-shape cases.
    use super::*;

    fn meta(phase: f64, progress: f64, chunk_order: f64) -> HistoryMetadata {
        HistoryMetadata {
            phase,
            chunk_order,
            progress,
        }
    }

    #[test]
    fn metadata_reduces_by_phase_then_progress_then_chunk() {
        let a = reduce_metadata(Some(meta(0.0, 100.0, 9.0)), meta(2.0, 40.0, 1.0));
        assert_eq!(a, meta(2.0, 40.0, 1.0), "phase outranks progress");
        assert!(!is_terminal(Some(a.phase), Some(a.progress)));
        assert_eq!(
            reduce_metadata(Some(meta(2.0, 40.0, 1.0)), meta(2.0, 100.0, 0.0)),
            meta(2.0, 100.0, 0.0)
        );
        assert!(is_terminal(Some(2.0), Some(100.0)));
    }

    #[test]
    fn threads_echoes_state_sync_and_patches_are_extracted() {
        let value = json!({
            "contacts": [{ "wa_id": "849", "profile": { "name": "Ann", "username": "ann" } }],
            "history": [{
                "metadata": { "phase": 2, "chunk_order": 1, "progress": 100 },
                "threads": [
                    { "id": "849", "messages": [
                        { "id": "m1", "from": "849", "timestamp": "1755700000", "type": "text", "text": { "body": "hi" } },
                        { "id": "m2", "from": "biz", "timestamp": 1755700001, "type": "image", "image": { "id": "media-1", "mime_type": "image/jpeg" } },
                    ]},
                    { "id": "850", "messages": [{ "id": "e", "type": "errors" }] },
                    { "id": "", "user_id": "US.1", "messages": [{ "id": "m3", "from": "", "from_user_id": "US.1", "type": "text", "text": { "body": "x" } }] },
                ],
            }],
            "smb_app_state_sync": [
                { "action": "add", "contact": { "phone_number": "851", "full_name": "Bob" } },
                { "action": "remove", "contact": { "phone_number": "852" } },
            ],
            "smb_message_echoes": [{ "from": "biz", "to": "849", "id": "m4", "type": "text", "text": { "body": "yo" } }],
            "messages": [
                { "id": "r", "from": "849", "type": "revoke", "revoke": { "original_message_id": "m1" } },
                { "id": "ed", "from": "849", "type": "edit", "edit": { "original_message_id": "m1", "message": { "text": { "body": "hey" } } } },
                { "id": "m2", "from": "849", "type": "image", "image": { "id": "media-2" } },
            ],
        });
        let out = extract_from_value(&value);
        assert!(!out.parse_failed);
        let ids: Vec<_> = out
            .entries
            .iter()
            .map(|e| {
                (
                    e.contact.source_id.as_str(),
                    e.message
                        .as_ref()
                        .map(|m| (m.source_id.as_str(), m.message_type)),
                )
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                ("849", Some(("m1", "incoming"))),
                ("849", Some(("m2", "outgoing"))),
                ("US.1", Some(("m3", "incoming"))),
                ("851", None),
                ("849", Some(("m4", "outgoing"))),
            ]
        );
        assert_eq!(out.entries[0].contact.first_name.as_deref(), Some("Ann"));
        assert_eq!(
            out.entries[0].contact.source_username.as_deref(),
            Some("ann")
        );
        assert_eq!(
            out.entries[0].message.as_ref().unwrap().created_at,
            Some(1_755_700_000_000)
        );
        assert_eq!(
            out.entries[1].message.as_ref().unwrap().attachments[0].origin_path,
            "wa-media:media-1"
        );
        assert_eq!(
            out.entries[2].contact.phone_number, None,
            "a BSUID is not a phone"
        );
        assert_eq!(out.entries[3].contact.first_name.as_deref(), Some("Bob"));
        let kinds: Vec<_> = out
            .patches
            .iter()
            .map(|p| (p.kind, p.source_id.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![("revoke", "m1"), ("edit", "m1"), ("media", "m2")]
        );
        assert_eq!(out.patches[1].text.as_deref(), Some("hey"));
        assert!(is_terminal(
            out.metadata.map(|m| m.phase),
            out.metadata.map(|m| m.progress)
        ));
    }

    #[test]
    fn a_decline_and_a_bad_shape_are_reported() {
        let declined =
            extract_from_value(&json!({ "history": [{ "errors": [{ "code": 2_593_109 }] }] }));
        assert!(declined.declined && !declined.parse_failed);
        assert!(extract_from_value(&json!({ "history": "nope" })).parse_failed);
        assert!(
            extract_from_value(&json!({ "history": [{ "threads": [{ "messages": [] }] }] }))
                .parse_failed
        );
        assert!(extract_from_value(&json!([])).parse_failed);
    }

    #[test]
    fn staged_rows_coalesce_per_contact_and_park_bad_rows() {
        let row = |name: Option<&str>, id: &str| {
            json!({
                "contacts": name.map(|n| vec![json!({ "wa_id": "849", "profile": { "name": n } })]).unwrap_or_default(),
                "history": [{ "threads": [{ "id": "849", "messages": [{ "id": id, "from": "849", "type": "text", "text": { "body": "t" } }] }] }],
            })
        };
        let reduced = reduce_staged_rows(&[
            (1, row(None, "a")),
            (2, json!({ "messages": 3 })),
            (3, row(Some("Ann"), "b")),
        ]);
        assert_eq!(reduced.parse_failed_row_ids, vec![2]);
        assert_eq!(reduced.batch.len(), 1);
        assert_eq!(reduced.batch[0].contact.first_name.as_deref(), Some("Ann"));
        assert_eq!(reduced.batch[0].messages.len(), 2);
    }

    #[test]
    fn a_pending_patch_round_trips_in_the_ts_shape() {
        let patch = Patch {
            kind: "edit",
            source_id: "m1".into(),
            contact_wa_id: "849".into(),
            text: Some("hey".into()),
            attachment: None,
            staged_at: None,
        };
        let stored = patch.to_json("2026-11-18T00:00:00.000Z");
        assert_eq!(stored["stagedAt"], "2026-11-18T00:00:00.000Z");
        let back = Patch::from_json(&stored).unwrap();
        assert_eq!(back.key(), "edit:849:m1");
        assert_eq!(back.text.as_deref(), Some("hey"));
    }
}
