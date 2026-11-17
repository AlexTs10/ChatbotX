//! The WhatsApp message routes of `features/messages` (Q.I1a slice 4):
//! `requestWhatsappCallPermission` (`calling/lib/request-call-permission.ts`,
//! `meta-call-permission.ts`), `sendWhatsappTemplateToConversation`
//! (`lib/send-whatsapp-template.ts`) with `resolveTemplateParams`
//! (`template-params/service.ts`), and the Flow response view
//! (`lib/get-whatsapp-flow-response.ts`, `build-whatsapp-flow-response-fields.ts`).

use std::time::Duration;

use chatbotx_core::flow_config::steps::{
    TemplateParameterValues, apply_wa_template_parameter_values, validate_wa_template_send_params,
    wa_template_params_schema,
};
use chatbotx_core::js;
use chatbotx_db::Db;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{
    ContactInbox, Conversation, IntegrationWhatsapp, WhatsappMessageTemplate,
};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::QueueName;
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result, sanitize_public_text};
use crate::message::{OutgoingInput, OutgoingJob, contact_inbox_json, conversation_json};
use crate::tx;

/// `GRAPH_API_URL` / `DEFAULT_API_VERSION` of `integrations/whatsapp`.
pub const GRAPH_API_URL: &str = "https://graph.facebook.com";
pub const DEFAULT_API_VERSION: &str = "v23.0";

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `shouldAddressBySourceUserId`: a BSUID-keyed contact inbox.
pub fn should_address_by_source_user_id(source_id: &str, source_user_id: Option<&str>) -> bool {
    match source_user_id.filter(|u| !u.is_empty()) {
        Some(user) => source_id == user || source_id.is_empty(),
        None => false,
    }
}

