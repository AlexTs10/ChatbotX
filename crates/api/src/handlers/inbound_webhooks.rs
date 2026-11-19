//! `features/inbound-webhooks/api/public.ts` (`inboundWebhooksPublicRouter`,
//! `inboundWebhookChannelsPublicRouter`; scope `integrations`).
//!
//! `previewMapping` runs `inbound_webhook_apply::preview_mapping` (no write).

use chatbotx_business::inbound_webhook as service;
use chatbotx_core::flow_config::z::{self, Schema};
use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::inbound_webhook_channel::{
    ChannelUserIdKind, ContactIdentityKind, InboxIdentifierKind, inbound_webhook_channel_spec,
    payload_resolvable_channels,
};
use chatbotx_core::utils::json_path::{format_json_path, is_jsonb_storable, parse_json_path};
use chatbotx_db::repo::inbound_webhook::{InboundWebhookListRow, InboundWebhookRequestListRow};
use chatbotx_db::repo::iso;
use chatbotx_db::repo::models::{InboundWebhook, InboundWebhookRequest};
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Principal, Reply, business_error, json,
    no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "inboundWebhookChannels.list",
    "inboundWebhooks.list",
    "inboundWebhooks.create",
    "inboundWebhooks.get",
    "inboundWebhooks.update",
    "inboundWebhooks.updateSettings",
    "inboundWebhooks.previewMapping",
    "inboundWebhooks.regenerateToken",
    "inboundWebhooks.listRequests",
    "inboundWebhooks.getRequest",
    "inboundWebhooks.deleteMany",
    "inboundWebhooks.delete",
];

/// `buildInboundWebhookUrl`: `<appUrl>/api/hooks/<token>`.
pub fn endpoint_url(app_url: &str, token: &str) -> String {
    url::Url::parse(app_url)
        .and_then(|base| base.join(&format!("/api/hooks/{token}")))
        .map_or_else(
            |_| format!("{}/api/hooks/{token}", app_url.trim_end_matches('/')),
            String::from,
        )
}

fn summary(row: &InboundWebhookListRow) -> Value {
    json!({
        "id": row.id.to_string(),
        "name": row.name,
        "active": row.active,
        "createdAt": iso(row.created_at),
        "updatedAt": iso(row.updated_at),
        "requestCount": row.request_count,
    })
}

/// `toInboundWebhookResource` (`endpointUrl` only when given).
pub(crate) fn resource(row: &InboundWebhook, endpoint: Option<String>) -> Value {
    let mut out = json!({
        "id": row.id.to_string(),
        "name": row.name,
        "active": row.active,
        "createdAt": iso(row.created_at),
        "updatedAt": iso(row.updated_at),
        "requestCount": row.request_count,
    });
    let obj = out.as_object_mut().expect("object literal");
    if let Some(url) = endpoint {
        obj.insert("endpointUrl".into(), Value::String(url));
    }
    let stored = |v: Option<&chatbotx_db::repo::Json>| v.map_or(Value::Null, |j| j.0.clone());
    obj.insert(
        "inboxResolution".into(),
        stored(row.inbox_resolution.as_ref()),
    );
    obj.insert("identifiers".into(), row.identifiers.0.clone());
    obj.insert("fieldMappings".into(), row.field_mappings.0.clone());
    obj.insert("actions".into(), row.actions.0.clone());
    obj.insert("sampleJson".into(), stored(row.sample_json.as_ref()));
    out
}

fn opt_id(v: Option<i64>) -> Value {
    v.map_or(Value::Null, |n| Value::String(n.to_string()))
}

fn list_row(r: &InboundWebhookRequestListRow) -> Value {
    json!({
        "id": r.id.to_string(),
        "status": r.status,
        "skipReason": r.skip_reason,
        "errorCode": r.error_code,
        "errorMessage": r.error_message,
        "inboxId": opt_id(r.inbox_id),
        "contactId": opt_id(r.contact_id),
        "attempts": r.attempts,
        "receivedAt": iso(r.created_at),
        "processedAt": r.processed_at.map(iso),
    })
}

/// `toInboundWebhookRequestDetail`.
fn request_detail(r: &InboundWebhookRequest, include_payload: bool) -> Value {
    let mut out = list_row(&InboundWebhookRequestListRow {
        id: r.id,
        status: r.status.clone(),
        skip_reason: r.skip_reason.clone(),
        error_code: r.error_code.clone(),
        error_message: r.error_message.clone(),
        inbox_id: r.inbox_id,
        contact_id: r.contact_id,
        attempts: r.attempts,
        created_at: r.created_at,
        processed_at: r.processed_at,
    });
    let obj = out.as_object_mut().expect("object literal");
    let stored = |v: Option<&chatbotx_db::repo::Json>| v.map_or(Value::Null, |j| j.0.clone());
    obj.insert("fieldErrors".into(), stored(r.field_errors.as_ref()));
    obj.insert("actionResults".into(), stored(r.action_results.as_ref()));
    if include_payload {
        obj.insert("payload".into(), r.payload.0.clone());
    }
    out
}

