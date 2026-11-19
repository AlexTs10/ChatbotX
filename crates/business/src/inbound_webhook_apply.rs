//! `packages/business/src/inbound-webhook/{extract,extraction-context,resolve-inbox,
//! resolve-contact,apply-mapped-fields,apply-payload}.ts` and
//! `contact/contact-inbox-identity.ts`: what a delivered payload does.
//!
//! ponytail: the contact lookup and creation run in one SQLite write
//! transaction, which replaces the per-inbox distributed lock and the
//! unique-violation race handling (the writer is serial).

use std::collections::HashMap;

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::datetime::TemporalInputParsing;
use chatbotx_core::utils::inbound_webhook_channel::{
    CONTACT_LOOKUP_ORDER, ContactIdentityKind, InboxIdentifierKind, InboxLookupStrategy,
    get_inbox_identifier_strategy, inbound_webhook_channel_spec,
};
use chatbotx_core::utils::json_path::{read_json_path, stringify_json_path_value};
use chatbotx_db::Db;
use chatbotx_db::repo::contact_inbox::{
    IdentityField, find_by_inbox_and_full_name, find_latest_by_inbox_and_contact_identity,
};
use chatbotx_db::repo::models::InboundWebhook;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value as Json, json};

use crate::contact::create::{CreateContactInput, CreateOptions, PhoneUniqueness};
use crate::contact::phone::normalize_identity_phone_to_e164;
use crate::contact_custom_field::{
    NormalizeInput, SetValuesInput, normalize_custom_field_value_for_storage,
    set_values_in_transaction,
};
use crate::errors::{BusinessError, Result};
use crate::facebook_lead_ads::rich_system_columns;
use crate::questionnaire::is_rich_system_contact_field;
use crate::tx;

const MAX_CHANNEL_USER_ID_LENGTH: usize = 255;
const MAX_FULL_NAME_LENGTH: usize = 255;
const GENDERS: &[&str] = &["male", "female", "unknown"];

fn repo(e: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(e.into())
}

/// `ContactInboxIdentities` (`IDENTITY_FIELD` keys).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identities {
    pub channel_user_id: Option<String>,
    pub phone_e164: Option<String>,
    pub email: Option<String>,
    pub full_name: Option<String>,
}

impl Identities {
    /// `identityValue`.
    pub fn get(&self, kind: ContactIdentityKind) -> Option<&str> {
        match kind {
            ContactIdentityKind::ChannelUserId => self.channel_user_id.as_deref(),
            ContactIdentityKind::Phone => self.phone_e164.as_deref(),
            ContactIdentityKind::Email => self.email.as_deref(),
            ContactIdentityKind::FullName => self.full_name.as_deref(),
        }
    }

    fn set(&mut self, kind: ContactIdentityKind, value: String) {
        let slot = match kind {
            ContactIdentityKind::ChannelUserId => &mut self.channel_user_id,
            ContactIdentityKind::Phone => &mut self.phone_e164,
            ContactIdentityKind::Email => &mut self.email,
            ContactIdentityKind::FullName => &mut self.full_name,
        };
        *slot = Some(value);
    }

    fn to_json(&self) -> Json {
        let mut out = serde_json::Map::new();
        for (key, value) in [
            ("channelUserId", &self.channel_user_id),
            ("phoneE164", &self.phone_e164),
            ("email", &self.email),
            ("fullName", &self.full_name),
        ] {
            if let Some(v) = value {
                out.insert(key.into(), json!(v));
            }
        }
        Json::Object(out)
    }
}

/// `normalizeContactInboxSourceId`.
pub fn normalize_contact_inbox_source_id(channel: ChannelType, raw: &str) -> Option<String> {
    match channel {
        ChannelType::Whatsapp => {
            let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty()).then_some(digits)
        }
        ChannelType::Threads => Some(raw.to_lowercase()),
        ChannelType::Omnichannel => None,
        _ => Some(raw.to_owned()),
    }
}

