//! The `chat` queue (`apps/worker/src/chat/worker.ts`) and
//! `chat/handlers/send-message.ts`: channel sends of stored messages,
//! comment delete / edit / like / hide, typing.

use chatbotx_db::repo::message::{
    self as repo_message, FindMessageByIdParams, MessageWithAttachments,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobCtx, JobError};
use chatbotx_rt::protocol::{ConversationRoute, route_for_conversation};
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Value, json};

use crate::channel::{
    OutgoingContact, SendFailure, SendMessage, SendResult, SendTyping,
    should_suppress_retryable_channel_error, will_send_retry,
};
use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, ms_of, str_of};

/// `ChatJobAction` names this port handles.
pub const SEND_CHANNEL_MESSAGE: &str = "sendChannelMessage";
pub const SEND_FLOW_MESSAGE: &str = "sendFlowMessage";
pub const SEND_CHAT_MESSAGE: &str = "sendChatMessage";
pub const SEND_TYPING: &str = "sendTyping";
pub const DELETE_CHANNEL_MESSAGE: &str = "deleteChannelMessage";
pub const EDIT_CHANNEL_MESSAGE: &str = "editChannelMessage";
pub const CHANGE_CHANNEL_MESSAGE_STATE: &str = "changeChannelMessageState";
pub const CHECK_OUTBOUND_AUTOMATED_RESPONSE: &str = "checkOutboundAutomatedResponse";
pub const NOTIFY_EXPORT_RESULT: &str = "notifyExportResult";

/// `MAX_SEND_ERROR_LENGTH`.
const MAX_SEND_ERROR_LENGTH: usize = 500;

/// The chat worker processor: dispatch on `job.data.type`.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type")
        .unwrap_or(job.name.as_str())
        .to_owned();
    let data = job.data.get("data").cloned().unwrap_or(Value::Null);
    let will_retry = !job.is_final_attempt();
    match kind.as_str() {
        SEND_CHANNEL_MESSAGE => {
            send_message_to_channel(&deps, &data, job.attempts_made, will_retry).await?;
        }
        SEND_FLOW_MESSAGE => crate::send_flow_step::send_flow_step(&deps, &data).await?,
        SEND_CHAT_MESSAGE => crate::send_flow_step::send_chat_message(&deps, &data).await?,
        SEND_TYPING => send_typing_to_channel(&deps, &data).await?,
        crate::wa_template::SEND_TEMPLATE_TO_CONVERSATION => {
            crate::wa_template::send_template_to_conversation(&deps, &data).await?
        }
        DELETE_CHANNEL_MESSAGE => delete_message_from_channel(&deps, &data).await?,
        EDIT_CHANNEL_MESSAGE => edit_message_from_channel(&deps, &data).await?,
        CHANGE_CHANNEL_MESSAGE_STATE => change_message_state_on_channel(&deps, &data).await?,
        NOTIFY_EXPORT_RESULT => {
            tracing::warn!(job_id = %job.job_id, "notifyExportResult job received but no handler is implemented");
        }
        // ponytail: the outbound automated-response cancel needs the
        // automated-response session port (Q.D6); the job is a no-op until then.
        CHECK_OUTBOUND_AUTOMATED_RESPONSE => {
            tracing::debug!(job_id = %job.job_id, "checkOutboundAutomatedResponse: not ported, skipped");
        }
        "broadcastEvent" => {
            tracing::info!(job_id = %job.job_id, "Ignoring legacy broadcastEvent job during realtime v2 rollout");
        }
        // Messenger templates are the Messenger unit's (Q.I1b).
        other => {
            return Err(JobError::unrecoverable(format!(
                "ChatJobAction action is not defined: {other}"
            )));
        }
    }
    Ok(Value::Null)
}