/// `whatsappMessageTemplateService.findByIdForWorkspace`.
pub async fn find_template_for_workspace(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<Option<WhatsappMessageTemplate>> {
    tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT {} FROM "WhatsappMessageTemplate" t
                   JOIN "IntegrationWhatsapp" w ON w."id" = t."integrationWhatsappId"
                   WHERE t."id" = ?1 AND w."workspaceId" = ?2"#,
                WhatsappMessageTemplate::columns_sql(Some("t"))
            ),
            params![id, workspace_id],
            WhatsappMessageTemplate::from_row,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

fn components_of(template: &WhatsappMessageTemplate) -> Vec<Value> {
    template
        .components
        .0
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// `resolveTemplateParams({ channel: "whatsapp", templateId, values, field })`.
pub async fn resolve_template_params(
    db: &Db,
    workspace_id: i64,
    template_id: i64,
    values: &TemplateParameterValues,
    field: &str,
) -> Result<Value> {
    let template = find_template_for_workspace(db, workspace_id, template_id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Template not found"))?;
    let applied = apply_wa_template_parameter_values(&components_of(&template), values);
    let problems: Vec<String> = [
        ("missing", &applied.missing),
        ("unknown", &applied.unknown),
        ("invalid", &applied.invalid),
        ("send templateData for", &applied.unsupported),
    ]
    .into_iter()
    .filter(|(_, keys)| !keys.is_empty())
    .map(|(label, keys)| format!("{label}: {}", keys.join(", ")))
    .collect();
    if !problems.is_empty() {
        return Err(BusinessError::validation(
            field,
            format!(
                "Template parameters do not match the template ({})",
                problems.join("; ")
            ),
        ));
    }
    let checked = wa_template_params_schema()
        .super_refine(|params, ctx| validate_wa_template_send_params(params, ctx, &[]))
        .safe_parse(&applied.params);
    if let Some(error) = checked.error {
        let messages: Vec<String> = error.issues.iter().map(|i| i.message.clone()).collect();
        return Err(BusinessError::validation(field, messages.join("; ")));
    }
    Ok(applied.params)
}

/// The most recent WhatsApp contact inbox of the contact (`lastMessageAt`
/// desc), as `findRecentWhatsappContactInbox`.
async fn find_recent_whatsapp_contact_inbox(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<ContactInbox>> {
    tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT {} FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
                   WHERE ci."contactId" = ?1 AND i."workspaceId" = ?2 AND ci."channel" = 'whatsapp'
                   ORDER BY coalesce(ci."lastMessageAt", 0) DESC, ci."id" ASC LIMIT 1"#,
                ContactInbox::columns_sql(Some("ci"))
            ),
            params![contact_id, workspace_id],
            ContactInbox::from_row,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// `ChatJobAction.sendWhatsappTemplateToConversation`.
pub const SEND_TEMPLATE_TO_CONVERSATION: &str = "sendWhatsappTemplateToConversation";

/// `sendWhatsappTemplateToConversation`: the chat job that sends the
/// template (the worker checks it is approved on the inbox's number).
/// `admin_id` = a signed-in member (`initiatedBy: "user"`), else `api`.
pub async fn send_template_to_conversation(
    db: &Db,
    conversation: &Conversation,
    template_id: &str,
    template_data: Option<Value>,
    inbox_id: Option<i64>,
    admin_id: Option<i64>,
) -> Result<OutgoingJob> {
    let contact_inbox = match inbox_id {
        Some(inbox) => {
            crate::message_actions::resolve_contact_inbox_for_conversation(
                db,
                conversation,
                Some(inbox),
            )
            .await?
        }
        None => find_recent_whatsapp_contact_inbox(
            db,
            conversation.workspace_id,
            conversation.contact_id,
        )
        .await?
        .ok_or_else(|| BusinessError::not_found("Inbox not found"))?,
    };
    if contact_inbox.channel != "whatsapp" {
        return Err(BusinessError::exception(
            "This conversation has no WhatsApp inbox",
            "systemError",
            400,
        ));
    }
    let mut data = json!({
        "conversation": conversation_json(conversation),
        "contactInbox": contact_inbox_json(&contact_inbox),
        "templateId": template_id,
        "initiatedBy": if admin_id.is_some() { "user" } else { "api" },
    });
    if let Some(template_data) = template_data {
        data["templateData"] = template_data;
    }
    if let Some(admin) = admin_id {
        data["adminId"] = json!(admin.to_string());
    }
    Ok(OutgoingJob {
        queue: QueueName::Chat,
        name: SEND_TEMPLATE_TO_CONVERSATION,
        data: json!({ "type": SEND_TEMPLATE_TO_CONVERSATION, "data": data }),
        attempts: None,
    })
}

// ---- call permission ------------------------------------------------------

/// `CallPermissionMessages` (`ENGLISH_CALL_PERMISSION_MESSAGES`).
pub struct CallPermissionMessages {
    pub not_whatsapp_conversation: &'static str,
    pub not_found: &'static str,
    pub inbox_not_found: &'static str,
    pub permission_check_failed: &'static str,
    pub permission_request_limit_reached: &'static str,
}

pub const ENGLISH_CALL_PERMISSION_MESSAGES: CallPermissionMessages = CallPermissionMessages {
    not_whatsapp_conversation: "This conversation is not on WhatsApp.",
    not_found: "WhatsApp channel not found",
    inbox_not_found: "The contact has no WhatsApp inbox with this `inboxId`.",
    permission_check_failed: "Could not check the customer's call permission with Meta. Try again.",
    permission_request_limit_reached: "Meta allows one call-permission request per 24 hours and two per 7 days for each customer: the limit is reached.",
};

fn system_error(message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

async fn find_whatsapp_contact_inbox(
    db: &Db,
    contact_id: i64,
    inbox_id: Option<i64>,
) -> Result<Option<ContactInbox>> {
    tx::read(db, move |conn| {
        conn.query_row(
            &format!(
                r#"SELECT {} FROM "ContactInbox" WHERE "contactId" = ?1 AND "channel" = 'whatsapp'
                   AND (?2 IS NULL OR "inboxId" = ?2) LIMIT 1"#,
                ContactInbox::columns_sql(None)
            ),
            params![contact_id, inbox_id],
            ContactInbox::from_row,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// The digits of a phone-like id (`digitsOf`).
fn digits_of(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    })
}

/// `GET /{phoneNumberId}/call_permissions` (`getCallPermissions` in
/// `rescue`): the response, or Meta's error message (`parseOriginError`).
async fn read_meta_call_permissions(
    base_url: &str,
    auth: &Value,
    contact_inbox: &ContactInbox,
) -> std::result::Result<Value, Option<String>> {
    let token = auth.pointer("/tokens/accessToken").and_then(Value::as_str);
    let phone = auth
        .pointer("/metadata/phoneNumber/id")
        .and_then(Value::as_str);
    let (Some(token), Some(phone)) = (token, phone) else {
        return Err(None);
    };
    let version = auth
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_API_VERSION);
    // `resolveDialIdentity(contactInbox).permissionTarget`.
    let target = if should_address_by_source_user_id(
        &contact_inbox.source_id,
        contact_inbox.source_user_id.as_deref(),
    ) {
        (
            "recipient",
            contact_inbox.source_user_id.clone().unwrap_or_default(),
        )
    } else {
        ("user_wa_id", digits_of(&contact_inbox.source_id))
    };
    let mut url = url::Url::parse(&format!("{base_url}/{version}/{phone}/call_permissions"))
        .map_err(|e| Some(e.to_string()))?;
    url.query_pairs_mut().append_pair(target.0, &target.1);
    let response = http_client()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| Some(e.to_string()))?;
    let ok = response.status().is_success();
    let body: Value = response
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if ok {
        Ok(body)
    } else {
        tracing::warn!(body = %body, "Whatsapp calling: could not read call permissions from Meta");
        Err(body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }
}

/// `canPerformCallAction(response, "send_call_permission_request")`.
fn can_send_call_permission_request(response: &Value) -> bool {
    response["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["action_name"] == "send_call_permission_request")
        .is_some_and(|a| {
            let flag = if a["can_perform_action"].is_null() {
                &a["can_perform"]
            } else {
                &a["can_perform_action"]
            };
            flag == &Value::Bool(true)
        })
}

/// `toPublicErrorMessage(error, fallback)` for a WhatsApp SDK error: the
/// message (with ` (code N)` when Meta's code is not in it) sanitized.
fn public_meta_message(message: Option<&str>, fallback: &str) -> String {
    let Some(text) = message.map(str::trim).filter(|t| !t.is_empty()) else {
        return fallback.to_owned();
    };
    if crate::errors::is_query_dump(text) {
        return fallback.to_owned();
    }
    let sanitized = sanitize_public_text(text);
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized
    }
}

/// `requestWhatsappCallPermission` (public route: `requireRequestedInbox`,
/// no user): checks Meta's counter, then stores the outgoing
/// `whatsapp_call_permission_request` message (the worker sends it).
///
/// ponytail: the 60 s `withCache` of Meta's answer is not kept; every call
/// re-reads Meta (the original drops the cache entry after a send anyway).
pub async fn request_call_permission(
    db: &Db,
    base_url: &str,
    conversation: &Conversation,
    text: String,
    inbox_id: Option<i64>,
    messages: &CallPermissionMessages,
) -> Result<crate::message::Outgoing> {
    let pinned = match inbox_id {
        Some(inbox) => {
            find_whatsapp_contact_inbox(db, conversation.contact_id, Some(inbox)).await?
        }
        None => None,
    };
    if inbox_id.is_some() && pinned.is_none() {
        return Err(BusinessError::not_found(messages.inbox_not_found));
    }
    let contact_inbox = match pinned {
        Some(ci) => ci,
        None => find_whatsapp_contact_inbox(db, conversation.contact_id, None)
            .await?
            .ok_or_else(|| system_error(messages.not_whatsapp_conversation))?,
    };
    let (workspace_id, inbox) = (conversation.workspace_id, contact_inbox.inbox_id);
    let integration: IntegrationWhatsapp = tx::read(db, move |conn| {
        Ok(
            chatbotx_db::repo::integration_whatsapp::find_by_inbox_id_for_workspace(
                conn,
                workspace_id,
                inbox,
            )?,
        )
    })
    .await?
    .ok_or_else(|| BusinessError::not_found(messages.not_found))?;
    let permissions = read_meta_call_permissions(base_url, &integration.auth.0, &contact_inbox)
        .await
        .map_err(|message| {
            system_error(public_meta_message(
                message.as_deref(),
                messages.permission_check_failed,
            ))
        })?;
    if !can_send_call_permission_request(&permissions) {
        return Err(system_error(messages.permission_request_limit_reached));
    }
    crate::message::create_outgoing(
        db,
        conversation,
        &contact_inbox,
        OutgoingInput {
            text: Some(text),
            content_attributes: Some(json!({ "type": "whatsapp_call_permission_request" })),
            ..OutgoingInput::default()
        },
        None,
    )
    .await
}

// ---- Flow response view ---------------------------------------------------

/// `getWhatsappFlowResponse(contentAttributes)`: the `nfm_reply` entity of
/// a Flow (not an `address_message`).
pub fn whatsapp_flow_response(attributes: Option<&Value>) -> Option<&Map<String, Value>> {
    let attrs = attributes?.as_object()?;
    let name_ok = attrs.get("name").is_none_or(|n| n == "flow");
    (attrs.get("type").and_then(Value::as_str) == Some("whatsapp_flow_response")
        && name_ok
        && attrs.get("flowResponse").is_some_and(Value::is_object))
    .then_some(attrs)
}

/// `humanizeKey`: `_` → space, `aB` → `a B`, first character upper-cased.
fn humanize_key(key: &str) -> String {
    let spaced = key.replace('_', " ");
    let chars: Vec<char> = spaced.chars().collect();
    let mut out = String::with_capacity(spaced.len() + 4);
    for (i, c) in chars.iter().enumerate() {
        out.push(*c);
        if c.is_ascii_lowercase() && chars.get(i + 1).is_some_and(char::is_ascii_uppercase) {
            out.push(' ');
        }
    }
    let mut it = out.chars();
    match it.next() {
        Some(first) => first.to_uppercase().chain(it).collect(),
        None => out,
    }
}

fn nullable_str(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .map_or(Value::Null, |s| json!(s))
}

/// `readLegacyFiles`: Meta's `{ file_name, mime_type, id }` entries.
fn read_legacy_files(items: &[Value]) -> Option<Vec<Value>> {
    items
        .iter()
        .map(|item| {
            let name = item.get("file_name")?.as_str()?;
            let media = item
                .get("id")
                .filter(|v| v.is_string())
                .or_else(|| item.get("media_id"));
            Some(json!({
                "status": "unprocessed",
                "fileName": name,
                "mimeType": nullable_str(item.get("mime_type")),
                "mediaId": nullable_str(media),
            }))
        })
        .collect()
}

/// `toFileView`: a kept file (no stored URL = `unprocessed`).
fn file_view(file: &Value, url: Option<&str>) -> Value {
    let (name, media) = (&file["fileName"], &file["mediaId"]);
    if file["status"] == "failed" {
        return json!({ "status": "failed", "fileName": name, "mimeType": file["mimeType"],
            "mediaId": media, "failureReason": file["reason"] });
    }
    match url {
        Some(url) => json!({ "status": "stored", "fileName": name, "mimeType": file["mimeType"],
            "mediaId": media, "size": file["size"], "url": url }),
        None => json!({ "status": "unprocessed", "fileName": name, "mimeType": file["mimeType"],
            "mediaId": media }),
    }
}

fn is_primitive(value: &Value) -> bool {
    value.is_string() || value.is_number()
}

/// `toField` with no Flow labels (`options` empty).
fn to_field(
    key: &str,
    value: &Value,
    files: Option<&Value>,
    url_of: &dyn Fn(&str) -> Option<String>,
) -> Value {
    let label = humanize_key(key);
    let field = |kind: &str, value: Value| json!({ "key": key, "label": label, "kind": kind, "value": value });
    match value {
        Value::Bool(b) => return field("boolean", json!(b)),
        Value::Null => return field("text", json!("")),
        v if is_primitive(v) => return field("text", json!(js::string_of(v))),
        Value::Array(items) => {
            let kept: Vec<Value> = files
                .and_then(|f| f.get(key))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let views = if kept.is_empty() {
                (!items.is_empty())
                    .then(|| read_legacy_files(items))
                    .flatten()
            } else {
                Some(
                    kept.iter()
                        .map(|f| {
                            let url = f["mediaId"].as_str().and_then(url_of);
                            file_view(f, url.as_deref())
                        })
                        .collect(),
                )
            };
            if let Some(views) = views {
                return field("files", Value::Array(views));
            }
            if items.iter().all(is_primitive) {
                return field(
                    "list",
                    Value::Array(items.iter().map(|i| json!(js::string_of(i))).collect()),
                );
            }
        }
        _ => {}
    }
    // `formatDateRange`: CalendarPicker range answers.
    if let (Some(start), Some(end)) = (
        value.get("start-date").and_then(Value::as_str),
        value.get("end-date").and_then(Value::as_str),
    ) {
        return field("text", json!(format!("{start} - {end}")));
    }
    field("json", json!(js::json_stringify(value)))
}

/// `jsonb` key order (shorter keys first, then bytewise): the order the
/// original reads a stored Flow response in.
fn jsonb_order<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<&'a String> {
    let mut keys: Vec<&String> = keys.collect();
    keys.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    keys
}

/// `getWhatsappFlowResponseView` + `buildWhatsappFlowResponseFields`.
///
/// ponytail: the Flow labels (`resolveSentWhatsappFlow` + the Flow JSON on
/// Meta, `getFlowResponseFields`) are not ported: every response reads with
/// humanized keys (`labelSource: fallback`), the original's answer whenever
/// the Flow cannot be identified.
pub fn flow_response_view(
    message: &chatbotx_db::repo::message::MessageWithAttachments,
) -> Result<Value> {
    let attrs = message.message.content_attributes.as_ref().map(|j| &j.0);
    let entity = whatsapp_flow_response(attrs)
        .ok_or_else(|| BusinessError::not_found("WhatsApp Flow response not found"))?;
    let response = entity["flowResponse"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    // ponytail: attachments carry no URL yet (`resolveMediaUrl`), so a kept
    // file reads as `unprocessed`.
    let url_of = |_: &str| None;
    let fields: Vec<Value> = jsonb_order(response.keys())
        .into_iter()
        .filter(|k| k.as_str() != "flow_token")
        .map(|k| to_field(k, &response[k], entity.get("files"), &url_of))
        .collect();
    Ok(json!({ "fields": fields, "labelSource": "fallback" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_keys() {
        assert_eq!(humanize_key("full_name"), "Full name");
        assert_eq!(humanize_key("newsletterOptIn"), "Newsletter Opt In");
        assert_eq!(humanize_key("age"), "Age");
    }

    #[test]
    fn reads_meta_permission_actions() {
        let ok = json!({ "actions": [{ "action_name": "send_call_permission_request", "can_perform_action": true }] });
        assert!(can_send_call_permission_request(&ok));
        let legacy = json!({ "actions": [{ "action_name": "send_call_permission_request", "can_perform": true }] });
        assert!(can_send_call_permission_request(&legacy));
        let no = json!({ "actions": [{ "action_name": "send_call_permission_request", "can_perform_action": false }] });
        assert!(!can_send_call_permission_request(&no));
        assert!(!can_send_call_permission_request(&json!({})));
    }

    #[test]
    fn flow_fields_follow_jsonb_order_and_kinds() {
        let response = json!({
            "age": 42, "topics": ["shoes", "bags"], "address": { "street": "Main" },
            "documents": [{ "file_name": "id.pdf", "mime_type": "application/pdf", "id": "media-1" }],
            "full_name": "Ada Lovelace", "flow_token": "t", "newsletterOptIn": true,
            "range": { "start-date": "2026-01-01", "end-date": "2026-01-02" },
        });
        let map = response.as_object().unwrap();
        let none = |_: &str| None;
        let fields: Vec<Value> = jsonb_order(map.keys())
            .into_iter()
            .filter(|k| k.as_str() != "flow_token")
            .map(|k| to_field(k, &map[k], None, &none))
            .collect();
        let keys: Vec<&str> = fields.iter().map(|f| f["key"].as_str().unwrap()).collect();
        assert_eq!(
            keys,
            [
                "age",
                "range",
                "topics",
                "address",
                "documents",
                "full_name",
                "newsletterOptIn"
            ]
        );
        assert_eq!(fields[0]["value"], "42");
        assert_eq!(fields[1]["value"], "2026-01-01 - 2026-01-02");
        assert_eq!(fields[2]["kind"], "list");
        assert_eq!(fields[3]["value"], "{\"street\":\"Main\"}");
        assert_eq!(
            fields[4]["value"],
            json!([{ "status": "unprocessed", "fileName": "id.pdf", "mimeType": "application/pdf", "mediaId": "media-1" }])
        );
        assert_eq!(fields[6]["kind"], "boolean");
    }

    #[test]
    fn bsuid_addressing() {
        assert!(should_address_by_source_user_id("US.1", Some("US.1")));
        assert!(should_address_by_source_user_id("", Some("US.1")));
        assert!(!should_address_by_source_user_id("849", Some("US.1")));
        assert!(!should_address_by_source_user_id("849", None));
    }
}