fn inbox_identifier_description(kind: InboxIdentifierKind) -> &'static str {
    use InboxIdentifierKind as I;
    match kind {
        I::PageId => {
            "The numeric Facebook page id (`Inbox.sourceId` of a Messenger inbox, see `inboxes.list`)."
        }
        I::IgId => "The numeric Instagram account id (`sourceId` of an Instagram inbox).",
        I::PhoneNumberId => {
            "The WhatsApp Business phone number id from Meta (`sourceId` of a WhatsApp inbox) — exact, preferred."
        }
        I::DisplayPhoneNumber => {
            "The WhatsApp number as people read it (e.g. +84 90 123 4567); only digits are compared, and two numbers sharing it make the request ambiguous."
        }
        I::BotId => {
            "The numeric Telegram bot id (`sourceId` of a Telegram inbox), not the @username."
        }
        I::OaId => "The Zalo Official Account id (`sourceId` of a Zalo inbox).",
        I::OpenId => "The connected TikTok account's open id, not the username or a customer's id.",
        I::ThreadsUserId => "The numeric Threads account id (`sourceId` of a Threads inbox).",
    }
}

fn channel_user_id_description(kind: ChannelUserIdKind) -> &'static str {
    use ChannelUserIdKind as U;
    match kind {
        U::Psid => "The Messenger page-scoped user id of the person.",
        U::Igsid => "The Instagram-scoped user id of the person.",
        U::TelegramChatId => "The Telegram chat id of the person.",
        U::ZaloUserId => "The Zalo user id of the person.",
        U::TiktokCustomerOpenId => {
            "The TikTok open id of the customer (not the connected account)."
        }
        U::ThreadsUsername => "The person's Threads username; matched lower-cased.",
        U::WhatsappId => {
            "The WhatsApp user id (`wa_id`): the phone number's digits, with or without a leading +."
        }
        U::ApiSourceId => "The caller-defined contact id used with the API channel.",
    }
}

/// `listInboundWebhookChannels`.
pub fn channels() -> Value {
    let resolvable = payload_resolvable_channels();
    Value::Array(
        ChannelType::ALL
            .iter()
            .map(|&channel| {
                let spec = inbound_webhook_channel_spec(channel);
                let identifiers: Vec<Value> = spec
                    .inbox_identifiers
                    .iter()
                    .map(|i| {
                        json!({
                            "kind": i.kind.as_str(),
                            "strategy": i.strategy.as_str(),
                            "examplePath": i.example_path,
                            "description": inbox_identifier_description(i.kind),
                        })
                    })
                    .collect();
                let user = spec.channel_user_id.map_or(Value::Null, |u| {
                    json!({
                        "kind": u.kind.as_str(),
                        "examplePath": u.example_path,
                        "description": channel_user_id_description(u.kind),
                    })
                });
                let creation: Vec<&str> = spec
                    .creation_identities
                    .iter()
                    .map(|k| k.as_str())
                    .collect();
                json!({
                    "channel": channel.as_str(),
                    "payloadResolvable": resolvable.contains(&channel),
                    "inboxIdentifiers": identifiers,
                    "channelUserId": user,
                    "creationIdentities": creation,
                })
            })
            .collect(),
    )
}

const MAX_JSON_PATH_LENGTH: usize = 500;
const MAX_SAMPLE_JSON_BYTES: usize = 64 * 1024;

/// `inboundWebhookJsonPathSchema`: stored in the canonical `$` form.
fn json_path() -> Schema {
    z::string()
        .trim()
        .min(1)
        .max(MAX_JSON_PATH_LENGTH)
        .transform(
            |value, ctx| match parse_json_path(value.as_str().unwrap_or_default()) {
                Some(segments) if !segments.is_empty() => {
                    Value::String(format_json_path(&segments))
                }
                _ => {
                    ctx.message("Unsupported JSON path");
                    Value::Null
                }
            },
        )
        .pipe(z::string().max(MAX_JSON_PATH_LENGTH))
}