/// The conversation and contact inbox a chat job names (re-read by id: the
/// job carries reduced copies).
pub(crate) async fn load_pair(
    deps: &Deps,
    data: &Value,
) -> Result<Option<(Conversation, ContactInbox)>, JobError> {
    let conversation_id = data
        .get("conversation")
        .or_else(|| data.get("conversationId"))
        .and_then(id_of)
        .ok_or_else(|| JobError::unrecoverable("job has no conversation"))?;
    let Some(conversation) = load_conversation(&deps.db, conversation_id).await? else {
        return Ok(None);
    };
    let contact_inbox_id = data
        .get("contactInbox")
        .or_else(|| data.get("contactInboxId"))
        .and_then(id_of)
        .ok_or_else(|| JobError::unrecoverable("job has no contact inbox"))?;
    let Some(contact_inbox) =
        load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    else {
        return Ok(None);
    };
    Ok(Some((conversation, contact_inbox)))
}

pub(crate) fn route_of(conversation: &Conversation, inbox_id: i64) -> chatbotx_rt::protocol::Route {
    route_for_conversation(
        Some(&ConversationRoute {
            assigned_inbox_team_id: conversation.assigned_inbox_team_id.map(|v| v.to_string()),
            assigned_user_id: conversation.assigned_user_id.map(|v| v.to_string()),
            inbox_id: Some(inbox_id.to_string()),
        }),
        None,
    )
}

pub(crate) async fn queue_event(
    deps: &Deps,
    conversation: &Conversation,
    inbox_id: i64,
    event_type: EventType,
    data: Value,
) {
    if let Some(realtime) = &deps.realtime {
        realtime
            .queue_workspace_event(
                &conversation.workspace_id.to_string(),
                Envelope::routed(event_type, data, route_of(conversation, inbox_id)),
            )
            .await;
    }
}

/// `messageService.findWithAttachments({ id, createdAt, workspaceId })`.
pub(crate) async fn find_message(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
    created_at: i64,
) -> Result<Option<MessageWithAttachments>, JobError> {
    deps.db
        .read(move |conn| {
            repo_message::find_by_id(
                conn,
                &FindMessageByIdParams {
                    id,
                    created_at,
                    workspace_id,
                    conversation_id: None,
                },
            )
            .map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)
}

/// `persistMessageSendError`: writes (or clears) `sendError`, then the
/// `messageFailed` event unless silent. Failures are logged.
pub(crate) async fn persist_send_error(
    deps: &Deps,
    conversation: &Conversation,
    inbox_id: i64,
    message: (i64, i64, Option<String>),
    send_error: Option<String>,
    silent: bool,
) {
    let (id, created_at, client_id) = message;
    let workspace_id = conversation.workspace_id;
    let error = send_error.clone();
    let written = deps
        .db
        .write(move |tx| {
            repo_message::update_send_error(tx, id, error.as_deref(), workspace_id, created_at)
                .map_err(crate::repo_err)
        })
        .await;
    if let Err(err) = written {
        tracing::error!(err = %err, message_id = id, workspace_id, "Failed to persist message sendError");
        return;
    }
    if silent {
        return;
    }
    queue_event(
        deps,
        conversation,
        inbox_id,
        EventType::MessageFailed,
        json!({ "messageId": id.to_string(), "clientId": client_id, "error": send_error }),
    )
    .await;
}

/// `contactInboxService.recordOutboundMessageSent` (`updateTracking`).
pub(crate) async fn record_outbound_message_sent(
    deps: &Deps,
    contact_inbox: &ContactInbox,
    workspace_id: i64,
    at: i64,
) {
    let (id, contact_id) = (contact_inbox.id, contact_inbox.contact_id);
    let result = deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "ContactInbox"
                   SET "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1
                         THEN ?1 ELSE "firstInteractionAt" END,
                       "lastMessageAt" = ?1, "lastOutboundMessageAt" = ?1,
                       "consecutiveFailedReply" = 0, "lastErrorLog" = NULL, "updatedAt" = ?2
                   WHERE "id" = ?3 AND "contactId" = ?4
                     AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?5)"#,
                params![at, now_ms(), id, contact_id, workspace_id],
            )?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, contact_inbox_id = id, "recordOutboundMessageSent failed");
    }
}