/// `deriveContactInboxSourceId`.
pub fn derive_contact_inbox_source_id(channel: ChannelType, ids: &Identities) -> Option<String> {
    match channel {
        ChannelType::Whatsapp => ids
            .phone_e164
            .as_deref()
            .and_then(|p| normalize_contact_inbox_source_id(channel, p)),
        ChannelType::Smtp => ids.email.clone(),
        ChannelType::Webchat | ChannelType::Omnichannel => None,
        _ => ids
            .channel_user_id
            .as_deref()
            .and_then(|id| normalize_contact_inbox_source_id(channel, id)),
    }
}

/// `selectContactLookup`: the first usable identity in lookup order.
pub fn select_contact_lookup(
    ids: &Identities,
    channel: Option<ChannelType>,
) -> Option<(ContactIdentityKind, String)> {
    CONTACT_LOOKUP_ORDER.iter().find_map(|&kind| {
        let value = ids.get(kind).filter(|v| !v.is_empty())?;
        let usable = kind != ContactIdentityKind::ChannelUserId
            || channel.is_none_or(|c| normalize_contact_inbox_source_id(c, value).is_some());
        usable.then(|| (kind, value.to_owned()))
    })
}

/// `ExtractedField`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedField {
    pub target: String,
    /// `gender` / `richSystem` / `customField`.
    pub kind: &'static str,
    pub raw: Option<String>,
    pub value: Option<String>,
    pub error: Option<&'static str>,
}

/// `ExtractedIdentifier`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedIdentifier {
    pub kind: ContactIdentityKind,
    pub raw: Option<String>,
    pub normalized: Option<String>,
}

/// `ExtractedInboundWebhookValues`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub inbox_identifier: Option<String>,
    pub identifiers: Vec<ExtractedIdentifier>,
    pub identities: Identities,
    pub contact_lookup: Option<(ContactIdentityKind, String)>,
    pub fields: Vec<ExtractedField>,
}

impl Extracted {
    /// `previewMappingResource`.
    pub fn to_json(&self) -> Json {
        let identifiers: Vec<Json> = self
            .identifiers
            .iter()
            .map(|i| json!({ "kind": i.kind.as_str(), "raw": i.raw, "normalized": i.normalized }))
            .collect();
        let fields: Vec<Json> = self
            .fields
            .iter()
            .map(|f| {
                json!({ "target": f.target, "kind": f.kind, "raw": f.raw, "value": f.value, "error": f.error })
            })
            .collect();
        let lookup = self
            .contact_lookup
            .as_ref()
            .map(|(k, v)| json!({ "kind": k.as_str(), "value": v }));
        json!({
            "inboxIdentifier": self.inbox_identifier,
            "identifiers": identifiers,
            "identities": self.identities.to_json(),
            "contactLookup": lookup,
            "fields": fields,
        })
    }
}

/// `ExtractionContext`.
#[derive(Debug, Clone, Default)]
pub struct ExtractionContext {
    pub target_country: Option<String>,
    /// custom field id → `CustomFieldType`.
    pub custom_field_types: HashMap<String, String>,
}

fn to_text(value: Option<&Json>) -> Option<String> {
    stringify_json_path_value(value)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `readIdentifierText`: a JSON number that is not a safe integer counts as
/// absent (a rounded id names a different account).
fn read_identifier_text(payload: &Json, path: &str) -> Option<String> {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;
    let value = read_json_path(payload, path);
    if let Some(Json::Number(n)) = value {
        let safe = n
            .as_i64()
            .map(|i| i.unsigned_abs() <= MAX_SAFE)
            .or_else(|| n.as_u64().map(|u| u <= MAX_SAFE))
            .unwrap_or(false);
        if !safe {
            return None;
        }
    }
    to_text(value)
}

fn str_field<'a>(v: &'a Json, key: &str) -> &'a str {
    v.get(key).and_then(Json::as_str).unwrap_or_default()
}

