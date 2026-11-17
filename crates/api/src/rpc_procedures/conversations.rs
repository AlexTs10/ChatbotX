//! The inbox: `features/conversations/api/private.ts`,
//! `features/messages/api/private.ts` and their server actions
//! (`features/conversations/actions`, `features/messages/actions`).
//!
//! Member scope as in the builder: any member of the workspace may list,
//! open and act on its conversations (no assigned-only filter on these
//! routes); without `emailAndPhone` the keyword search and the contact
//! filter cannot match email / phone (`includeEmailAndPhone`).

use chatbotx_business::conversation_actions as actions;
use chatbotx_business::conversation_list as list;
use chatbotx_business::message::{self, ListForConversation, message_json};
use chatbotx_business::message_actions;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public};
use crate::handlers::contact_messages::{decode_cursor, deliver, encode_cursor, outgoing_input};
use crate::handlers::conversations::{
    conversation_json, encode_cursor as encode_conversation_cursor, list_input, queue_jobs,
};
use crate::handlers::input::Input;
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "archiveConversationAction",
    "unarchiveConversationAction",
    "assignConversationAction",
    "enableBotAction",
    "disableBotAction",
    "readConversationAction",
    "unreadConversationAction",
    "followConversationAction",
    "unfollowConversationAction",
    "createMessageAction",
    "editMessageAction",
    "deleteMessageAction",
    "changeMessageAttributesAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "conversationsAPI.listConversationsAuthenticatedAPI"
        | "conversationsAPI.listConversationsByPOSTAuthenticatedAPI" => {
            list_conversations(ctx, input).await
        }
        "conversationsAPI.findConversationAuthenticatedAPI" => match In(input).id("id") {
            Ok(id) => list::find(&ctx.api.db, ctx.workspace_id, id)
                .await
                .map(|row| json!({ "data": conversation_json(ctx.api, &row, false) }))
                .map_err(fail),
            Err(error) => Err(error),
        },
        "messagesAPI.listMessagesAuthenticatedAPI" => list_messages(ctx, input).await,
        _ => return None,
    })
}

/// `listConversations(input, { includeEmailAndPhone })`.
async fn list_conversations(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let mut object = input.as_object().cloned().ok_or_else(invalid)?;
    object.remove("workspaceId");
    let mut parsed = Input::from_object(object);
    let (mut request, compact) = list_input(&mut parsed, ctx.workspace_id);
    if parsed.failed() {
        return Err(invalid());
    }
    request.hide_email_and_phone = !ctx.has("emailAndPhone");
    let page = list::list(&ctx.api.db, request).await.map_err(fail)?;
    Ok(json!({
        "data": page.rows.iter().map(|row| conversation_json(ctx.api, row, compact)).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor.as_ref().map(encode_conversation_cursor),
        "prevCursor": page.prev_cursor.as_ref().map(encode_conversation_cursor),
    }))
}

/// `listMessages`: newest first, `perPage` default 20.
async fn list_messages(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let limit = match input.get("perPage") {
        None => 20,
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64).ok_or_else(invalid)?,
        Some(Value::String(t)) => t.trim().parse::<f64>().map_err(|_| invalid())? as i64,
        Some(_) => return Err(invalid()),
    };
    let page = message::list_for_conversation(
        &ctx.api.db,
        ListForConversation {
            workspace_id: ctx.workspace_id,
            conversation_id: input.opt_id("conversationId")?,
            contact_inbox_id: input.opt_id("contactInboxId")?,
            cursor: input.opt_str("cursor")?.as_deref().and_then(decode_cursor),
            limit,
        },
    )
    .await
    .map_err(fail)?;
    Ok(json!({
        "data": page.data.iter().map(message_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor.as_ref().map(encode_cursor),
        "prevCursor": Value::Null,
    }))
}