/// `conversationService.markReadByOutbound` (the inbox option
/// `markReadOnOutbound`), logged on failure.
pub(crate) async fn mark_read_after_delivery(
    deps: &Deps,
    conversation: &Conversation,
    inbox_id: i64,
    read_at: i64,
    silent: bool,
) {
    let (id, workspace_id) = (conversation.id, conversation.workspace_id);
    let updated = deps
        .db
        .write(move |tx| {
            Ok(tx.execute(
                r#"UPDATE "Conversation" SET "agentLastReadAt" = ?1
                   WHERE "id" = ?2 AND "workspaceId" = ?3
                     AND ("agentLastReadAt" IS NULL OR "agentLastReadAt" < ?1)
                     AND EXISTS (SELECT 1 FROM "Inbox" WHERE "id" = ?4 AND "workspaceId" = ?3
                                 AND "markReadOnOutbound" = 1)"#,
                params![read_at, id, workspace_id, inbox_id],
            )?)
        })
        .await;
    match updated {
        Ok(rows) if rows > 0 && !silent => {
            queue_event(
                deps,
                conversation,
                inbox_id,
                EventType::ConversationUpdated,
                json!({
                    "conversationIds": [id.to_string()],
                    "changes": { "agentLastReadAt": chatbotx_db::repo::iso(read_at) },
                }),
            )
            .await;
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(err = %err, conversation_id = id, "markReadByOutbound after a delivered send failed");
        }
    }
}

/// `isBulkOutboundMetadata`: a sequence payload, or a broadcast payload of a
/// bulk send (`isBulkBroadcast`); a broadcast button continuation is not bulk.
pub(crate) fn is_bulk_outbound(metadata: Option<&Value>, is_bulk_broadcast: bool) -> bool {
    metadata.is_some_and(|m| {
        chatbotx_core::flow_config::is_bulk_outbound_metadata(m, is_bulk_broadcast)
    })
}