/// `IDENTITY_NORMALIZERS`.
fn normalize_identity(
    kind: ContactIdentityKind,
    raw: &str,
    ctx: &ExtractionContext,
) -> Option<String> {
    match kind {
        ContactIdentityKind::ChannelUserId => {
            (raw.chars().count() <= MAX_CHANNEL_USER_ID_LENGTH).then(|| raw.to_owned())
        }
        ContactIdentityKind::Phone => {
            normalize_identity_phone_to_e164(raw, ctx.target_country.as_deref())
        }
        ContactIdentityKind::Email => crate::email_marketing::is_email(raw).then(|| raw.to_owned()),
        ContactIdentityKind::FullName => {
            let collapsed = raw
                .split([' ', '\t', '\n', '\u{b}', '\u{c}', '\r'])
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let n = collapsed.chars().count();
            (n > 0 && n <= MAX_FULL_NAME_LENGTH).then_some(collapsed)
        }
    }
}

/// `classifyMappedTarget`.
pub fn classify_mapped_target(target: &str) -> &'static str {
    if target == "gender" {
        "gender"
    } else if is_rich_system_contact_field(target) {
        "richSystem"
    } else {
        "customField"
    }
}

type Coerced = (Option<String>, Option<&'static str>);

fn coerce_phone(raw: &str, ctx: &ExtractionContext) -> Coerced {
    match normalize_identity_phone_to_e164(raw, ctx.target_country.as_deref()) {
        Some(e164) => (Some(e164), None),
        None => (None, Some("invalidPhone")),
    }
}

/// `FIELD_COERCERS`.
fn coerce_field(kind: &str, target: &str, raw: &str, ctx: &ExtractionContext, now: i64) -> Coerced {
    let ok = || (Some(raw.to_owned()), None);
    let invalid = (None, Some("invalidValue"));
    match kind {
        "gender" => {
            let gender = raw.to_lowercase();
            if GENDERS.contains(&gender.as_str()) {
                (Some(gender), None)
            } else {
                invalid
            }
        }
        "richSystem" => match target {
            "phone" | "phone_number" => coerce_phone(raw, ctx),
            "email" if !crate::email_marketing::is_email(raw) => invalid,
            _ => ok(),
        },
        _ => {
            let Some(field_type) = ctx.custom_field_types.get(target) else {
                return (None, Some("unknownField"));
            };
            let input = NormalizeInput {
                parsing: TemporalInputParsing::Lenient,
                ..NormalizeInput::new(field_type, raw)
            };
            match normalize_custom_field_value_for_storage(
                &input,
                &mut || Ok(String::from("UTC")),
                now,
            ) {
                Ok(Some(_)) => ok(),
                _ => invalid,
            }
        }
    }
}

/// `extractInboundWebhookValues`: reads the payload, writes nothing.
pub fn extract(
    payload: &Json,
    resolution: Option<&Json>,
    identifiers: &Json,
    field_mappings: &Json,
    ctx: &ExtractionContext,
    channel: Option<ChannelType>,
) -> Extracted {
    let now = chatbotx_db::repo::now_ms();
    let extracted: Vec<ExtractedIdentifier> = identifiers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let kind = ContactIdentityKind::parse(str_field(i, "kind"))?;
            let raw = read_identifier_text(payload, str_field(i, "jsonPath"));
            let normalized = raw
                .as_deref()
                .and_then(|r| normalize_identity(kind, r, ctx));
            Some(ExtractedIdentifier {
                kind,
                raw,
                normalized,
            })
        })
        .collect();
    let mut identities = Identities::default();
    for i in &extracted {
        if let Some(n) = &i.normalized {
            identities.set(i.kind, n.clone());
        }
    }
    let fields = field_mappings
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let target = str_field(m, "target").to_owned();
            let kind = classify_mapped_target(&target);
            let raw = to_text(read_json_path(payload, str_field(m, "jsonPath")));
            let (value, error) = match &raw {
                None => (None, None),
                Some(r) => coerce_field(kind, &target, r, ctx, now),
            };
            ExtractedField {
                target,
                kind,
                raw,
                value,
                error,
            }
        })
        .collect();
    let inbox_identifier = resolution
        .filter(|r| str_field(r, "mode") == "payload")
        .and_then(|r| read_identifier_text(payload, str_field(r, "jsonPath")));
    Extracted {
        inbox_identifier,
        contact_lookup: select_contact_lookup(&identities, channel),
        identifiers: extracted,
        identities,
        fields,
    }
}

