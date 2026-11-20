//! The inbox: `features/conversations/api/private.ts`,
//! `features/messages/api/private.ts`, the comment-filter post options
//! (`features/channel-posts/api.ts`) and their server actions
//! (`features/conversations/actions`, `features/messages/actions`).
//!
//! Member scope as in the builder: any member of the workspace may list,
//! open and act on its conversations (no assigned-only filter on these
//! routes); without `emailAndPhone` the keyword search and the contact
//! filter cannot match email / phone (`includeEmailAndPhone`).

use chatbotx_business::BusinessError;
use chatbotx_business::conversation_actions as actions;
use chatbotx_business::conversation_list as list;
use chatbotx_business::message::{self, ListForConversation};
use chatbotx_business::message_actions;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, query_of};
use crate::handlers::contact_messages::{decode_cursor, deliver, encode_cursor, outgoing_input};
use crate::handlers::conversations::{
    conversation_json, encode_cursor as encode_conversation_cursor, list_input, queue_jobs,
};
use crate::handlers::input::{Input, Str};
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
    "threadControlAction",
    "syncThreadOwnerAction",
    "sendWhatsappTemplateAction",
    // The plain `"use server"` exports (`fn({ workspaceId, conversationId, parsedInput })`).
    "editMessage",
    "deleteMessage",
    "changeMessageAttributes",
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
        "messagesAPI.getWhatsappFlowResponseAuthenticatedAPI" => flow_response(ctx, input).await,
        "conversationsAPI.getPostDetailsAuthenticatedAPI" => post_details(ctx, input).await,
        "channelPostAPIs.privateListChannelPostOptionsAPI" => {
            let mut query = query_of(input, &["limit", "search"]);
            if let Some(cursor) = input.get("cursor").filter(|c| !c.is_null()) {
                let pairs = [("id", &cursor["id"]), ("sortAt", &cursor["sortAt"])];
                for (key, value) in pairs {
                    let text = match value {
                        Value::String(text) => text.clone(),
                        Value::Null => continue,
                        other => other.to_string(),
                    };
                    let pair = query_of(&json!({ "v": text }), &["v"]);
                    query.push_str(&format!("&cursor%5B{key}%5D={}", &pair[2..]));
                }
            }
            channel_posts(ctx, "channelPosts.list", query.trim_start_matches('&')).await
        }
        "channelPostAPIs.privateGetChannelPostOptionsByIdsAPI" => {
            let mut query = String::new();
            match input.get("ids") {
                Some(Value::Array(ids)) => {
                    for (index, id) in ids.iter().enumerate() {
                        let text = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
                        let pair = query_of(&json!({ "v": text }), &["v"]);
                        query.push_str(&format!("&ids%5B{index}%5D={}", &pair[2..]));
                    }
                }
                // `ids` missing / not an array: the route's own issue.
                Some(other) => query.push_str(&format!("&ids={}", other)),
                None => {}
            }
            channel_posts(
                ctx,
                "channelPosts.listSelected",
                query.trim_start_matches('&'),
            )
            .await
        }
        _ => return None,
    })
}

/// A channel-post option route (an input error is oRPC's validation error).
async fn channel_posts(ctx: &Ctx<'_>, route: &str, query: &str) -> Result<Value, OrpcError> {
    public(ctx, route, &[], Some(query), None)
        .await
        .map_err(|error| match error.status {
            422 => invalid(),
            _ => error,
        })
}