/// `sendMessageToChannel`. `attempts_made` > 0 clears an earlier send
/// error after a success.
pub async fn send_message_to_channel(
    deps: &Deps,
    data: &Value,
    attempts_made: u32,
    will_retry_on_throw: bool,
) -> Result<SendResult, JobError> {
    let Some((conversation, contact_inbox)) = load_pair(deps, data).await? else {
        return Err(fail("Conversation not found"));
    };
    let message = data.get("message").cloned().unwrap_or(Value::Null);
    let metadata = data.get("metadata").filter(|m| !m.is_null());
    let is_bulk = is_bulk_outbound(metadata, data["isBulkBroadcast"] == json!(true))
        || is_bulk_outbound(
            message.pointer("/contentAttributes/metadata"),
            data["isBulkBroadcast"] == json!(true),
        );
    let message_id = id_of(&message["id"]);
    let created_at = ms_of(&message["createdAt"]);
    let client_id = str_of(&message, "clientId").map(str::to_owned);
    let is_comment = str_of(&message, "type") == Some("comment");
    let is_private_reply =
        is_comment && message.pointer("/contentAttributes/isPrivateReply") == Some(&json!(true));

    let outcome = dispatch_send(deps, &conversation, &contact_inbox, data, message.clone()).await;
    let result = match outcome {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(err = %error, "An error occurred while sending the message");
            let will_retry = will_send_retry(&error, &contact_inbox.channel, will_retry_on_throw);
            crate::events::emit_message_failed(
                deps.events.as_ref(),
                &crate::events::MessageContext {
                    workspace_id: conversation.workspace_id,
                    contact_id: conversation.contact_id,
                    conversation_id: conversation.id,
                    channel: contact_inbox.channel.clone(),
                    contact_inbox_id: contact_inbox.id,
                    inbox_id: contact_inbox.inbox_id,
                    source_id: contact_inbox.source_id.clone(),
                },
                message_id,
                crate::events::parsed_error(&error),
                metadata,
                will_retry,
            )
            .await;
            if let (Some(id), Some(created_at)) = (message_id, created_at) {
                let text: String = error
                    .message()
                    .chars()
                    .take(MAX_SEND_ERROR_LENGTH)
                    .collect();
                persist_send_error(
                    deps,
                    &conversation,
                    contact_inbox.inbox_id,
                    (id, created_at, client_id),
                    Some(text),
                    is_bulk,
                )
                .await;
            }
            if should_suppress_retryable_channel_error(&error, &contact_inbox.channel) {
                return Ok(SendResult::default());
            }
            return Err(fail(error.message().to_owned()));
        }
    };

    if let (Some(id), Some(created_at)) = (message_id, created_at) {
        if let Some(provider_id) = result.message_ids.first().cloned() {
            let workspace_id = conversation.workspace_id;
            let source = provider_id.clone();
            let updated = deps
                .db
                .write(move |tx| {
                    repo_message::update_source_id(tx, id, &source, workspace_id, created_at)
                        .map_err(crate::repo_err)
                })
                .await;
            match updated {
                Err(err) => {
                    tracing::error!(err = %err, "Failed to update message sourceId with provider id");
                }
                Ok(_) if is_comment && message["parentId"].is_string() => {
                    queue_event(
                        deps,
                        &conversation,
                        contact_inbox.inbox_id,
                        EventType::MessageIdAssigned,
                        json!({ "messageId": id.to_string(), "commentId": provider_id }),
                    )
                    .await;
                }
                Ok(_) => {}
            }
        }
        if attempts_made > 0 {
            persist_send_error(
                deps,
                &conversation,
                contact_inbox.inbox_id,
                (id, created_at, client_id),
                None,
                is_bulk,
            )
            .await;
        }
    }
    let at = created_at.unwrap_or_else(now_ms);
    record_outbound_message_sent(deps, &contact_inbox, conversation.workspace_id, at).await;
    // A public comment reply reaches no one's inbox; a private reply does.
    let is_direct = !is_comment || is_private_reply;
    if is_direct {
        record_delivered_when_no_receipts(
            deps,
            &contact_inbox,
            result.sent_count,
            conversation.workspace_id,
        )
        .await;
    }
    if is_direct && result.sent_count > 0 {
        mark_read_after_delivery(deps, &conversation, contact_inbox.inbox_id, at, is_bulk).await;
    }
    if !is_comment {
        unblock_if_blocked(deps, conversation.workspace_id, conversation.contact_id).await;
    }
    Ok(result)
}

/// `recordDeliveredWhenNoReceipts`: on a channel without delivery receipts
/// the accepted send is the "Last Delivered" moment (a channel with
/// receipts is recorded by the delivered listener). Logged, never rethrown:
/// the send already happened.
async fn record_delivered_when_no_receipts(
    deps: &Deps,
    contact_inbox: &ContactInbox,
    sent_count: u32,
    workspace_id: i64,
) {
    let Some(channel) = deps.channels.get(&contact_inbox.channel) else {
        return;
    };
    if channel.delivery_receipts() || sent_count == 0 {
        return;
    }
    let contact_id = contact_inbox.contact_id;
    let result = deps
        .db
        .write(move |tx| {
            chatbotx_db::repo::contact_activity::upsert_last_delivered(
                tx,
                workspace_id,
                &[(contact_id, now_ms())],
            )
            .map_err(crate::repo_err)
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, workspace_id, "last-delivered: failed to record a successful send");
    }
}

/// `contactService.unblockIfBlocked` after a successful non-comment send
/// (best effort).
async fn unblock_if_blocked(deps: &Deps, workspace_id: i64, contact_id: i64) {
    let blocked = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "blockedAt" IS NOT NULL FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![contact_id, workspace_id],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .await;
    let result = match blocked {
        Ok(false) => return,
        Ok(true) => chatbotx_business::contact::service::unblock_and_record(
            &deps.db,
            workspace_id,
            contact_id,
        )
        .await
        .map(drop)
        .map_err(|e| e.message()),
        Err(err) => Err(err.to_string()),
    };
    if let Err(err) = result {
        tracing::warn!(err = %err, "Auto-unblock on successful send failed");
    }
}