/// `toFieldErrors`.
pub fn field_errors(fields: &[ExtractedField]) -> Vec<Json> {
    fields
        .iter()
        .filter_map(|f| {
            f.error
                .map(|code| json!({ "target": f.target, "code": code }))
        })
        .collect()
}

/// `loadExtractionContext`.
fn load_context(
    c: &Connection,
    workspace_id: i64,
    field_mappings: &Json,
) -> Result<ExtractionContext> {
    let target_country: Option<Option<String>> = c
        .query_row(
            r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo)?;
    let target_country =
        target_country.ok_or_else(|| BusinessError::not_found("Workspace not found"))?;
    let ids: Vec<i64> = field_mappings
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| str_field(m, "target"))
        .filter(|t| classify_mapped_target(t) == "customField")
        .filter_map(|t| {
            t.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| t.parse().ok())
                .flatten()
        })
        .collect();
    let mut custom_field_types = HashMap::new();
    if !ids.is_empty() {
        let mut stmt = c
            .prepare(r#"SELECT "id", "type" FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#)
            .map_err(repo)?;
        let rows = stmt
            .query_map(
                params![workspace_id, chatbotx_db::repo::id_list(&ids)],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(repo)?;
        for row in rows {
            let (id, t) = row.map_err(repo)?;
            custom_field_types.insert(id.to_string(), t);
        }
    }
    Ok(ExtractionContext {
        target_country,
        custom_field_types,
    })
}

/// An inbox of the workspace: `(id, channel, status)`.
type InboxRow = (i64, String, String);

fn find_inbox(c: &Connection, workspace_id: i64, id: i64) -> Result<Option<InboxRow>> {
    c.query_row(
        r#"SELECT "id", "channel", "status" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        [id, workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(repo)
}

fn id_of(v: Option<&Json>) -> Option<i64> {
    v.and_then(|v| {
        v.as_str()
            .map(str::to_owned)
            .or_else(|| v.as_i64().map(|n| n.to_string()))
    })
    .and_then(|s| s.parse().ok())
}

/// `resolveEffectiveChannel`: `None` when the chosen inbox is gone.
fn effective_channel(
    c: &Connection,
    workspace_id: i64,
    resolution: &Json,
) -> Result<Option<ChannelType>> {
    if str_field(resolution, "mode") == "payload" {
        return Ok(ChannelType::parse(str_field(resolution, "channel")));
    }
    let Some(id) = id_of(resolution.get("inboxId")) else {
        return Ok(None);
    };
    Ok(find_inbox(c, workspace_id, id)?.and_then(|(_, ch, _)| ChannelType::parse(&ch)))
}

/// `inboundWebhookService.previewMapping`.
pub async fn preview_mapping(
    db: &Db,
    workspace_id: i64,
    payload: Json,
    resolution: Option<Json>,
    identifiers: Json,
    field_mappings: Json,
) -> Result<Json> {
    tx::read(db, move |c| {
        let ctx = load_context(c, workspace_id, &field_mappings)?;
        let channel = match &resolution {
            Some(r) => effective_channel(c, workspace_id, r)?,
            None => None,
        };
        Ok(extract(
            &payload,
            resolution.as_ref(),
            &identifiers,
            &field_mappings,
            &ctx,
            channel,
        )
        .to_json())
    })
    .await
}

/// `resolveInboundWebhookInbox`: the inbox or a skip reason.
fn resolve_inbox(
    c: &Connection,
    workspace_id: i64,
    resolution: &Json,
    identifier: Option<&str>,
) -> Result<std::result::Result<InboxRow, &'static str>> {
    let pick = |candidates: Vec<InboxRow>| match candidates.len() {
        0 => Err("inboxNotFound"),
        1 if candidates[0].2 == "connected" => Ok(candidates[0].clone()),
        1 => Err("inboxDisconnected"),
        _ => Err("inboxAmbiguous"),
    };
    let by_ids = |ids: Vec<i64>| -> Result<Vec<InboxRow>> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(row) = find_inbox(c, workspace_id, id)? {
                out.push(row);
            }
        }
        Ok(out)
    };
    if str_field(resolution, "mode") != "payload" {
        return Ok(pick(by_ids(
            id_of(resolution.get("inboxId")).into_iter().collect(),
        )?));
    }
    let Some(identifier) = identifier else {
        return Ok(Err("missingInboxIdentifier"));
    };
    let channel = ChannelType::parse(str_field(resolution, "channel"));
    let kind = InboxIdentifierKind::parse(str_field(resolution, "identifierKind"));
    let (Some(channel), Some(kind)) = (channel, kind) else {
        return Ok(Err("inboxNotFound"));
    };
    let Some(strategy) = get_inbox_identifier_strategy(channel, kind) else {
        return Ok(Err("inboxNotFound"));
    };
    let value = match strategy {
        InboxLookupStrategy::WhatsappDisplayPhone => {
            identifier.chars().filter(char::is_ascii_digit).collect()
        }
        _ => identifier.to_owned(),
    };
    if value.is_empty() {
        return Ok(Err("missingInboxIdentifier"));
    }
    let candidates = match strategy {
        InboxLookupStrategy::InboxSourceId => {
            let id: Option<i64> = c
                .query_row(
                    r#"SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?1 AND "channel" = ?2 AND "sourceId" = ?3 LIMIT 1"#,
                    params![workspace_id, channel.as_str(), value],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo)?;
            by_ids(id.into_iter().collect())?
        }
        InboxLookupStrategy::ConnectionSourceId => {
            let conn = chatbotx_db::repo::connection::find_by_provider_source_id(
                c,
                workspace_id,
                channel.as_str(),
                &value,
            )?;
            by_ids(conn.and_then(|c| c.inbox_id).into_iter().collect())?
        }
        InboxLookupStrategy::WhatsappDisplayPhone => {
            let rows = chatbotx_db::repo::integration_whatsapp::list_by_display_phone_digits(
                c,
                workspace_id,
                &value,
            )?;
            by_ids(rows.into_iter().map(|r| r.inbox_id).collect())?
        }
    };
    Ok(pick(candidates))
}