// ---- actions --------------------------------------------------------------

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            match name {
                // `bulkUpdateIdsRequest`: ids of the workspace; others are skipped.
                "archiveConversationAction"
                | "unarchiveConversationAction"
                | "enableBotAction"
                | "disableBotAction" => {
                    for id in In(arg(args, 1)).ids("ids")? {
                        match name {
                            "archiveConversationAction" | "unarchiveConversationAction" => {
                                actions::set_archived(
                                    db,
                                    ws,
                                    id,
                                    name == "archiveConversationAction",
                                )
                                .await
                            }
                            _ => {
                                actions::set_bot_enabled(db, ws, id, name == "enableBotAction")
                                    .await
                            }
                        }
                        .map_err(action_fail)?;
                    }
                    Ok(Value::Null)
                }
                "assignConversationAction" => assign(ctx, arg(args, 1)).await,
                "readConversationAction" | "unreadConversationAction" => {
                    let id = conversation_id(args)?;
                    let event = if name == "readConversationAction" {
                        actions::mark_read(db, ws, id).await
                    } else {
                        actions::mark_unread(db, ws, id).await
                    }
                    .map_err(action_fail)?;
                    let read_at = event.as_ref().map_or(Value::Null, |e| {
                        e.data["changes"]["agentLastReadAt"].clone()
                    });
                    if let Some(event) = event {
                        ctx.api.queue_realtime(ws, event).await;
                    }
                    Ok(json!({ "agentLastReadAt": read_at }))
                }
                "followConversationAction" | "unfollowConversationAction" => {
                    let id = conversation_id(args)?;
                    actions::set_followed(db, ws, id, name == "followConversationAction")
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                "createMessageAction" => create_message(ctx, args).await,
                _ => message_route(ctx, name, args).await,
            }
        }
        .await,
    )
}

/// The second bind arg (`[workspaceId, conversationId]`).
fn conversation_id(args: &[Value]) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, 1) })).id("id")
}

/// `assignByContactIds`: every conversation of `contactIds` gets the
/// assignee (`u_<id>` / `t_<id>` / `null`).
// ponytail: one `assignOne` per conversation; the builder's flow triggers and
// SLA tracking on assign are not ported (P3.6-D3 decision).
async fn assign(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let contact_ids = input.ids("contactIds")?;
    let assigned = match input.0.get("assignedId") {
        Some(Value::Null) => None,
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => return Err(invalid()),
    };
    let ws = ctx.workspace_id;
    let ids: Vec<i64> = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let ids: Vec<chatbotx_db::repo::models::Conversation> = chatbotx_db::repo::find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 AND "contactId" IN (SELECT value FROM json_each(?2))"#,
            &[&ws, &chatbotx_db::repo::id_list(&contact_ids)],
        )?;
        Ok(ids.into_iter().map(|c| c.id).collect())
    })
    .await
    .map_err(fail)?;
    for id in ids {
        let mut done = actions::assign_one(&ctx.api.db, ws, id, assigned.clone())
            .await
            .map_err(action_fail)?;
        if let Some(event) = done.event.take() {
            ctx.api.queue_realtime(ws, event).await;
        }
        // No notification when a member assigns to themself (`assignedBy`).
        if assigned.as_deref() != Some(format!("u_{}", ctx.user_id).as_str()) {
            queue_jobs(ctx.api, done.job).await;
        }
    }
    Ok(Value::Null)
}

/// `createMessageAction`: `createOutgoing` as the member (`senderType`
/// `user`). Delivery is the worker's job (queued here).
async fn create_message(ctx: &Ctx<'_>, args: &[Value]) -> Result<Value, OrpcError> {
    let id = conversation_id(args)?;
    let object = arg(args, 2).as_object().cloned().ok_or_else(invalid)?;
    let outgoing = outgoing_input(Input::from_object(object), &[])?;
    let db = &ctx.api.db;
    let conversation = list::find_or_fail(db, ctx.workspace_id, id)
        .await
        .map_err(action_fail)?;
    let contact_inbox = message_actions::resolve_contact_inbox_for_conversation(
        db,
        &conversation,
        outgoing.inbox_id,
    )
    .await
    .map_err(action_fail)?;
    let sent = message::create_outgoing(
        db,
        &conversation,
        &contact_inbox,
        outgoing,
        Some(ctx.user_id),
    )
    .await
    .map_err(action_fail)?;
    let body = sent.message.as_ref().map_or(Value::Null, message_json);
    deliver(ctx.api, &conversation, sent).await;
    Ok(body)
}

/// `editMessageAction` / `deleteMessageAction` / `changeMessageAttributesAction`:
/// the same operations as the public message routes.
async fn message_route(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let conversation = conversation_id(args)?;
    let mut body = arg(args, 2).as_object().cloned().ok_or_else(invalid)?;
    let id_key = if name == "deleteMessageAction" {
        "id"
    } else {
        "messageId"
    };
    let message = body
        .remove(id_key)
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|t| !t.is_empty())
        .ok_or_else(invalid)?;
    let route = match name {
        "editMessageAction" => "messages.update",
        "deleteMessageAction" => "messages.delete",
        _ => "messages.changeAttributes",
    };
    let params = [
        ("conversationId", conversation.to_string()),
        ("messageId", message),
    ];
    public(ctx, route, &params, None, Some(&Value::Object(body))).await
}
