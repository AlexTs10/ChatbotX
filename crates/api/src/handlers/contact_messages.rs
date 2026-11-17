//! `features/contacts/api/public/messages.ts` (`contactsMessagesPublicRouter`)
//! and the builder `listMessages` query. No member scope
//! (`assertMemberMayStartFlow`, `resolveSenderForCaller`): the token API
//! sends as `api`. Media library / device files answer 501 `notPorted`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chatbotx_business::conversation::{
    find_by_contact_with_inboxes, resolve_contact_inbox_for_send,
};
use chatbotx_business::message::{
    self, ListForConversation, Outgoing, OutgoingInput, find_for_contact, list_for_conversation,
    message_json,
};
use chatbotx_business::{BusinessError, automated_response};
use chatbotx_core::flow_config::steps::send_products_content_schema;
use chatbotx_core::flow_config::z;
use chatbotx_db::repo::message::PaginationCursor;
use chatbotx_db::repo::models::Conversation;
use chatbotx_jobs::JobOptions;
use chatbotx_rt::protocol::{ConversationRoute, route_for_conversation};
use serde_json::{Value, json};

use super::input::{Input, Num, Str};
use super::{ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, json, no_content};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.sendMessage",
    "contacts.listMessages",
    "contacts.getMessage",
    "contacts.triggerAutoReply",
    "contacts.sendFlow",
];

/// `possibleErrorsOnSendingMessage`.
const SENDING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("productMessageUnresolvable", 422),
    ("imagesNotSupportedOnChannel", 422),
];

fn bigint() -> z::Schema {
    z::string().regex("^\\d+$", "")
}

fn text() -> z::Schema {
    z::string().trim().min(1).max(1000)
}

fn files() -> z::Schema {
    // `z.instanceof(File)`: a JSON body never carries a File.
    z::array(z::custom(|_| false, Some("Input not instance of File"))).min(1)
}

fn media_file() -> z::Schema {
    z::object(vec![("path", z::string())])
}

/// `createMessageRequest` (with `identifier`).
fn create_message_request() -> z::Schema {
    z::intersection(
        z::union(vec![
            z::object(vec![("products", send_products_content_schema())]),
            z::object(vec![("text", text()), ("files", files())]),
            z::object(vec![("text", text()), ("mediaFile", media_file())]),
            z::object(vec![("text", text()), ("mediaFileId", bigint())]),
            z::object(vec![
                ("text", text()),
                ("mediaFileIds", z::array(bigint()).min(1).max(10)),
            ]),
            z::object(vec![("text", text())]),
            z::object(vec![("files", files())]),
            z::object(vec![("mediaFile", media_file())]),
            z::object(vec![("mediaFileId", bigint())]),
            z::object(vec![("mediaFileIds", z::array(bigint()).min(1).max(10))]),
            z::object(vec![("flowId", bigint()), ("nodeId", bigint().optional())]),
        ]),
        z::object(vec![
            ("inboxId", bigint().optional()),
            ("clientId", bigint().optional()),
            ("replyToMessageId", z::string().optional()),
            ("replyToMessageCreatedAt", z::date().coerce().optional()),
            ("isPrivateReply", z::boolean().optional()),
        ]),
    )
}

fn not_ported(what: &str) -> BusinessError {
    BusinessError::exception(format!("{what} are not ported yet"), "notPorted", 501)
}

fn parse_id(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_str).and_then(|s| s.parse().ok())
}

/// Adds the jobs and publishes the realtime `messageCreated` event.
pub(crate) async fn deliver(state: &ApiState, conversation: &Conversation, outgoing: Outgoing) {
    if let Some(payload) = outgoing.realtime {
        let route = route_for_conversation(
            Some(&ConversationRoute {
                assigned_inbox_team_id: conversation.assigned_inbox_team_id.map(|i| i.to_string()),
                assigned_user_id: conversation.assigned_user_id.map(|i| i.to_string()),
                inbox_id: None,
            }),
            None,
        );
        let mut event = chatbotx_rt::Envelope::new(chatbotx_rt::EventType::MessageCreated, payload);
        event.route = Some(route);
        state.queue_realtime(conversation.workspace_id, event).await;
    }
    queue_outgoing_jobs(state, outgoing.jobs).await;
}