/// `(contactId, contactInboxId)`.
type Found = (i64, i64);

/// `LookupOutcome`: `Err(())` = ambiguous.
type Lookup = std::result::Result<Option<Found>, ()>;

/// `findLatestBySource` (workspace-scoped).
fn find_by_source_id(
    c: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    source_id: Option<String>,
) -> Result<Option<Found>> {
    let Some(source_id) = source_id else {
        return Ok(None);
    };
    c.query_row(
        r#"SELECT ci."contactId", ci."id" FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
           WHERE ci."inboxId" = ?1 AND ci."sourceId" = ?2 AND i."workspaceId" = ?3
           ORDER BY ci."lastMessageAt" DESC NULLS LAST LIMIT 1"#,
        params![inbox_id, source_id, workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(repo)
}

/// `CONTACT_LOOKUPS`.
fn lookup(
    c: &Connection,
    ws: i64,
    inbox: (i64, ChannelType),
    kind: ContactIdentityKind,
    value: &str,
) -> Result<Lookup> {
    let (inbox_id, channel) = inbox;
    let by_profile = |ids: Identities, field: IdentityField| -> Result<Lookup> {
        Ok(Ok(match derive_contact_inbox_source_id(channel, &ids) {
            Some(source) => find_by_source_id(c, ws, inbox_id, Some(source))?,
            None => find_latest_by_inbox_and_contact_identity(c, ws, inbox_id, field, value)?
                .map(|r| (r.contact_id, r.contact_inbox_id)),
        }))
    };
    match kind {
        ContactIdentityKind::ChannelUserId => Ok(Ok(find_by_source_id(
            c,
            ws,
            inbox_id,
            normalize_contact_inbox_source_id(channel, value),
        )?)),
        ContactIdentityKind::Phone => by_profile(
            Identities {
                phone_e164: Some(value.into()),
                ..Identities::default()
            },
            IdentityField::PhoneNumber,
        ),
        ContactIdentityKind::Email => by_profile(
            Identities {
                email: Some(value.into()),
                ..Identities::default()
            },
            IdentityField::Email,
        ),
        ContactIdentityKind::FullName => {
            let rows = find_by_inbox_and_full_name(c, ws, inbox_id, value)?;
            Ok(if rows.len() > 1 {
                Err(())
            } else {
                Ok(rows.first().map(|r| (r.contact_id, r.contact_inbox_id)))
            })
        }
    }
}

/// `findCreationOwner`.
fn creation_owner(
    c: &Connection,
    ws: i64,
    inbox: (i64, ChannelType),
    ids: &Identities,
) -> Result<Lookup> {
    let (inbox_id, channel) = inbox;
    if let Some(source) = derive_contact_inbox_source_id(channel, ids) {
        return Ok(Ok(find_by_source_id(c, ws, inbox_id, Some(source))?));
    }
    let creation = inbound_webhook_channel_spec(channel).creation_identities;
    let first = CONTACT_LOOKUP_ORDER
        .iter()
        .find(|k| creation.contains(k) && ids.get(**k).is_some_and(|v| !v.is_empty()));
    match first {
        Some(&kind) => lookup(c, ws, inbox, kind, ids.get(kind).unwrap_or_default()),
        None => Ok(Ok(None)),
    }
}

/// The new contact's name: the full name, then the mapped system fields.
fn new_contact_name(
    full_name: Option<&str>,
    writable: &[ExtractedField],
) -> (Option<String>, Option<String>) {
    let (mut first, mut last) = (None, None);
    let mut apply = |field: &str, value: &str| {
        for (column, v) in rich_system_columns(field, value) {
            match column {
                "firstName" => first = v,
                "lastName" => last = v,
                _ => {}
            }
        }
    };
    if let Some(name) = full_name {
        apply("full_name", name);
    }
    for (target, value) in rich_fields(writable) {
        apply(&target, &value);
    }
    (first, last)
}

/// `toRichSystemFields`: later rows win and move to the end.
fn rich_fields(fields: &[ExtractedField]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for f in fields.iter().filter(|f| f.kind == "richSystem") {
        if let Some(v) = &f.value {
            out.retain(|(t, _)| *t != f.target);
            out.push((f.target.clone(), v.clone()));
        }
    }
    out
}

/// `ApplyInboundWebhookPayloadResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    Processed {
        inbox_id: i64,
        contact_id: i64,
        contact_inbox_id: i64,
        is_new: bool,
        field_errors: Vec<Json>,
    },
    Skipped {
        reason: &'static str,
        inbox_id: Option<i64>,
        field_errors: Vec<Json>,
    },
}