async fn dispatch_send(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    data: &Value,
    mut message: Value,
) -> Result<SendResult, SendFailure> {
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            contact_inbox,
        )
        .await?;
    let is_comment = str_of(&message, "type") == Some("comment");
    if is_comment
        && let (Some(parent_id), Some(parent_created_at)) = (
            id_of(&message["parentId"]),
            ms_of(&message["parentCreatedAt"]),
        )
    {
        let parent = find_message(
            deps,
            conversation.workspace_id,
            parent_id,
            parent_created_at,
        )
        .await
        .map_err(|e| SendFailure::Other(e.to_string()))?;
        let reply_to = parent
            .as_ref()
            .and_then(|p| p.message.source_id.clone())
            .map(Value::String)
            .or_else(|| {
                message
                    .pointer("/contentAttributes/replyToCommentId")
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let post_id = parent
            .as_ref()
            .and_then(|p| p.message.content_attributes.as_ref())
            .and_then(|a| a.0.get("postId"))
            .filter(|v| v.is_string())
            .cloned()
            .or_else(|| message.pointer("/contentAttributes/postId").cloned())
            .unwrap_or(Value::Null);
        let attributes = message
            .get("contentAttributes")
            .filter(|a| a.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut attributes = attributes;
        attributes["replyToCommentId"] = reply_to;
        attributes["postId"] = post_id;
        message["contentAttributes"] = attributes;
    }
    let is_private_reply =
        is_comment && message.pointer("/contentAttributes/isPrivateReply") == Some(&json!(true));
    let send = SendMessage {
        contact: OutgoingContact::new(contact_inbox, conversation),
        message,
        quick_replies: if is_comment {
            None
        } else {
            data.get("quickReplies").filter(|q| !q.is_null()).cloned()
        },
        metadata: data.get("metadata").filter(|m| !m.is_null()).cloned(),
        send_from: str_of(data, "sendFrom").map(str::to_owned),
    };
    if is_private_reply {
        channel.send_private_reply(&ctx, send).await
    } else if is_comment {
        channel.send_comment(&ctx, send).await
    } else {
        channel.send_message(&ctx, send).await
    }
}

/// The comment row of a delete / edit / state job: `None` (logged) when
/// it is gone or not a channel comment.
async fn comment_of(
    deps: &Deps,
    conversation: &Conversation,
    data: &Value,
    handler: &str,
) -> Result<Option<MessageWithAttachments>, JobError> {
    let message = &data["message"];
    let (Some(id), Some(created_at)) = (id_of(&message["id"]), ms_of(&message["createdAt"])) else {
        return Err(JobError::unrecoverable("job has no message"));
    };
    let Some(found) = find_message(deps, conversation.workspace_id, id, created_at).await? else {
        tracing::warn!(message_id = id, "{handler}: message not found in shard");
        return Ok(None);
    };
    if found.message.r#type != "comment" || found.message.source_id.is_none() {
        tracing::warn!(
            message_id = id,
            r#type = %found.message.r#type,
            "{handler}: message is not a comment or has no sourceId, skipping"
        );
        return Ok(None);
    }
    Ok(Some(found))
}

fn channel_fail(error: SendFailure) -> JobError {
    fail(error.message().to_owned())
}

/// `deleteMessageFromChannel` (no comment moderation log yet: Q.D10).
pub async fn delete_message_from_channel(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some((conversation, contact_inbox)) = load_pair(deps, data).await? else {
        return Ok(());
    };
    let Some(found) = comment_of(deps, &conversation, data, "deleteMessageFromChannel").await?
    else {
        return Ok(());
    };
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            &contact_inbox,
        )
        .await
        .map_err(channel_fail)?;
    channel
        .delete_comment(&ctx, found.message.source_id.unwrap_or_default())
        .await
        .map_err(channel_fail)
}