/// Adds the jobs of a business call (`queue.add(name, data, opts)`).
pub(crate) async fn queue_outgoing_jobs(
    state: &ApiState,
    jobs_to_add: Vec<chatbotx_business::message::OutgoingJob>,
) {
    if let Some(jobs) = &state.jobs {
        for job in jobs_to_add {
            let opts = JobOptions {
                attempts: job.attempts,
                ..JobOptions::default()
            };
            if let Err(error) = jobs.add(job.queue, job.name, job.data, opts).await {
                tracing::error!(%error, job = job.name, "Failed to enqueue outgoing-message job");
            }
        }
    }
}

async fn send(
    state: &ApiState,
    call: &Call<'_>,
    inbox_id: Option<i64>,
    input: OutgoingInput,
    errors: ErrorMap,
) -> Reply {
    let contact_id = super::contacts::resolve(state, call, errors).await?;
    let (found, contact_inbox) =
        resolve_contact_inbox_for_send(&state.db, call.workspace_id, contact_id, inbox_id)
            .await
            .map_err(|e| business_error(e, errors))?;
    super::member_scope::assert_member_may_start_flow(
        &state.db,
        call.workspace_id,
        input.flow_id,
        errors,
    )
    .await?;
    let sender = super::member_scope::sender_for_caller();
    let outgoing =
        message::create_outgoing(&state.db, &found.conversation, &contact_inbox, input, sender)
            .await
            .map_err(|e| business_error(e, errors))?;
    deliver(state, &found.conversation, outgoing).await;
    Ok(no_content())
}

async fn send_message(state: &ApiState, call: Call<'_>) -> Reply {
    let input = Input::from_body(call.params, call.body);
    let outgoing = outgoing_input(input, SENDING)?;
    send(state, &call, outgoing.inbox_id, outgoing, SENDING).await
}

/// Validates a `createMessageRequest` body (the other `input` checks are
/// already queued) and maps it to the ported part of `createOutgoing`.
/// Media library / device files: 501 `notPorted`.
pub(crate) fn outgoing_input(
    input: Input,
    errors: ErrorMap,
) -> Result<OutgoingInput, crate::orpc::OrpcError> {
    let body = Value::Object(input.obj.clone());
    let parsed = create_message_request().safe_parse(&body);
    let mut input = input;
    if !parsed.success {
        let issues = parsed
            .issues()
            .iter()
            .filter_map(|issue| serde_json::to_value(issue).ok())
            .collect();
        input.extend_issues(issues);
    }
    input.finish()?;
    let data = parsed.data.unwrap_or(Value::Null);
    let has = |k: &str| data.get(k).is_some_and(|v| !v.is_null());
    if !has("products") && !has("flowId") && !has("text")
        || has("mediaFile")
        || has("mediaFileId")
        || has("mediaFileIds")
    {
        return Err(business_error(
            not_ported("Media library and device files"),
            errors,
        ));
    }
    Ok(OutgoingInput {
        flow_id: parse_id(data.get("flowId")),
        node_id: data
            .get("nodeId")
            .and_then(Value::as_str)
            .map(str::to_string),
        products: data.get("products").cloned(),
        text: data.get("text").and_then(Value::as_str).map(str::to_string),
        inbox_id: parse_id(data.get("inboxId")),
        client_id: data
            .get("clientId")
            .and_then(Value::as_str)
            .map(str::to_string),
        reply_to_message_id: data
            .get("replyToMessageId")
            .and_then(Value::as_str)
            .map(str::to_string),
        reply_to_message_created_at: body.get("replyToMessageCreatedAt").and_then(|v| match v {
            Value::String(s) => chatbotx_core::js::parse_date(s),
            Value::Number(n) => n.as_i64(),
            _ => None,
        }),
        is_private_reply: data
            .get("isPrivateReply")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ..OutgoingInput::default()
    })
}

