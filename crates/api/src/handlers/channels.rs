//! `features/integration-api/api/public.ts`: the channel-token routes
//! (`channels.*`). `channels.me` (P3.6-D2), `channels.sendMessage`
//! (Q.I1a: the API channel inbound), `typing`, `markRead` and
//! `deliveryStatus` (Q.I2).

use chatbotx_business::BusinessError;
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Map, Value, json};

use super::input::{Input, Seg, Str, child, key};
use super::{ApiState, Reply, business_error, internal_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "channels.me",
    "channels.sendMessage",
    "channels.typing",
    "channels.markRead",
    "channels.deliveryStatus",
];

/// The authenticated channel (`context.inbox`, `context.workspace`).
#[derive(Debug, Clone, Copy)]
pub struct ChannelCall {
    pub inbox_id: i64,
    pub workspace_id: i64,
}

/// Runs one of [`ROUTES`]; `None` = not ported.
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: ChannelCall,
    body: &[u8],
) -> Option<Reply> {
    match route_id {
        "channels.me" => Some(me(state, call).await),
        "channels.sendMessage" => Some(send_message(state, call, body).await),
        "channels.typing" => Some(typing(state, call, body)),
        "channels.markRead" => Some(mark_read(state, call, body).await),
        "channels.deliveryStatus" => Some(delivery_status(state, call, body).await),
        _ => None,
    }
}