/// `editMessageFromChannel`.
pub async fn edit_message_from_channel(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some((conversation, contact_inbox)) = load_pair(deps, data).await? else {
        return Ok(());
    };
    let Some(found) = comment_of(deps, &conversation, data, "editMessageFromChannel").await? else {
        return Ok(());
    };
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            &contact_inbox,
        )
        .await
        .map_err(channel_fail)?;
    channel
        .edit_comment(
            &ctx,
            found.message.source_id.unwrap_or_default(),
            str_of(data, "newText").unwrap_or_default().to_owned(),
            str_of(data, "newAttachmentUrl").map(str::to_owned),
        )
        .await
        .map_err(channel_fail)
}

/// `changeMessageStateOnChannel`: stores `{ liked, hidden }`, then likes /
/// hides the comment on the channel.
pub async fn change_message_state_on_channel(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some((conversation, contact_inbox)) = load_pair(deps, data).await? else {
        return Ok(());
    };
    let Some(found) = comment_of(deps, &conversation, data, "changeMessageStateOnChannel").await?
    else {
        return Ok(());
    };
    let liked = data.get("liked").and_then(Value::as_bool);
    let hidden = data.get("hidden").and_then(Value::as_bool);
    let current = found.message.attributes.as_ref().map(|a| a.0.clone());
    let current_flag = |key: &str| {
        current
            .as_ref()
            .and_then(|c| c.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let new_liked = liked.unwrap_or_else(|| current_flag("liked"));
    let new_hidden = hidden.unwrap_or_else(|| current_flag("hidden"));
    let (id, workspace_id, created_at) = (
        found.message.id,
        conversation.workspace_id,
        found.message.created_at,
    );
    deps.db
        .write(move |tx| {
            repo_message::update_message_attributes(
                tx,
                id,
                workspace_id,
                new_liked,
                new_hidden,
                created_at,
            )
            .map_err(crate::repo_err)?;
            Ok(())
        })
        .await
        .map_err(db_fail)?;
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            &contact_inbox,
        )
        .await
        .map_err(channel_fail)?;
    let comment_id = found.message.source_id.clone().unwrap_or_default();
    if let Some(liked) = liked {
        channel
            .like_comment(&ctx, comment_id.clone(), liked)
            .await
            .map_err(channel_fail)?;
    }
    if let Some(hidden) = hidden {
        let post_id = found
            .message
            .content_attributes
            .as_ref()
            .and_then(|a| a.0.get("postId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        channel
            .hide_comment(&ctx, comment_id, hidden, post_id)
            .await
            .map_err(channel_fail)?;
    }
    Ok(())
}

/// `sendTypingToChannel`: best effort; an unregistered channel is skipped.
pub async fn send_typing_to_channel(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some((conversation, contact_inbox)) = load_pair(deps, data).await? else {
        return Ok(());
    };
    if deps.channels.get(&contact_inbox.channel).is_none() {
        tracing::debug!(
            "No integration registered for typing on channel: {}",
            contact_inbox.channel
        );
        return Ok(());
    }
    let (channel, ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            conversation.workspace_id,
            &contact_inbox,
        )
        .await
        .map_err(channel_fail)?;
    // ponytail: the WhatsApp wamid anchor (`resolveWhatsappMessageSourceId`)
    // comes with the WhatsApp channel unit (Q.I1).
    channel
        .send_typing(
            &ctx,
            SendTyping {
                contact: OutgoingContact::new(&contact_inbox, &conversation),
                typing: data.get("typing").and_then(Value::as_bool).unwrap_or(true),
                seconds: data.get("seconds").and_then(Value::as_f64),
                message_source_id: None,
            },
        )
        .await
        .map_err(channel_fail)
}
