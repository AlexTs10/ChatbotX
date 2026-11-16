//! `apps/builder/src/features/messages/api/public.ts` (`messagesPublicRouter`,
//! `/v1/conversations/{conversationId}/messages`, scope `inbox`). Workspace
//! tokens only (no member scope): a send is unattributed (`api`). Not ported
//! (501 `notPorted`): media library / device files, a new attachment on
//! edit, the WhatsApp Flow response view, WhatsApp templates and call
//! permission.

use chatbotx_business::BusinessError;
use chatbotx_business::conversation_list as list;
use chatbotx_business::message::{self, ListForConversation, message_json};
use chatbotx_business::message_actions as actions;
use chatbotx_db::repo::models::Conversation;
use serde_json::{Value, json};

use super::contact_messages::{decode_cursor, deliver, encode_cursor, outgoing_input};
use super::conversations::queue_jobs;
use super::input::{Input, Num, Str, key};
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
    list::find_or_fail(&state.db, call.workspace_id, id.unwrap_or(-1))
        .await
        .map_err(|e| business_error(e, errors))
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
    let sent = message::create_outgoing(&state.db, &found, &contact_inbox, outgoing, None)
        .await
        .map_err(|e| business_error(e, CREATING))?;
    let body = sent.message.as_ref().map_or(Value::Null, message_json);
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

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "messages.list" => list_messages(state, call).await,
        "messages.get" => Ok(json(200, message_json(&find_message(state, &call).await?))),
        "messages.getFlowResponse" => {
            let found = find_message(state, &call).await?;
            let attrs = found.message.content_attributes.as_ref().map(|j| &j.0);
            if !actions::is_whatsapp_flow_response(attrs) {
                return Err(business_error(
                    BusinessError::not_found("WhatsApp Flow response not found"),
                    FINDING,
                ));
            }
            Err(not_ported("The WhatsApp Flow response view is", FINDING))
        }
        "messages.create" => create(state, call).await,
        "messages.update" => update(state, call).await,
        "messages.delete" => delete(state, call).await,
        "messages.changeAttributes" => change_attributes(state, call).await,
        _ => {
            // `assertConversationAccessForCaller` runs first: 404 / 422.
            let mut input = Input::from_body(call.params, call.body);
            let conversation_id = input.digits_id("conversationId");
            input.finish()?;
            conversation(state, &call, conversation_id, MUTATING).await?;
            Err(not_ported(
                if route_id == "messages.sendWhatsappTemplate" {
                    "WhatsApp templates are"
                } else {
                    "WhatsApp call permission is"
                },
                MUTATING,
            ))
        }
    }
}