/// `inboundWebhookConfigRequest`.
fn config_schema() -> Schema {
    let channels: Vec<&str> = ChannelType::ALL.iter().map(|c| c.as_str()).collect();
    let inbox_kinds: Vec<&str> = InboxIdentifierKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    let identity_kinds: Vec<&str> = ContactIdentityKind::ALL
        .iter()
        .map(|k| k.as_str())
        .collect();
    let bigint = || z::string().regex(r"^\d+$", "");
    let inbox_resolution = z::discriminated_union(
        "mode",
        vec![
            z::object(vec![
                ("mode", z::literal(json!("inbox"))),
                ("inboxId", bigint()),
            ]),
            z::object(vec![
                ("mode", z::literal(json!("payload"))),
                ("channel", z::enum_(&channels)),
                ("identifierKind", z::enum_(&inbox_kinds)),
                ("jsonPath", json_path()),
            ]),
        ],
    );
    let identifiers = z::array(z::object(vec![
        ("kind", z::enum_(&identity_kinds)),
        ("jsonPath", json_path()),
    ]))
    .max(4)
    .refine(
        |v| {
            let kinds: Vec<&Value> = v
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| &i["kind"])
                .collect();
            kinds
                .iter()
                .enumerate()
                .all(|(i, k)| !kinds[..i].contains(k))
        },
        Some("Each identity kind can be configured once"),
        None,
    );
    let field_mappings = z::array(z::object(vec![
        ("jsonPath", json_path()),
        ("target", z::string().trim().min(1).max(255)),
    ]))
    .max(50);
    let all = super::triggers::all_actions();
    let pick = |name: &str| {
        all.iter()
            .find(|schema| schema.field("type").literal_values() == [json!(name)])
            .cloned()
            .unwrap_or_else(z::never)
    };
    let actions = z::array(z::union(
        [
            "startAnotherFlow",
            "addTag",
            "removeTag",
            "setCustomField",
            "clearCustomField",
            "transferConversationToHuman",
        ]
        .into_iter()
        .map(pick)
        .collect(),
    ))
    .max(10);
    let sample_json = z::unknown()
        .refine(
            is_jsonb_storable,
            Some("The sample JSON contains unsupported characters or is nested too deeply"),
            None,
        )
        .abort()
        .refine(
            |v| serde_json::to_vec(v).is_ok_and(|b| b.len() <= MAX_SAMPLE_JSON_BYTES),
            Some("The sample JSON is too large"),
            None,
        )
        .nonoptional();
    z::object(vec![
        ("inboxResolution", inbox_resolution.nullable()),
        ("identifiers", identifiers),
        ("fieldMappings", field_mappings),
        ("actions", actions),
        ("sampleJson", sample_json),
    ])
}

/// The `previewMapping` body (`inboundWebhookPreviewPayloadSchema` + the
/// config parts).
fn preview_schema() -> Schema {
    let config = config_schema();
    let payload = z::unknown()
        .refine(
            is_jsonb_storable,
            Some("The payload is nested too deeply or contains unsupported text"),
            None,
        )
        .abort()
        .refine(
            |v| serde_json::to_vec(v).is_ok_and(|b| b.len() <= service::MAX_PAYLOAD_BYTES),
            Some("The payload is too large"),
            None,
        )
        .nonoptional();
    z::object(vec![
        ("payload", payload),
        ("inboxResolution", config.field("inboxResolution")),
        ("identifiers", config.field("identifiers")),
        ("fieldMappings", config.field("fieldMappings")),
    ])
}

/// Parses the config body into `input`'s issues.
fn parse_config(input: &mut Input) -> Option<service::Config> {
    let parsed = config_schema().safe_parse(&Value::Object(input.obj.clone()));
    if !parsed.success {
        let issues = parsed
            .issues()
            .iter()
            .filter_map(|issue| serde_json::to_value(issue).ok())
            .collect();
        input.extend_issues(issues);
        return None;
    }
    let data = parsed.data.unwrap_or_default();
    let take = |k: &str| data.get(k).cloned().unwrap_or(Value::Null);
    let opt = |k: &str| Some(take(k)).filter(|v| !v.is_null());
    Some(service::Config {
        inbox_resolution: opt("inboxResolution"),
        identifiers: take("identifiers"),
        field_mappings: take("fieldMappings"),
        actions: take("actions"),
        sample_json: opt("sampleJson"),
    })
}

fn id_param(input: &mut Input, name: &str) -> i64 {
    let value = input.get(name).cloned();
    digits_at(input, &key(name), value.as_ref(), true).unwrap_or_default()
}