/// `applyInboundWebhookPayload`: resolves the inbox, finds or creates the
/// contact, writes the mapped fields. Safe to repeat (lookups and upserts).
pub async fn apply_payload(db: &Db, webhook: &InboundWebhook, payload: Json) -> Result<Applied> {
    let ws = webhook.workspace_id;
    let resolution = webhook
        .inbox_resolution
        .clone()
        .map(|j| j.0)
        .filter(|v| !v.is_null());
    let identifiers = webhook.identifiers.0.clone();
    let mappings = webhook.field_mappings.0.clone();
    tx::write(db, move |c| {
        let Some(resolution) = resolution else {
            return Ok(Applied::Skipped {
                reason: "webhookInactive",
                inbox_id: None,
                field_errors: Vec::new(),
            });
        };
        let ctx = load_context(c, ws, &mappings)?;
        let extracted = extract(
            &payload,
            Some(&resolution),
            &identifiers,
            &mappings,
            &ctx,
            None,
        );
        let field_errors = field_errors(&extracted.fields);
        let inbox = match resolve_inbox(c, ws, &resolution, extracted.inbox_identifier.as_deref())?
        {
            Ok(inbox) => inbox,
            Err(reason) => {
                return Ok(Applied::Skipped {
                    reason,
                    inbox_id: None,
                    field_errors,
                });
            }
        };
        let skip = |reason| Applied::Skipped {
            reason,
            inbox_id: Some(inbox.0),
            field_errors: field_errors.clone(),
        };
        let Some(channel) = ChannelType::parse(&inbox.1) else {
            return Ok(skip("missingIdentifier"));
        };
        let ids = &extracted.identities;
        let Some((kind, value)) = select_contact_lookup(ids, Some(channel)) else {
            return Ok(skip("missingIdentifier"));
        };
        let key = (inbox.0, channel);
        let writable: Vec<ExtractedField> = extracted
            .fields
            .iter()
            .filter(|f| f.value.is_some() && f.error.is_none())
            .cloned()
            .collect();
        let existing = match lookup(c, ws, key, kind, &value)? {
            Err(()) => return Ok(skip("contactAmbiguous")),
            Ok(Some(found)) => Some(found),
            Ok(None) => {
                let creation = inbound_webhook_channel_spec(channel).creation_identities;
                if !creation
                    .iter()
                    .any(|k| ids.get(*k).is_some_and(|v| !v.is_empty()))
                {
                    return Ok(skip("contactNotFound"));
                }
                match creation_owner(c, ws, key, ids)? {
                    Err(()) => return Ok(skip("contactAmbiguous")),
                    found => found.ok().flatten(),
                }
            }
        };
        let (contact_id, contact_inbox_id, is_new) = match existing {
            Some((contact, ci)) => (contact, ci, false),
            None => {
                let (first_name, last_name) = new_contact_name(ids.full_name.as_deref(), &writable);
                let created = crate::contact::create::create_with_inbox_in(
                    c,
                    ws,
                    &CreateContactInput {
                        inbox_id: inbox.0,
                        channel: inbox.1.clone(),
                        contact_id: ids.channel_user_id.clone(),
                        phone_number: ids.phone_e164.clone(),
                        email: ids.email.clone().unwrap_or_default(),
                        gender: "unknown".into(),
                        first_name,
                        last_name,
                        source_user_id: None,
                    },
                    &CreateOptions {
                        source: "inboundWebhook".into(),
                        phone_uniqueness: PhoneUniqueness::Inbox,
                    },
                )?;
                (created.contact.id, created.contact_inbox.id, true)
            }
        };
        apply_mapped_fields(c, ws, contact_id, &writable)?;
        Ok(Applied::Processed {
            inbox_id: inbox.0,
            contact_id,
            contact_inbox_id,
            is_new,
            field_errors,
        })
    })
    .await
}