/// `channels.me`: the inbox and workspace of the token.
async fn me(state: &ApiState, call: ChannelCall) -> Reply {
    let inbox_id = call.inbox_id;
    let name = chatbotx_business::tx::read(&state.db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        conn.query_row(
            r#"SELECT "name" FROM "Inbox" WHERE "id" = ?1"#,
            [inbox_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
    .map_err(|e| business_error(e, &[]))?;
    Ok(json(
        200,
        json!({
            "inboxId": call.inbox_id.to_string(),
            "inboxName": name.unwrap_or_default(),
            "workspaceId": call.workspace_id.to_string(),
        }),
    ))
}

/// A required `z.object` at `path`.
fn object_at(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Map<String, Value>> {
    match value {
        Some(Value::Object(obj)) => Some(obj.clone()),
        None => {
            input.issue(
                path,
                "invalid_type",
                "Invalid input: expected object, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(path, "invalid_type", "Invalid input: expected object");
            None
        }
    }
}

/// `{ contact: { sourceId: z.string().min(1) } }`: the contact source id.
fn contact_source_id(input: &mut Input) -> Option<String> {
    let path = key("contact");
    let raw = input.get("contact").cloned();
    let contact = object_at(input, &path, raw.as_ref())?;
    input.string_at(
        &child(&path, "sourceId"),
        contact.get("sourceId"),
        Str {
            min: Some(1),
            ..Str::default()
        },
        true,
    )
}

/// `assertNotRateLimited(context.inbox.id, context.resHeaders)`: the 429
/// with `Retry-After`, `None` = allowed.
fn rate_limited(state: &ApiState, call: ChannelCall) -> Option<axum::response::Response> {
    let kv = state.kv.as_ref()?;
    let inbox = call.inbox_id.to_string();
    chatbotx_kv::rate_limit::assert_api_not_rate_limited(
        chatbotx_kv::rate_limit::ApiRateLimitInput {
            scope: "channel-api-rate-limit",
            key: &inbox,
            store: &kv.mem,
            now_ms: chatbotx_db::repo::now_ms(),
            limit: None,
        },
        None,
    )
    .err()
    .map(|limited| {
        let mut response = axum::response::IntoResponse::into_response(
            crate::auth::rate_limited_to_orpc(&limited),
        );
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from(limited.retry_after),
        );
        response
    })
}

/// `enqueueIntegrationJob` for the API channel.
async fn enqueue(
    state: &ApiState,
    name: &'static str,
    data: Value,
) -> Result<(), crate::orpc::OrpcError> {
    let jobs = state
        .jobs
        .as_ref()
        .ok_or_else(|| internal_error("job queue is off"))?;
    jobs.add(
        QueueName::Integration,
        name,
        json!({ "type": name, "data": data }),
        JobOptions::default(),
    )
    .await
    .map(|_| ())
    .map_err(|e| internal_error(&e.to_string()))
}

/// `channels.typing`: validated, rate limited and logged only (the original
/// has no downstream consumer).
fn typing(state: &ApiState, call: ChannelCall, body: &[u8]) -> Reply {
    let mut input = Input::from_body(&[], body);
    let source_id = contact_source_id(&mut input);
    let raw = input.get("typing").cloned();
    input.bool_at(&key("typing"), raw.as_ref(), true);
    input.finish()?;
    if let Some(response) = rate_limited(state, call) {
        return Ok(response);
    }
    tracing::debug!(
        inbox_id = call.inbox_id,
        source_id,
        "Received contact typing notification"
    );
    Ok(no_content())
}

/// `channels.markRead`: a `contactMarkAsRead` job.
async fn mark_read(state: &ApiState, call: ChannelCall, body: &[u8]) -> Reply {
    let mut input = Input::from_body(&[], body);
    let source_id = contact_source_id(&mut input);
    input.finish()?;
    if let Some(response) = rate_limited(state, call) {
        return Ok(response);
    }
    let payload: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    enqueue(
        state,
        "contactMarkAsRead",
        json!({
            "integrationType": "api",
            "integrationIdentifier": call.inbox_id.to_string(),
            "sourceConversationId": source_id,
            "payload": { "contact": { "sourceId": payload["contact"]["sourceId"] } },
        }),
    )
    .await?;
    Ok(no_content())
}

/// `channels.deliveryStatus`: a `messageStatus` job.
async fn delivery_status(state: &ApiState, call: ChannelCall, body: &[u8]) -> Reply {
    let mut input = Input::from_body(&[], body);
    let message_id = input.get("messageId").cloned();
    input.string_at(
        &key("messageId"),
        message_id.as_ref(),
        Str {
            min: Some(1),
            ..Str::default()
        },
        true,
    );
    let status = input.get("status").cloned();
    input.enum_at(
        &key("status"),
        status.as_ref(),
        &["delivered", "failed", "read"],
        true,
    );
    let timestamp = input.get("timestamp").cloned();
    input.string_at(&key("timestamp"), timestamp.as_ref(), Str::default(), true);
    input.finish()?;
    if let Some(response) = rate_limited(state, call) {
        return Ok(response);
    }
    // zod strips unknown keys; `error` is `z.unknown().optional()`.
    let sent: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let mut payload = Map::new();
    for field in ["messageId", "status", "timestamp", "error"] {
        if let Some(value) = sent.get(field) {
            payload.insert(field.into(), value.clone());
        }
    }
    enqueue(
        state,
        "messageStatus",
        json!({
            "integrationType": "api",
            "integrationIdentifier": call.inbox_id.to_string(),
            "payload": payload,
        }),
    )
    .await?;
    Ok(no_content())
}

/// `incomingApiMessageSchema` (`integrations/api` incoming-message.ts):
/// the zod checks, then the parsed body (defaults applied) to queue.
fn parse_incoming_api_message(body: &[u8]) -> Result<Value, crate::orpc::OrpcError> {
    let mut input = Input::from_body(&[], body);
    let any = Str::default();
    let source_id = Str {
        min: Some(1),
        ..Str::default()
    };
    let contact_path = key("contact");
    let raw = input.get("contact").cloned();
    let contact = object_at(&mut input, &contact_path, raw.as_ref());
    if let Some(contact) = &contact {
        input.string_at(
            &child(&contact_path, "sourceId"),
            contact.get("sourceId"),
            source_id,
            true,
        );
        for field in ["firstName", "lastName", "phoneNumber", "locale"] {
            input.string_at(&child(&contact_path, field), contact.get(field), any, false);
        }
        let email_path = child(&contact_path, "email");
        if let Some(email) = input.string_at(&email_path, contact.get("email"), any, false)
            && !super::contacts::is_email(&email)
        {
            input.issue(&email_path, "invalid_format", "Invalid email address");
        }
        let avatar_path = child(&contact_path, "avatar");
        if let Some(avatar) = input.string_at(&avatar_path, contact.get("avatar"), any, false)
            && !chatbotx_core::utils::zod::is_url(&avatar)
        {
            input.issue(&avatar_path, "invalid_format", "Invalid URL");
        }
    }
    let message_path = key("message");
    let raw = input.get("message").cloned();
    let message = object_at(&mut input, &message_path, raw.as_ref());
    let mut content_type = "text".to_owned();
    if let Some(message) = &message {
        input.string_at(
            &child(&message_path, "sourceId"),
            message.get("sourceId"),
            source_id,
            true,
        );
        if !message.get("text").is_none_or(Value::is_null) {
            input.string_at(
                &child(&message_path, "text"),
                message.get("text"),
                any,
                false,
            );
        }
        let attachments_path = child(&message_path, "attachments");
        match message.get("attachments") {
            None => {}
            Some(Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    let mut path = attachments_path.clone();
                    path.push(Seg::Index(index));
                    let Some(item) = object_at(&mut input, &path, Some(item)) else {
                        continue;
                    };
                    let url_path = child(&path, "url");
                    if let Some(url) = input.string_at(&url_path, item.get("url"), any, true)
                        && !chatbotx_core::utils::zod::is_url(&url)
                    {
                        input.issue(&url_path, "invalid_format", "Invalid URL");
                    }
                    input.enum_at(
                        &child(&path, "fileType"),
                        item.get("fileType"),
                        // `fileTypes` (no `gif` inbound).
                        &["image", "audio", "video", "file"],
                        true,
                    );
                    input.string_at(&child(&path, "mimeType"), item.get("mimeType"), any, true);
                    input.string_at(&child(&path, "name"), item.get("name"), any, false);
                }
            }
            Some(_) => input.issue(
                &attachments_path,
                "invalid_type",
                "Invalid input: expected array",
            ),
        }
        if let Some(kind) = input.enum_at(
            &child(&message_path, "contentType"),
            message.get("contentType"),
            &["text", "location"],
            false,
        ) {
            content_type = kind;
        }
        match message.get("contentAttributes") {
            None | Some(Value::Object(_)) => {}
            Some(_) => input.issue(
                &child(&message_path, "contentAttributes"),
                "invalid_type",
                "Invalid input: expected record",
            ),
        }
    }
    let raw = input.get("postbackPayload").cloned();
    if !raw.as_ref().is_none_or(Value::is_null) {
        input.string_at(&key("postbackPayload"), raw.as_ref(), any, false);
    }
    let mut out = Map::new();
    if let Some(contact) = contact {
        out.insert("contact".into(), Value::Object(contact));
    }
    if let Some(mut message) = message {
        message.insert("contentType".into(), json!(content_type));
        out.insert("message".into(), Value::Object(message));
    }
    if let Some(postback) = input.get("postbackPayload") {
        out.insert("postbackPayload".into(), postback.clone());
    }
    input.finish()?;
    Ok(Value::Object(out))
}

/// `channels.sendMessage`: queue the inbound message on the integration
/// queue (`incomingMessage`), `202 { accepted, messageSourceId }`.
async fn send_message(state: &ApiState, call: ChannelCall, body: &[u8]) -> Reply {
    // oRPC validates the input before the handler's rate limit.
    let payload = parse_incoming_api_message(body)?;
    if let Some(response) = rate_limited(state, call) {
        return Ok(response);
    }
    let source_id = payload["message"]["sourceId"].clone();
    enqueue(
        state,
        "incomingMessage",
        json!({
            "integrationType": "api",
            "integrationIdentifier": call.inbox_id.to_string(),
            "payload": payload,
        }),
    )
    .await?;
    Ok(json(
        202,
        json!({ "accepted": true, "messageSourceId": source_id }),
    ))
}