fn name_spec() -> Str {
    Str::trimmed(1, service::MAX_NAME_LENGTH)
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let url = |token: &str| endpoint_url(&state.app_url, token);
    match route_id {
        "inboundWebhookChannels.list" => Ok(json(200, channels())),
        "inboundWebhooks.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let (rows, page_count) = service::list(db, ws, page, per_page)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows.iter().map(summary).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "inboundWebhooks.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", name_spec());
            input.finish()?;
            let row = service::create(db, ws, name.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, CREATING))?;
            state
                .record_audit(
                    "create",
                    &format!("created a new inbound webhook (#{})", row.id),
                )
                .await;
            Ok(json(201, resource(&row, Some(url(&row.token)))))
        }
        "inboundWebhooks.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id");
            input.finish()?;
            let row = service::find_or_fail(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            // The URL is a credential: a read-only token never receives it.
            let endpoint = (!principal.read_only).then(|| url(&row.token));
            Ok(json(200, resource(&row, endpoint)))
        }
        "inboundWebhooks.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let config = parse_config(&mut input);
            let id = id_param(&mut input, "id");
            input.finish()?;
            // `finish` passed, so the config parsed.
            let config = config.ok_or_else(|| super::internal_error("inbound webhook config"))?;
            let row = service::update(db, ws, id, config)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            state
                .record_audit("update", &format!("updated inbound webhook (#{id})"))
                .await;
            Ok(json(200, resource(&row, Some(url(&row.token)))))
        }
        "inboundWebhooks.updateSettings" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.opt_string("name", name_spec());
            let active = input.opt_bool("active");
            let id = id_param(&mut input, "id");
            input.finish()?;
            let changed = name.is_some() || active.is_some();
            let row = service::update_settings(db, ws, id, name, active)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if changed {
                state
                    .record_audit(
                        "update",
                        &format!("updated inbound webhook (#{id}) settings"),
                    )
                    .await;
            }
            Ok(json(200, resource(&row, Some(url(&row.token)))))
        }
        "inboundWebhooks.previewMapping" => {
            let mut input = Input::from_body(call.params, call.body);
            let parsed = preview_schema().safe_parse(&Value::Object(input.obj.clone()));
            if !parsed.success {
                let issues = parsed
                    .issues()
                    .iter()
                    .filter_map(|issue| serde_json::to_value(issue).ok())
                    .collect();
                input.extend_issues(issues);
            }
            input.finish()?;
            let data = parsed.data.unwrap_or_default();
            let take = |k: &str| data.get(k).cloned().unwrap_or(Value::Null);
            let resolution = Some(take("inboxResolution")).filter(|v| !v.is_null());
            let out = chatbotx_business::inbound_webhook_apply::preview_mapping(
                db,
                ws,
                take("payload"),
                resolution,
                take("identifiers"),
                take("fieldMappings"),
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "inboundWebhooks.regenerateToken" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input, "id");
            input.finish()?;
            let row = service::regenerate_token(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            state
                .record_audit(
                    "update",
                    &format!("regenerated the token of inbound webhook (#{id})"),
                )
                .await;
            Ok(json(200, json!({ "endpointUrl": url(&row.token) })))
        }
        "inboundWebhooks.listRequests" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let id = id_param(&mut input, "id");
            let value = input.get("status").cloned();
            let status = input.enum_at(
                &key("status"),
                value.as_ref(),
                service::REQUEST_STATUSES,
                false,
            );
            input.finish()?;
            service::find_or_fail(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let (rows, page_count) = service::list_requests(db, ws, id, status, page, per_page)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = rows.iter().map(list_row).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "inboundWebhooks.getRequest" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id");
            let request_id = id_param(&mut input, "requestId");
            let value = input.get("includePayload").cloned();
            let include = input.enum_at(
                &key("includePayload"),
                value.as_ref(),
                &["true", "false"],
                false,
            );
            input.finish()?;
            let row = service::find_request_or_fail(db, ws, id, request_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(
                200,
                request_detail(&row, include.as_deref() == Some("true")),
            ))
        }
        "inboundWebhooks.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = array(&mut input, "ids", 0, usize::MAX, |i, path, v| {
                digits_at(i, path, Some(v), true)
            });
            input.finish()?;
            let deleted = service::delete_many(db, ws, ids.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if !deleted.is_empty() {
                let list: Vec<String> = deleted.iter().map(|id| format!("#{id}")).collect();
                state
                    .record_audit(
                        "delete",
                        &format!("deleted inbound webhooks ({})", list.join(", ")),
                    )
                    .await;
            }
            Ok(no_content())
        }
        "inboundWebhooks.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id");
            input.finish()?;
            service::delete(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            state
                .record_audit("delete", &format!("deleted inbound webhook (#{id})"))
                .await;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