/// `applyMappedContactFields`: system fields, gender, custom fields.
fn apply_mapped_fields(
    c: &Connection,
    ws: i64,
    contact_id: i64,
    fields: &[ExtractedField],
) -> Result<()> {
    let mut columns: Vec<(&'static str, Option<String>)> = Vec::new();
    for (target, value) in rich_fields(fields) {
        for (column, v) in rich_system_columns(&target, &value) {
            columns.retain(|(c, _)| *c != column);
            columns.push((column, v));
        }
    }
    if let Some(gender) = fields
        .iter()
        .rev()
        .find(|f| f.kind == "gender")
        .and_then(|f| f.value.clone())
    {
        columns.push(("gender", Some(gender)));
    }
    if !columns.is_empty() {
        crate::contact::service::update_text_columns(c, ws, contact_id, &columns)?;
    }
    let custom: Vec<(i64, String)> = fields
        .iter()
        .filter(|f| f.kind == "customField")
        .filter_map(|f| Some((f.target.parse().ok()?, f.value.clone()?)))
        .collect();
    if !custom.is_empty() {
        let input = SetValuesInput {
            workspace_id: ws,
            contact_id,
            fields: custom,
            temporal_input_parsing: Some(TemporalInputParsing::Lenient),
            ..SetValuesInput::default()
        };
        set_values_in_transaction(c, &input, chatbotx_db::repo::now_ms())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ExtractionContext {
        ExtractionContext {
            target_country: Some("US".into()),
            custom_field_types: HashMap::from([("7".into(), "number".into())]),
        }
    }

    /// `inbound-webhook-extract.test.ts`: identities, lookup order and verdicts.
    #[test]
    fn extracts_identities_and_field_verdicts() {
        let payload = json!({
            "user": { "id": 12, "email": "a@b.co", "name": "  Ada \t Lovelace " },
            "phone": "N/A", "gender": "FEMALE", "age": "x", "big": 9_007_199_254_740_993_u64,
        });
        let identifiers = json!([
            { "kind": "phone", "jsonPath": "$.phone" },
            { "kind": "email", "jsonPath": "$.user.email" },
            { "kind": "fullName", "jsonPath": "$.user.name" },
            { "kind": "channelUserId", "jsonPath": "$.big" },
        ]);
        let mappings = json!([
            { "jsonPath": "$.gender", "target": "gender" },
            { "jsonPath": "$.age", "target": "7" },
            { "jsonPath": "$.age", "target": "99" },
            { "jsonPath": "$.missing", "target": "email" },
            { "jsonPath": "$.phone", "target": "phone" },
        ]);
        let out = extract(&payload, None, &identifiers, &mappings, &ctx(), None);
        assert_eq!(out.identifiers[0].normalized, None);
        assert_eq!(out.identifiers[3].raw, None, "an unsafe integer is absent");
        assert_eq!(out.identities.full_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            out.contact_lookup,
            Some((ContactIdentityKind::Email, "a@b.co".into()))
        );
        let verdicts: Vec<_> = out
            .fields
            .iter()
            .map(|f| (f.value.clone(), f.error))
            .collect();
        assert_eq!(
            verdicts,
            vec![
                (Some("female".into()), None),
                (None, Some("invalidValue")),
                (None, Some("unknownField")),
                (None, None),
                (None, Some("invalidPhone")),
            ]
        );
        assert_eq!(field_errors(&out.fields).len(), 3);
    }

    /// `contact-inbox-identity`: source ids per channel and the lookup key.
    #[test]
    fn source_ids_and_lookup_key() {
        let ids = Identities {
            channel_user_id: Some("abc".into()),
            phone_e164: Some("+1 555".into()),
            ..Identities::default()
        };
        assert_eq!(
            derive_contact_inbox_source_id(ChannelType::Whatsapp, &ids).as_deref(),
            Some("1555")
        );
        assert_eq!(
            derive_contact_inbox_source_id(ChannelType::Webchat, &ids),
            None
        );
        assert_eq!(
            normalize_contact_inbox_source_id(ChannelType::Threads, "AbC").as_deref(),
            Some("abc")
        );
        // An id the channel cannot use is skipped like an absent value.
        assert_eq!(
            select_contact_lookup(&ids, Some(ChannelType::Omnichannel)).map(|l| l.0),
            Some(ContactIdentityKind::Phone)
        );
        assert_eq!(
            select_contact_lookup(&ids, None).map(|l| l.0),
            Some(ContactIdentityKind::ChannelUserId)
        );
    }

    /// `buildNewContactName`: mapped name fields over the full name.
    #[test]
    fn new_contact_name_layers_mapped_fields() {
        let field = |target: &str, value: &str| ExtractedField {
            target: target.into(),
            kind: "richSystem",
            raw: Some(value.into()),
            value: Some(value.into()),
            error: None,
        };
        assert_eq!(
            new_contact_name(Some("Ada Lovelace"), &[field("first_name", "Augusta")]),
            (Some("Augusta".into()), Some("Lovelace".into()))
        );
        assert_eq!(new_contact_name(None, &[]), (None, None));
    }
}