/// `z.coerce.date()`.
fn coerce_date(value: Option<&Value>) -> Result<i64, OrpcError> {
    match value {
        Some(Value::String(text)) => chatbotx_core::js::parse_date(text).ok_or_else(invalid),
        Some(Value::Number(n)) => n.as_f64().map(|n| n as i64).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

/// `getWhatsappFlowResponseView`: the message is looked up in the workspace,
/// then must belong to `conversationId` ("Message not found").
async fn flow_response(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let inp = In(input);
    let conversation_id = inp.id("conversationId")?;
    let message_id = inp.id("messageId")?;
    let created_at = coerce_date(inp.get("createdAt"))?;
    let found = message_actions::find_by_id_with_urls(
        &ctx.api.db,
        ctx.workspace_id,
        message_id,
        created_at,
    )
    .await
    .map_err(fail)?;
    if found.message.conversation_id != conversation_id {
        return Err(fail(BusinessError::not_found("Message not found")));
    }
    crate::handlers::messages::flow_response(ctx.api, ctx.workspace_id, &found).await
}

/// `getPostDetailsQuery` (`channel` is a `channelTypes` value).
async fn post_details(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let inp = In(input);
    let inbox_id = inp.opt_str("inboxId")?.ok_or_else(invalid)?;
    let post_id = inp.opt_str("postId")?.ok_or_else(invalid)?;
    let channel = inp.opt_str("channel")?.ok_or_else(invalid)?;
    if !chatbotx_business::connection::providers::CHANNEL_TYPES.contains(&channel.as_str()) {
        return Err(invalid());
    }
    chatbotx_business::post_details::get_post_details(
        &ctx.api.db,
        &chatbotx_business::connection::credentials::HttpUpstream,
        ctx.workspace_id,
        &inbox_id,
        &post_id,
        &channel,
    )
    .await
    .map_err(fail)
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
    let data =
        crate::handlers::contact_messages::messages_json(ctx.api, ctx.workspace_id, &page.data)
            .await
            .map_err(fail)?;
    Ok(json!({
        "data": data,
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
                                    ctx.api.jobs.as_deref(),
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
                "sendWhatsappTemplateAction" => send_template(ctx, args).await,
                "editMessage" | "deleteMessage" | "changeMessageAttributes" => {
                    plain_message_function(ctx, name, args).await
                }
                "threadControlAction" | "syncThreadOwnerAction" => {
                    thread_control(ctx, name, args).await
                }
                _ => message_route(ctx, name, args).await,
            }
        }
        .await,
    )
}

/// `threadControlAction` (`[workspaceId, conversationId]` + take / release / pass) and
/// `syncThreadOwnerAction` (`[workspaceId]` + `{ contactInboxId, conversationId }`).
async fn thread_control(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    use chatbotx_business::thread_control as tc;
    let sync = name == "syncThreadOwnerAction";
    let raw = arg(args, if sync { 1 } else { 2 });
    let mut input = Input::from_object(raw.as_object().cloned().ok_or_else(invalid)?);
    let (contact_inbox, action, target_role, metadata, conversation_id) = if sync {
        let ci = input.digits_id("contactInboxId");
        (
            ci,
            Some("sync".to_owned()),
            None,
            None,
            input.digits_id("conversationId"),
        )
    } else {
        let request = crate::handlers::conversations::thread_control_request(
            &mut input,
            &["take", "release", "pass"],
        );
        let id = conversation_id(args)?;
        (
            request.contact_inbox,
            request.action,
            request.target_role,
            request.metadata,
            Some(id),
        )
    };
    input.finish()?;
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let conversation = list::find_or_fail(db, ws, conversation_id.unwrap_or(-1))
        .await
        .map_err(action_fail)?;
    // `requireContactAccessForMember`.
    let member = ctx.require_contact_scope()?;
    if super::contacts::scoped_ids(ctx, member, vec![conversation.contact_id])
        .await?
        .is_empty()
    {
        return Err(action_fail(BusinessError::not_found("Contact not found")));
    }
    let (action, contact_inbox) = (action.unwrap_or_default(), contact_inbox.unwrap_or(-1));
    let (graph, rt) = (ctx.api.graph_url.as_str(), ctx.api.realtime.as_ref());
    let result = if sync {
        tc::sync_conversation_owner(db, rt, graph, &conversation, contact_inbox).await
    } else {
        tc::request_conversation_action(
            db,
            rt,
            graph,
            &conversation,
            contact_inbox,
            &action,
            target_role.as_deref(),
            metadata.as_deref(),
        )
        .await
    };
    let not_found = || {
        action_fail(BusinessError::not_found(
            "This conversation is not on that channel",
        ))
    };
    match result {
        Ok(row) => Ok(json!({
            "status": if sync { "synced" } else { "applied" },
            "snapshot": tc::snapshot_json(&row),
        })),
        Err(tc::ThreadControlError::Business(error)) if error.code() == Some("notFound") => {
            Err(not_found())
        }
        Err(error) if sync => Err(action_fail(match error {
            tc::ThreadControlError::Business(error) => error,
            other => BusinessError::exception(other.to_string(), "systemError", 500),
        })),
        Err(error) => match tc::map_error(error, &action, &tc::INBOX_COPY) {
            Ok(status) => Ok(json!({ "status": status })),
            Err(error) => Err(action_fail(error)),
        },
    }
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
    let outgoing = outgoing_input(Input::from_object(object)).map_err(super::schema_errors)?;
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
    let (sent, body) = crate::handlers::contact_messages::create_outgoing(
        ctx.api,
        &conversation,
        &contact_inbox,
        outgoing,
        Some(ctx.user_id),
    )
    .await
    .map_err(action_fail)?;
    deliver(ctx.api, &conversation, sent).await;
    Ok(body)
}

/// `sendWhatsappTemplateAction` (`[workspaceId, conversationId]` +
/// `sendWhatsappTemplateRequest`): queues the template send as the member.
async fn send_template(ctx: &Ctx<'_>, args: &[Value]) -> Result<Value, OrpcError> {
    let id = conversation_id(args)?;
    let object = arg(args, 2).as_object().cloned().ok_or_else(invalid)?;
    let mut input = Input::from_object(object);
    let template_id = input.string("templateId", Str::trimmed(1, usize::MAX));
    let data = crate::handlers::messages::template_data(&mut input);
    let inbox_id = if input.has("inboxId") {
        input.digits_id("inboxId")
    } else {
        None
    };
    input.finish().map_err(super::schema_errors)?;
    let db = &ctx.api.db;
    let conversation = list::find_or_fail(db, ctx.workspace_id, id)
        .await
        .map_err(action_fail)?;
    let job = chatbotx_business::whatsapp::send_template_to_conversation(
        db,
        &conversation,
        &template_id.unwrap_or_default(),
        data,
        inbox_id,
        Some(ctx.user_id),
    )
    .await
    .map_err(action_fail)?;
    crate::handlers::contact_messages::queue_outgoing_jobs(ctx.api, vec![job]).await;
    Ok(Value::Null)
}

/// The plain `editMessage` / `deleteMessage` / `changeMessageAttributes`
/// exports: one `{ workspaceId, conversationId, parsedInput }` argument. The
/// `/rpc` membership check ran for the call's `workspaceId`; the props must
/// name the same workspace.
async fn plain_message_function(
    ctx: &Ctx<'_>,
    name: &str,
    args: &[Value],
) -> Result<Value, OrpcError> {
    let props = arg(args, 0);
    if props["workspaceId"].as_str() != Some(ctx.workspace_id.to_string().as_str()) {
        return Err(fail(BusinessError::not_found("Workspace not found")));
    }
    let bound = [
        props["workspaceId"].clone(),
        props["conversationId"].clone(),
        props["parsedInput"].clone(),
    ];
    message_route(ctx, &format!("{name}Action"), &bound).await
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
        "deleteMessageAction" => "messages.deleteAction",
        _ => "messages.changeAttributes",
    };
    let params = [
        ("conversationId", conversation.to_string()),
        ("messageId", message),
    ];
    public(ctx, route, &params, None, Some(&Value::Object(body))).await
}