/// `decodeCursor` (`null` for an invalid cursor: the first page).
pub fn decode_cursor(text: &str) -> Option<PaginationCursor> {
    let bytes = super::contact_inboxes::lenient_base64(text)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    if !matches!(object.get("direction")?.as_str()?, "next" | "prev") {
        return None;
    }
    let created_at = match object.get("createdAt")? {
        Value::String(text) => chatbotx_core::js::parse_date(text)?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    let id = match object.get("id")? {
        Value::String(t) if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => {
            t.parse().ok()?
        }
        _ => return None,
    };
    let shard_id = match object.get("shardId") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    Some(PaginationCursor {
        created_at,
        id: Some(id),
        shard_id,
    })
}

/// `encodeCursor({ direction: "prev", createdAt, id, shardId })`.
pub fn encode_cursor(cursor: &PaginationCursor) -> String {
    let mut text = format!(
        r#"{{"direction":"prev","createdAt":"{}","id":"{}""#,
        chatbotx_db::repo::iso(cursor.created_at),
        cursor.id.map(|i| i.to_string()).unwrap_or_default()
    );
    if let Some(shard) = &cursor.shard_id {
        text.push_str(&format!(r#","shardId":{}"#, json!(shard)));
    }
    text.push('}');
    STANDARD.encode(text)
}

async fn conversation_of(
    state: &ApiState,
    call: &Call<'_>,
) -> Result<Conversation, crate::orpc::OrpcError> {
    let contact_id = super::contacts::resolve(state, call, FINDING).await?;
    find_by_contact_with_inboxes(&state.db, call.workspace_id, contact_id)
        .await
        .map_err(|e| business_error(e, FINDING))?
        .map(|found| found.conversation)
        .ok_or_else(|| business_error(BusinessError::not_found("Conversation not found"), FINDING))
}

async fn list_messages(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let per_page = input.opt_coerce_int("perPage", Num::default());
    let cursor = input.opt_string("cursor", Str::default());
    let contact_inbox_id = match input.get("contactInboxId").cloned() {
        Some(_) => input.id("contactInboxId"),
        None => None,
    };
    input.finish()?;
    let conversation = conversation_of(state, &call).await?;
    let page = list_for_conversation(
        &state.db,
        ListForConversation {
            workspace_id: call.workspace_id,
            conversation_id: Some(conversation.id),
            contact_inbox_id,
            cursor: cursor.as_deref().and_then(decode_cursor),
            limit: per_page.unwrap_or(20),
        },
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    let data: Vec<Value> = page.data.iter().map(message_json).collect();
    Ok(json(
        200,
        json!({
            "data": data,
            "nextCursor": page.next_cursor.as_ref().map(encode_cursor),
            "prevCursor": Value::Null,
        }),
    ))
}

async fn get_message(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let message_id = input.id("messageId");
    input.finish()?;
    let conversation = conversation_of(state, &call).await?;
    let found = find_for_contact(
        &state.db,
        call.workspace_id,
        message_id.unwrap_or(-1),
        conversation.id,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, message_json(&found)))
}

async fn trigger_auto_reply(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let keyword = input.string(
        "keyword",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let inbox_id = match input.get("inboxId").cloned() {
        Some(_) => input.id("inboxId"),
        None => None,
    };
    input.finish()?;
    super::contacts::resolve(state, &call, MUTATING).await?;
    let rule = automated_response::find_by_inbound_keyword(
        &state.db,
        call.workspace_id,
        keyword.as_deref().unwrap_or_default(),
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?
    .ok_or_else(|| {
        business_error(
            BusinessError::not_found("No automated response found for this keyword"),
            MUTATING,
        )
    })?;
    let outgoing = match rule.flow_id {
        Some(flow_id) => OutgoingInput {
            flow_id: Some(flow_id),
            inbox_id,
            ..OutgoingInput::default()
        },
        None => OutgoingInput {
            text: Some(rule.text.unwrap_or_default()),
            inbox_id,
            ..OutgoingInput::default()
        },
    };
    send(state, &call, inbox_id, outgoing, MUTATING).await
}

async fn send_flow(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let flow_id = input.id("flowId");
    let inbox_id = match input.get("inboxId").cloned() {
        Some(_) => input.id("inboxId"),
        None => None,
    };
    input.finish()?;
    let outgoing = OutgoingInput {
        flow_id: Some(flow_id.unwrap_or(-1)),
        inbox_id,
        ..OutgoingInput::default()
    };
    send(state, &call, inbox_id, outgoing, MUTATING).await
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.sendMessage" => send_message(state, call).await,
        "contacts.listMessages" => list_messages(state, call).await,
        "contacts.getMessage" => get_message(state, call).await,
        "contacts.triggerAutoReply" => trigger_auto_reply(state, call).await,
        "contacts.sendFlow" => send_flow(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}
