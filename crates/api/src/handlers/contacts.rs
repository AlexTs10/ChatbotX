//! `features/contacts/api/public/crud.ts` (P3.6-D2 slice contacts core):
//! `contacts.get / create / update / updateLegacy / delete / block /
//! unblock / upsert / findByCustomField`. The list, count, search, import and sub-resource
//! routes are not ported yet.

use chatbotx_business::BusinessError;
use chatbotx_business::contact::avatar::{AvatarInbox, TenantUrls, resolve_contact_avatar_url};
use chatbotx_business::contact::{
    CreateContactInput, CreateOptions, UpsertData, block_and_record, create_with_inbox,
    delete_and_record, find_public_contact_or_fail, list_by_custom_field_value,
    resolve_id_by_identifier, unblock_and_record, update_fields_and_custom_fields,
    upsert_by_identifier,
};
use chatbotx_db::repo::contact::{ContactInboxWithInbox, ConversationWithAssignees, PublicContact};
use chatbotx_db::repo::models::{Contact, ContactCustomField, Inbox, Tag};
use serde_json::{Map, Value, json};

use super::input::{Input, Str, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, date, id, json, no_content,
    opt_date, opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.get",
    "contacts.create",
    "contacts.update",
    "contacts.updateLegacy",
    "contacts.delete",
    "contacts.block",
    "contacts.unblock",
    "contacts.upsert",
    "contacts.findByCustomField",
    "contacts.list",
    "contacts.search",
    "contacts.count",
];

/// `possibleErrorsOnCreatingContact`.
const CREATING_CONTACT: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
    ("invalidCustomFieldValue", 400),
];
/// `possibleErrorsOnUpsertingContact`.
const UPSERTING_CONTACT: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
    ("invalidCustomFieldValue", 400),
    ("phoneExists", 422),
];
/// `possibleErrorsOnWritingContactFields`.
const WRITING_FIELDS: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("invalidCustomFieldValue", 400),
    ("invalidFieldOperation", 400),
];

