//! `apps/builder/src/features/messages/api/public.ts` (`messagesPublicRouter`,
//! `/v1/conversations/{conversationId}/messages`, scope `inbox`). Workspace
//! tokens only (no member scope): a send is unattributed (`api`). Not ported
//! (501 `notPorted`): device files, a new attachment on edit. The WhatsApp routes (Flow response view, template send, call
//! permission) call `chatbotx_business::whatsapp`.

use chatbotx_business::BusinessError;
use chatbotx_business::conversation_list as list;
use chatbotx_business::message::{self, ListForConversation, message_json};
use chatbotx_business::message_actions as actions;
use chatbotx_business::whatsapp;
use chatbotx_core::flow_config::steps::{TemplateParameterValues, wa_template_params_schema};
use chatbotx_db::repo::models::Conversation;
use serde_json::{Value, json};

use super::contact_messages::{
    create_outgoing, decode_cursor, deliver, encode_cursor, outgoing_input, queue_outgoing_jobs,
};
use super::conversations::queue_jobs;
use super::input::{Input, Num, Seg, Str, key};
use super::{ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, json, no_content};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "messages.list",
    "messages.get",
    "messages.getFlowResponse",
    "messages.create",
    "messages.update",
    "messages.delete",
    "messages.changeAttributes",
    "messages.sendWhatsappTemplate",
    "messages.requestWhatsappCallPermission",
];

/// `possibleErrorsOnCreatingResource` + the product / image errors.
const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("productMessageUnresolvable", 422),
    ("imagesNotSupportedOnChannel", 422),
];

fn not_ported(what: &str, errors: ErrorMap) -> crate::orpc::OrpcError {
    business_error(
        BusinessError::exception(format!("{what} not ported yet"), "notPorted", 501),
        errors,
    )
}

