//! `features/integration-api/api/public.ts`: the channel-token routes
//! (`channels.*`). `channels.me` (P3.6-D2) and `channels.sendMessage`
//! (Q.I1a: the API channel inbound), `typing` / `markRead` /
//! `deliveryStatus` (Q.D2c: queued for the worker's API `Channel`).

use axum::response::IntoResponse as _;
use chatbotx_business::BusinessError;
use chatbotx_jobs::queues::integration::{
    IntegrationJob, IntegrationJobAction, enqueue_integration_job,
};
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Map, Value, json};

use super::input::{Input, Seg, Str, child, key};
use super::{ApiState, Reply, business_error, internal_error, json};

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
        "channels.typing" | "channels.markRead" | "channels.deliveryStatus" => {
            Some(signal(state, route_id, call, body).await)
        }
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

/// `incomingApiMessageSchema` (`integrations/api` incoming-message.ts):
/// the zod checks, then the parsed body (defaults applied) to queue.
fn parse_incoming_api_message(body: &[u8]) -> Result<Value, crate::orpc::OrpcError> {
    let mut input = Input::from_body(&[], body);
    let any = Str::default();
    let source_id = Str {
        min: Some(1),
        ..Str::default()
    };
    let object_at = |input: &mut Input, path: &[Seg], value: Option<&Value>| match value {
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
    if let Some(limited) = rate_limited(state, call) {
        return Ok(limited);
    }
    let payload = parse_incoming_api_message(body)?;
    let source_id = payload["message"]["sourceId"].clone();
    let jobs = state
        .jobs
        .as_ref()
        .ok_or_else(|| internal_error("job queue is off"))?;
    jobs.add(
        QueueName::Integration,
        "incomingMessage",
        json!({ "type": "incomingMessage", "data": {
            "integrationType": "api",
            "integrationIdentifier": call.inbox_id.to_string(),
            "payload": payload,
        }}),
        JobOptions::default(),
    )
    .await
    .map_err(|e| internal_error(&e.to_string()))?;
    Ok(json(
        202,
        json!({ "accepted": true, "messageSourceId": source_id }),
    ))
}

/// `{ contact: { sourceId: z.string().min(1) } }`.
fn contact_source_id(input: &mut Input) -> Option<String> {
    let path = key("contact");
    match input.get("contact").cloned() {
        Some(Value::Object(contact)) => {
            let value = contact.get("sourceId");
            let spec = Str {
                min: Some(1),
                ..Str::default()
            };
            input.string_at(&child(&path, "sourceId"), value, spec, true)
        }
        None => {
            input.issue(
                &path,
                "invalid_type",
                "Invalid input: expected object, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(&path, "invalid_type", "Invalid input: expected object");
            None
        }
    }
}

/// `channels.typing` (accepted and logged only), `channels.markRead`
/// (`contactMarkAsRead` job) and `channels.deliveryStatus` (`messageStatus`
/// job): `204` with no body.
async fn signal(state: &ApiState, route_id: &str, call: ChannelCall, body: &[u8]) -> Reply {
    let mut input = Input::from_body(&[], body);
    let job = match route_id {
        "channels.typing" => {
            let source_id = contact_source_id(&mut input);
            let value = input.get("typing").cloned();
            input.bool_at(&key("typing"), value.as_ref(), true);
            input.finish()?;
            if let Some(limited) = rate_limited(state, call) {
                return Ok(limited);
            }
            tracing::debug!(inbox_id = call.inbox_id, source_id = ?source_id, "Received contact typing notification");
            None
        }
        "channels.markRead" => {
            let source_id = contact_source_id(&mut input);
            input.finish()?;
            if let Some(limited) = rate_limited(state, call) {
                return Ok(limited);
            }
            let source_id = source_id.unwrap_or_default();
            Some(IntegrationJob {
                action: IntegrationJobAction::ContactMarkAsRead,
                data: json!({
                    "integrationType": "api",
                    "integrationIdentifier": call.inbox_id.to_string(),
                    "sourceConversationId": source_id,
                    "payload": { "contact": { "sourceId": source_id } },
                }),
            })
        }
        _ => {
            let message_id = input.string(
                "messageId",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            let value = input.get("status").cloned();
            let status = input.enum_at(
                &key("status"),
                value.as_ref(),
                &["delivered", "failed", "read"],
                true,
            );
            let timestamp = input.string("timestamp", Str::default());
            let error = input.get("error").cloned();
            input.finish()?;
            if let Some(limited) = rate_limited(state, call) {
                return Ok(limited);
            }
            let mut payload = Map::new();
            payload.insert("messageId".into(), json!(message_id));
            payload.insert("status".into(), json!(status));
            payload.insert("timestamp".into(), json!(timestamp));
            if let Some(error) = error {
                payload.insert("error".into(), error);
            }
            Some(IntegrationJob {
                action: IntegrationJobAction::MessageStatus,
                data: json!({
                    "integrationType": "api",
                    "integrationIdentifier": call.inbox_id.to_string(),
                    "payload": payload,
                }),
            })
        }
    };
    if let Some(job) = job {
        let jobs = state
            .jobs
            .as_ref()
            .ok_or_else(|| internal_error("job queue is off"))?;
        enqueue_integration_job(jobs.as_ref(), job, None)
            .await
            .map_err(|e| internal_error(&e.to_string()))?;
    }
    Ok(super::no_content())
}

/// `assertNotRateLimited` (120 requests per inbox per 10 s): the 429 with
/// `Retry-After`, `None` = allowed.
fn rate_limited(state: &ApiState, call: ChannelCall) -> Option<axum::response::Response> {
    let kv = state.kv.as_ref()?;
    let inbox = call.inbox_id.to_string();
    let limited = chatbotx_kv::rate_limit::assert_api_not_rate_limited(
        chatbotx_kv::rate_limit::ApiRateLimitInput {
            scope: "channel-api-rate-limit",
            key: &inbox,
            store: &kv.mem,
            now_ms: chatbotx_db::repo::now_ms(),
            limit: None,
        },
        None,
    )
    .err()?;
    let mut response = crate::auth::rate_limited_to_orpc(&limited).into_response();
    response.headers_mut().insert(
        axum::http::header::RETRY_AFTER,
        axum::http::HeaderValue::from(limited.retry_after),
    );
    Some(response)
}