const CHANNEL_TYPES: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];
const GENDER_TYPES: &[&str] = &["male", "female", "unknown"];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.get" => get(state, call).await,
        "contacts.create" => create(state, call).await,
        "contacts.update" => update(state, call, WRITING_FIELDS).await,
        "contacts.updateLegacy" => update(state, call, MUTATING).await,
        "contacts.delete" => delete(state, call).await,
        "contacts.block" => block(state, call, true).await,
        "contacts.unblock" => block(state, call, false).await,
        "contacts.upsert" => upsert(state, call).await,
        "contacts.findByCustomField" => find_by_custom_field(state, call).await,
        "contacts.list" => list(state, call, false).await,
        "contacts.search" => list(state, call, true).await,
        "contacts.count" => count(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The tenant URLs of the public avatar read.
pub fn tenant_urls(state: &ApiState) -> TenantUrls {
    let storage = std::env::var("NEXT_PUBLIC_STORAGE_URL").ok();
    TenantUrls::from_app_url(&state.app_url, storage.as_deref())
}

// ---- output ---------------------------------------------------------------

fn contact_json(contact: &Contact) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), id(contact.id));
    out.insert("createdAt".into(), date(contact.created_at));
    out.insert("updatedAt".into(), date(contact.updated_at));
    out.insert("avatar".into(), json!(contact.avatar));
    out.insert("phoneNumber".into(), json!(contact.phone_number));
    out.insert("email".into(), json!(contact.email));
    out.insert("emailVerified".into(), json!(contact.email_verified));
    out.insert("emailOptIn".into(), json!(contact.email_opt_in));
    out.insert("firstName".into(), json!(contact.first_name));
    out.insert("lastName".into(), json!(contact.last_name));
    out.insert("fullName".into(), json!(contact.full_name));
    out.insert("gender".into(), json!(contact.gender));
    out.insert("lastReadAt".into(), opt_date(contact.last_read_at));
    out.insert("ref".into(), json!(contact.r#ref));
    out.insert("country".into(), json!(contact.country));
    out.insert("state".into(), json!(contact.state));
    out.insert("city".into(), json!(contact.city));
    out.insert(
        "location".into(),
        contact
            .location
            .as_ref()
            .map_or(Value::Null, |l| l.0.clone()),
    );
    out.insert("locale".into(), json!(contact.locale));
    out.insert("timezone".into(), json!(contact.timezone));
    out.insert("subscribedAt".into(), opt_date(contact.subscribed_at));
    out.insert(
        "broadcastSubscribedAt".into(),
        opt_date(contact.broadcast_subscribed_at),
    );
    out.insert("blockedAt".into(), opt_date(contact.blocked_at));
    out.insert("totalTagged".into(), json!(contact.total_tagged));
    out.insert("totalNewTagged".into(), json!(contact.total_new_tagged));
    out.insert("workspaceId".into(), id(contact.workspace_id));
    out
}

/// `inboxResource`.
pub fn inbox_json(inbox: &Inbox) -> Value {
    json!({
        "id": id(inbox.id),
        "createdAt": date(inbox.created_at),
        "updatedAt": date(inbox.updated_at),
        "name": inbox.name,
        "channel": inbox.channel,
        "sourceId": inbox.source_id,
        "workspaceId": id(inbox.workspace_id),
        "status": inbox.status,
        "markReadOnOutbound": inbox.mark_read_on_outbound,
        "disconnectedAt": opt_date(inbox.disconnected_at),
        "disconnectReason": inbox.disconnect_reason,
        "threadControlSeenAt": opt_date(inbox.thread_control_seen_at),
    })
}

fn contact_inbox_json(row: &ContactInboxWithInbox) -> Value {
    let ci = &row.contact_inbox;
    json!({
        "id": id(ci.id),
        "contactId": id(ci.contact_id),
        "inboxId": id(ci.inbox_id),
        "channel": ci.channel,
        "source": ci.source,
        "sourceId": ci.source_id,
        "sourceUserId": ci.source_user_id,
        "sourceUsername": ci.source_username,
        "language": ci.language,
        "contactLastReadAt": opt_date(ci.contact_last_read_at),
        "lastIncomingMessageAt": opt_date(ci.last_incoming_message_at),
        "inbox": inbox_json(&row.inbox),
    })
}

fn conversation_json(row: &ConversationWithAssignees) -> Value {
    let c = &row.conversation;
    json!({
        "id": id(c.id),
        "createdAt": date(c.created_at),
        "updatedAt": date(c.updated_at),
        "botEnabled": c.bot_enabled,
        "botResumeAt": opt_date(c.bot_resume_at),
        "archivedAt": opt_date(c.archived_at),
        "additionalAttributes": c.additional_attributes.as_ref().map_or(Value::Null, |a| a.0.clone()),
        "contactLastReadAt": opt_date(c.contact_last_read_at),
        "agentLastReadAt": opt_date(c.agent_last_read_at),
        "aiContextLastMessageId": opt_id(c.ai_context_last_message_id),
        "lastActivityAt": opt_date(c.last_activity_at),
        "followed": c.followed,
        "assignedUserId": opt_id(c.assigned_user_id),
        "assignedInboxTeamId": opt_id(c.assigned_inbox_team_id),
        "workspaceId": id(c.workspace_id),
        "contactId": id(c.contact_id),
        "sourceId": c.source_id,
        "lastStep": c.last_step,
        "currentStep": c.current_step,
        "adminRepliedAt": opt_date(c.admin_replied_at),
        "contactRepliedAt": opt_date(c.contact_replied_at),
        "awaitingHumanReplySince": opt_date(c.awaiting_human_reply_since),
        "assignedUserAt": opt_date(c.assigned_user_at),
        "responseDueAt": opt_date(c.response_due_at),
        "responseSlaBreachingAt": opt_date(c.response_sla_breaching_at),
        "responseSlaTargetSeconds": c.response_sla_target_seconds,
        "assignedUser": row.assigned_user.as_ref().map_or(Value::Null, |u| json!({
            "id": id(u.id),
            "name": u.name,
            "email": u.email,
        })),
        "assignedInboxTeam": row.assigned_inbox_team.as_ref().map_or(Value::Null, |t| json!({
            "id": id(t.id),
            "createdAt": date(t.created_at),
            "updatedAt": date(t.updated_at),
            "workspaceId": id(t.workspace_id),
            "name": t.name,
        })),
    })
}

fn tag_json(tag: &Tag) -> Value {
    json!({
        "id": id(tag.id),
        "createdAt": date(tag.created_at),
        "updatedAt": date(tag.updated_at),
        "name": tag.name,
        "deletedAt": opt_date(tag.deleted_at),
        "folderId": opt_id(tag.folder_id),
        "workspaceId": id(tag.workspace_id),
    })
}

fn custom_field_json(value: &ContactCustomField) -> Value {
    json!({
        "id": id(value.id),
        "createdAt": date(value.created_at),
        "updatedAt": date(value.updated_at),
        "value": value.value,
        "contactId": id(value.contact_id),
        "customFieldId": id(value.custom_field_id),
    })
}

/// `contactResponse` of a public contact; `resolve_avatar` = the
/// `resolveContactAvatars(..., { publicUrls: true })` of the read routes.
pub fn public_contact_json(
    state: &ApiState,
    contact: &PublicContact,
    resolve_avatar: bool,
) -> Value {
    let mut out = contact_json(&contact.contact);
    if resolve_avatar {
        let inboxes: Vec<AvatarInbox> = contact
            .contact_inboxes
            .iter()
            .map(|row| AvatarInbox {
                id: row.contact_inbox.id,
                channel: row.contact_inbox.channel.clone(),
                last_message_at: row.contact_inbox.last_message_at,
            })
            .collect();
        let avatar = resolve_contact_avatar_url(
            contact.contact.workspace_id,
            contact.contact.avatar.as_deref(),
            &inboxes,
            &tenant_urls(state),
            now_ms(),
        );
        out.insert("avatar".into(), json!(avatar));
    }
    out.insert(
        "contactCustomFields".into(),
        contact
            .contact_custom_fields
            .iter()
            .map(custom_field_json)
            .collect(),
    );
    out.insert("tags".into(), contact.tags.iter().map(tag_json).collect());
    out.insert(
        "contactInboxes".into(),
        contact
            .contact_inboxes
            .iter()
            .map(contact_inbox_json)
            .collect(),
    );
    out.insert(
        "conversation".into(),
        contact
            .conversation
            .as_ref()
            .map_or(Value::Null, conversation_json),
    );
    Value::Object(out)
}

// ---- input ----------------------------------------------------------------

/// `z.email()` (zod 4 default pattern).
pub fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    if text.contains("..") || local.is_empty() || local.starts_with('.') || domain.contains('@') {
        return false;
    }
    let local_ok = local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_'+-.".contains(c))
        && local
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || "_+-".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let Some((tld, rest)) = labels.split_last() else {
        return false;
    };
    let labels_ok = !rest.is_empty()
        && rest.iter().all(|label| {
            label
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    local_ok && labels_ok && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
}

/// `/\+?\d{10,20}/` (unanchored: ten digits in a row).
fn has_ten_digits(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 10 {
            return true;
        }
    }
    false
}

/// `WHATSAPP_BSUID_PATTERN` `/^(?!\d+$)[A-Za-z0-9._-]+$/`.
fn is_bsuid(text: &str) -> bool {
    !text.is_empty()
        && !text.chars().all(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// `z.union([z.literal(""), z.email().max(100)])`.
fn email_union(input: &mut Input, name: &str, required: bool) -> Option<String> {
    match input.obj.get(name).cloned() {
        None => {
            if required {
                input.issue(&key(name), "invalid_union", "Invalid input");
            }
            None
        }
        Some(Value::String(text)) if text.is_empty() => Some(text),
        Some(Value::String(text)) if is_email(&text) && text.encode_utf16().count() <= 100 => {
            Some(text)
        }
        Some(_) => {
            input.issue(&key(name), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `z.string().min(10).max(20).regex(/\+?\d{10,20}/)` (+ `""` in a union).
fn phone_field(input: &mut Input, name: &str, allow_empty: bool) -> Option<String> {
    match input.obj.get(name).cloned() {
        None => None,
        Some(Value::String(text)) if allow_empty && text.is_empty() => Some(text),
        Some(Value::String(text)) => {
            let len = text.encode_utf16().count();
            if (10..=20).contains(&len) && has_ten_digits(&text) {
                Some(text)
            } else {
                let code = if allow_empty {
                    "invalid_union"
                } else {
                    "invalid_format"
                };
                input.issue(&key(name), code, "Invalid input");
                None
            }
        }
        Some(_) => {
            input.issue(&key(name), "invalid_type", "Invalid input: expected string");
            None
        }
    }
}

fn identifier(call: &Call<'_>) -> String {
    call.params
        .iter()
        .find(|(name, _)| *name == "identifier")
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

/// `createContactRequest` (+ its `superRefine`).
fn parse_create(input: &mut Input) -> Option<CreateContactInput> {
    let phone_number = phone_field(input, "phoneNumber", true);
    let source_user_id = match input.obj.get("sourceUserId").cloned() {
        None => None,
        Some(Value::String(text)) if text.is_empty() => Some(text),
        Some(Value::String(text)) => {
            let trimmed = chatbotx_core::js::trim(&text).to_string();
            if trimmed.encode_utf16().count() <= 255 && is_bsuid(&trimmed) {
                Some(trimmed)
            } else {
                input.issue(&key("sourceUserId"), "invalid_union", "Invalid input");
                None
            }
        }
        Some(_) => {
            input.issue(&key("sourceUserId"), "invalid_union", "Invalid input");
            None
        }
    };
    let email = email_union(input, "email", true);
    let contact_id = input.opt_string("contactId", Str::max(255));
    let first_name = input.opt_string(
        "firstName",
        Str {
            trim: true,
            min: None,
            max: Some(100),
            pattern: None,
        },
    );
    let last_name = input.opt_string(
        "lastName",
        Str {
            trim: true,
            min: None,
            max: Some(100),
            pattern: None,
        },
    );
    let gender_value = input.obj.get("gender").cloned();
    let gender = input.enum_at(&key("gender"), gender_value.as_ref(), GENDER_TYPES, true);
    let channel_value = input.obj.get("channel").cloned();
    let channel = input.enum_at(&key("channel"), channel_value.as_ref(), CHANNEL_TYPES, true);
    let inbox_id = input.id("inboxId");
    if input.failed() {
        return None;
    }
    let channel = channel?;
    let has = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.is_empty());
    if channel != "whatsapp" && has(&source_user_id) {
        input.issue(
            &key("sourceUserId"),
            "custom",
            "BSUID is only supported for WhatsApp",
        );
    }
    if channel == "whatsapp" {
        if !(has(&phone_number) || has(&source_user_id)) {
            input.issue(
                &key("phoneNumber"),
                "custom",
                "Enter a phone number or a BSUID for WhatsApp",
            );
        }
    } else if channel == "smtp" {
        if !has(&email) {
            input.issue(
                &key("email"),
                "custom",
                "Email is required for the Email channel",
            );
        }
    } else if channel != "webchat" && channel != "omnichannel" && !has(&contact_id) {
        input.issue(
            &key("contactId"),
            "custom",
            "User ID is required for this channel",
        );
    }
    if input.failed() {
        return None;
    }
    Some(CreateContactInput {
        phone_number,
        source_user_id,
        email: email?,
        contact_id,
        first_name,
        last_name,
        gender: gender?,
        channel,
        inbox_id: inbox_id?,
    })
}

// ---- handlers -------------------------------------------------------------

async fn read_public(
    state: &ApiState,
    workspace_id: i64,
    contact_id: i64,
    errors: ErrorMap,
    resolve: bool,
) -> Reply {
    let contact = find_public_contact_or_fail(&state.db, workspace_id, contact_id)
        .await
        .map_err(|e| business_error(e, errors))?;
    Ok(json(200, public_contact_json(state, &contact, resolve)))
}

pub(super) async fn resolve(
    state: &ApiState,
    call: &Call<'_>,
    errors: ErrorMap,
) -> Result<i64, crate::orpc::OrpcError> {
    resolve_id_by_identifier(&state.db, call.workspace_id, &identifier(call))
        .await
        .map_err(|e| business_error(e, errors))
}

async fn get(state: &ApiState, call: Call<'_>) -> Reply {
    let contact_id = resolve(state, &call, FINDING).await?;
    read_public(state, call.workspace_id, contact_id, FINDING, true).await
}

async fn create(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let parsed = parse_create(&mut input);
    input.finish()?;
    let Some(parsed) = parsed else {
        return Err(business_error(
            BusinessError::validation("body", "Invalid input"),
            CREATING_CONTACT,
        ));
    };
    let created = create_with_inbox(
        &state.db,
        call.workspace_id,
        parsed,
        CreateOptions::default(),
    )
    .await
    .map_err(|e| business_error(e, CREATING_CONTACT))?;
    read_public(
        state,
        call.workspace_id,
        created.contact.id,
        CREATING_CONTACT,
        false,
    )
    .await
}

async fn update(state: &ApiState, call: Call<'_>, errors: ErrorMap) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let mut fields = Vec::new();
    let entries: Vec<(String, Value)> = input
        .obj
        .iter()
        .filter(|(name, _)| name.as_str() != "identifier")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    for (name, value) in entries {
        match value {
            Value::String(text) => fields.push((name, text)),
            other => {
                let received = match other {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::Array(_) => "array",
                    _ => "object",
                };
                input.issue(
                    &key(&name),
                    "invalid_type",
                    &format!("Invalid input: expected string, received {received}"),
                );
            }
        }
    }
    input.finish()?;
    let contact_id = resolve(state, &call, errors).await?;
    update_fields_and_custom_fields(&state.db, call.workspace_id, contact_id, fields)
        .await
        .map_err(|e| business_error(e, errors))?;
    Ok(no_content())
}

async fn delete(state: &ApiState, call: Call<'_>) -> Reply {
    let contact_id = resolve(state, &call, MUTATING).await?;
    delete_and_record(&state.db, call.workspace_id, &[contact_id])
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    Ok(no_content())
}

async fn block(state: &ApiState, call: Call<'_>, blocked: bool) -> Reply {
    let contact_id = resolve(state, &call, MUTATING).await?;
    let outcome = if blocked {
        block_and_record(&state.db, call.workspace_id, contact_id, now_ms()).await
    } else {
        unblock_and_record(&state.db, call.workspace_id, contact_id).await
    };
    outcome.map_err(|e| business_error(e, MUTATING))?;
    Ok(no_content())
}

async fn upsert(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let trimmed = Str {
        trim: true,
        min: None,
        max: Some(100),
        pattern: None,
    };
    let first_name = input.opt_string("firstName", trimmed);
    let last_name = input.opt_string("lastName", trimmed);
    let email = email_union(&mut input, "email", false);
    let phone_number = phone_field(&mut input, "phoneNumber", false);
    let avatar = input.opt_string("avatar", Str::default());
    let gender_value = input.obj.get("gender").cloned();
    let gender = input.enum_at(&key("gender"), gender_value.as_ref(), GENDER_TYPES, false);
    input.finish()?;
    let data = UpsertData {
        first_name,
        last_name,
        email,
        phone_number,
        gender,
    };
    let outcome = upsert_by_identifier(
        &state.db,
        call.workspace_id,
        &identifier(&call),
        data,
        "api",
        avatar,
    )
    .await
    .map_err(|e| business_error(e, UPSERTING_CONTACT))?;
    read_public(
        state,
        call.workspace_id,
        outcome.contact.id,
        UPSERTING_CONTACT,
        false,
    )
    .await
}

/// `contacts.findByCustomField` (deprecated): `{ data }` without the avatar
/// resolution.
async fn find_by_custom_field(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let custom_field_id = input.string("customFieldId", Str::default());
    let value = input.string("value", Str::default());
    input.finish()?;
    let (Some(custom_field_id), Some(value)) = (custom_field_id, value) else {
        return Err(crate::routes::not_ported_error());
    };
    let contacts =
        list_by_custom_field_value(&state.db, call.workspace_id, &custom_field_id, &value)
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
    let data: Vec<Value> = contacts
        .iter()
        .map(|contact| public_contact_json(state, contact, false))
        .collect();
    Ok(json(200, json!({ "data": data })))
}

// ---- list / search / count ------------------------------------------------

/// `contacts.list` (GET, query) / `contacts.search` (POST, body):
/// `listContactsPublicRequest`, the avatars resolved as public URLs.
async fn list(state: &ApiState, call: Call<'_>, from_body: bool) -> Reply {
    let _ = (state, call, from_body);
    todo!("P3.6-D2")
}

/// `contacts.count`: `countContactsPublicRequest`.
async fn count(state: &ApiState, call: Call<'_>) -> Reply {
    let _ = (state, call);
    todo!("P3.6-D2")
}