/// A required `z.coerce.date()` key (`new Date(value)` must be valid).
fn created_at(input: &mut Input) -> Option<i64> {
    let parsed = match input.get("createdAt") {
        Some(Value::String(text)) => chatbotx_core::js::parse_date(text),
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64),
        Some(Value::Null) => Some(0),
        Some(Value::Bool(b)) => Some(i64::from(*b)),
        _ => None,
    };
    if parsed.is_none() {
        input.issue(
            &key("createdAt"),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

/// `assertConversationAccessForCaller`: the conversation of the workspace.
async fn conversation(
    state: &ApiState,
    call: &Call<'_>,
    id: Option<i64>,
    errors: ErrorMap,
) -> Result<Conversation, crate::orpc::OrpcError> {
    let found = list::find_or_fail(&state.db, call.workspace_id, id.unwrap_or(-1))
        .await
        .map_err(|e| business_error(e, errors))?;
    // `assertConversationAccessForCaller` (mobile members).
    super::member_scope::assert_conversation_allowed(
        found.assigned_user_id,
        found.assigned_inbox_team_id,
        errors,
    )?;
    Ok(found)
}

/// The `conversationId` / `messageId` / `createdAt` of a message route.
struct Target {
    conversation: Option<i64>,
    message: i64,
    created_at: i64,
}

fn target(input: &mut Input) -> Target {
    Target {
        conversation: input.digits_id("conversationId"),
        message: input.digits_id("messageId").unwrap_or(-1),
        created_at: created_at(input).unwrap_or(0),
    }
}

async fn list_messages(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let conversation_id = input.digits_id("conversationId");
    let per_page = input.opt_coerce_int("perPage", Num::range(1.0, 50.0));
    let cursor = input.opt_string("cursor", Str::default());
    input.finish()?;
    let found = conversation(state, &call, conversation_id, FINDING).await?;
    let page = message::list_for_conversation(
        &state.db,
        ListForConversation {
            workspace_id: call.workspace_id,
            conversation_id: Some(found.id),
            contact_inbox_id: None,
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

/// `get` / `getFlowResponse`: the message of this conversation.
async fn find_message(
    state: &ApiState,
    call: &Call<'_>,
) -> Result<chatbotx_db::repo::message::MessageWithAttachments, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, call.query);
    let t = target(&mut input);
    input.finish()?;
    let found = conversation(state, call, t.conversation, FINDING).await?;
    let message =
        actions::find_by_id_with_urls(&state.db, call.workspace_id, t.message, t.created_at)
            .await
            .map_err(|e| business_error(e, FINDING))?;
    if message.message.conversation_id != found.id {
        return Err(business_error(
            BusinessError::not_found("Message not found"),
            FINDING,
        ));
    }
    Ok(message)
}

async fn create(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let conversation_id = input.digits_id("conversationId");
    let outgoing = outgoing_input(input, CREATING)?;
    let found = conversation(state, &call, conversation_id, CREATING).await?;
    let contact_inbox =
        actions::resolve_contact_inbox_for_conversation(&state.db, &found, outgoing.inbox_id)
            .await
            .map_err(|e| business_error(e, CREATING))?;
    super::member_scope::assert_member_may_start_flow(
        &state.db,
        call.workspace_id,
        outgoing.flow_id,
        CREATING,
    )
    .await?;
    // `resolveSenderForCaller`: a member's message is attributed to them.
    let sender = super::member_scope::sender_for_caller();
    let (sent, body) = create_outgoing(state, &found, &contact_inbox, outgoing, sender)
        .await
        .map_err(|e| business_error(e, CREATING))?;
    deliver(state, &found, sent).await;
    Ok(json(201, body))
}

async fn update(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let t = target(&mut input);
    let new_text = input.string("newText", Str::trimmed(1, 2000));
    let new_attachment = input.opt_string("newAttachmentPath", Str::default());
    for name in [
        "newAttachmentPublicUrl",
        "newAttachmentMimeType",
        "newAttachmentName",
    ] {
        input.opt_string(name, Str::default());
    }
    input.opt_number("newAttachmentSize", Num::default());
    let remove_attachment = input.opt_bool("removeAttachment").unwrap_or(false);
    input.finish()?;
    let found = conversation(state, &call, t.conversation, MUTATING).await?;
    if new_attachment.is_some() {
        return Err(not_ported("A new attachment on edit is", MUTATING));
    }
    let (body, done) = actions::edit_message(
        &state.db,
        &found,
        t.message,
        t.created_at,
        new_text.unwrap_or_default(),
        remove_attachment,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    if let Some(event) = done.event {
        state.queue_realtime(call.workspace_id, event).await;
    }
    queue_jobs(state, done.job).await;
    Ok(json(200, body))
}

async fn delete(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let t = target(&mut input);
    input.finish()?;
    let found = conversation(state, &call, t.conversation, MUTATING).await?;
    let done = actions::delete_message(&state.db, &found, t.message, t.created_at)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    if let Some(event) = done.event {
        state.queue_realtime(call.workspace_id, event).await;
    }
    queue_jobs(state, done.job).await;
    Ok(no_content())
}

async fn change_attributes(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let t = target(&mut input);
    let liked = input.opt_bool("liked");
    let hidden = input.opt_bool("hidden");
    input.finish()?;
    let found = conversation(state, &call, t.conversation, MUTATING).await?;
    let job = actions::change_attributes(&state.db, &found, t.message, t.created_at, liked, hidden)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    queue_jobs(state, Some(job)).await;
    Ok(json(
        200,
        json!({ "success": true, "messageId": t.message.to_string() }),
    ))
}

/// An optional `zodBigintAsString` key.
fn opt_digits_id(input: &mut Input, name: &str) -> Option<i64> {
    if input.has(name) {
        input.digits_id(name)
    } else {
        None
    }
}

/// `messages.requestWhatsappCallPermission` (202 `{ queued: true }`).
async fn request_call_permission(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let conversation_id = input.digits_id("conversationId");
    let text = input.string("text", Str::trimmed(1, 1024));
    let inbox_id = opt_digits_id(&mut input, "inboxId");
    input.finish()?;
    let found = conversation(state, &call, conversation_id, MUTATING).await?;
    let sent = whatsapp::request_call_permission(
        &state.db,
        whatsapp::GRAPH_API_URL,
        &found,
        text.unwrap_or_default(),
        inbox_id,
        &whatsapp::ENGLISH_CALL_PERMISSION_MESSAGES,
        None,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    deliver(state, &found, sent).await;
    Ok(json(202, json!({ "queued": true })))
}

/// `templateParams`: `z.record(z.string(), z.string()).optional()`.
fn template_params(input: &mut Input) -> Option<TemplateParameterValues> {
    let value = input.get("templateParams")?.clone();
    let Value::Object(map) = value else {
        input.issue(
            &key("templateParams"),
            "invalid_type",
            "Invalid input: expected record, received ",
        );
        return None;
    };
    let mut out = TemplateParameterValues::new();
    for (name, value) in map {
        match value {
            Value::String(text) => {
                out.insert(name, text);
            }
            _ => input.issue(
                &[Seg::Key("templateParams".into()), Seg::Key(name)],
                "invalid_type",
                "Invalid input: expected string",
            ),
        }
    }
    Some(out)
}

/// `templateData`: `waTemplateParamsSchema.optional()`.
fn template_data(input: &mut Input) -> Option<Value> {
    let value = input.get("templateData")?.clone();
    let parsed = wa_template_params_schema().safe_parse(&value);
    if let Some(error) = parsed.error {
        let issues = error
            .issues
            .iter()
            .filter_map(|issue| serde_json::to_value(issue).ok())
            .map(|mut issue| {
                if let Some(path) = issue["path"].as_array_mut() {
                    path.insert(0, json!("templateData"));
                }
                issue
            })
            .collect();
        input.extend_issues(issues);
        return None;
    }
    parsed.data
}

/// `messages.sendWhatsappTemplate` (202 `{ queued: true }`).
async fn send_template(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let template_id = input.string("templateId", Str::trimmed(1, usize::MAX));
    let params = template_params(&mut input);
    let data = template_data(&mut input);
    let inbox_id = opt_digits_id(&mut input, "inboxId");
    // The object refinement runs only when the fields passed.
    if !input.failed() && params.is_some() && data.is_some() {
        input.issue(
            &key("templateParams"),
            "custom",
            "Send templateParams or templateData, not both",
        );
    }
    let conversation_id = input.digits_id("conversationId");
    input.finish()?;
    let found = conversation(state, &call, conversation_id, MUTATING).await?;
    let template_id = template_id.unwrap_or_default();
    let template_data = match params {
        Some(values) => Some(
            whatsapp::resolve_template_params(
                &state.db,
                call.workspace_id,
                template_id.parse().unwrap_or(-1),
                &values,
                "templateParams",
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?,
        ),
        None => data,
    };
    let job = whatsapp::send_template_to_conversation(
        &state.db,
        &found,
        &template_id,
        template_data,
        inbox_id,
        None,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    queue_outgoing_jobs(state, vec![job]).await;
    Ok(json(202, json!({ "queued": true })))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "messages.list" => list_messages(state, call).await,
        "messages.get" => Ok(json(200, message_json(&find_message(state, &call).await?))),
        "messages.getFlowResponse" => {
            let found = find_message(state, &call).await?;
            let view =
                whatsapp::flow_response_view(&found).map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, view))
        }
        "messages.create" => create(state, call).await,
        "messages.update" => update(state, call).await,
        "messages.delete" => delete(state, call).await,
        "messages.changeAttributes" => change_attributes(state, call).await,
        "messages.sendWhatsappTemplate" => send_template(state, call).await,
        _ => request_call_permission(state, call).await,
    }
}
