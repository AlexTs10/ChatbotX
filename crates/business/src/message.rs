//! `message/list-for-conversation.ts` (`listForConversation`,
//! `findForContact`) and a reduced `message/create-outgoing.ts`
//! (`createOutgoing`: text, flow and the product-message refusals; no files,
//! media library files, private replies, image limits or agent-reply metric).
//!
//! The service does not enqueue or publish: [`Outgoing`] carries the jobs
//! (`sendChannelMessage` on `chat`, `sendFlow` on `integration`) and the
//! realtime `messageCreated` payload for the caller (`ApiState`).
//! Attachment URLs are not presigned (no `resolveMediaUrl` port yet):
//! `url` / `fallbackUrl` are `null`.

use chatbotx_db::Db;
use chatbotx_db::repo::message::{
    self as repo_message, CreateMessageInput, FindTriggerMessageOptions, ListMessagesQuery,
    MessageWithAttachments, Pagination, PaginationCursor, SinceTimeInput, get_safe_since_time,
};
use chatbotx_db::repo::models::{Attachment, ContactInbox, Conversation, Message};
use chatbotx_db::repo::{Model, flow, iso, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_jobs::QueueName;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

const HOUR_MS: i64 = 60 * 60 * 1000;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn sid(value: i64) -> Value {
    Value::String(value.to_string())
}

fn opt_sid(value: Option<i64>) -> Value {
    value.map_or(Value::Null, sid)
}

fn opt_iso(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |ms| Value::String(iso(ms)))
}

/// `conversationService.findBy({ id, workspaceId })`.
fn conversation_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Conversation>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            Conversation::select_sql()
        ),
        params![id, workspace_id],
        Conversation::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// The conversation fields the job payloads carry (reduced; the worker
/// port reads the rest by id).
pub fn conversation_json(conversation: &Conversation) -> Value {
    json!({
        "id": sid(conversation.id),
        "workspaceId": sid(conversation.workspace_id),
        "contactId": sid(conversation.contact_id),
        "sourceId": conversation.source_id,
        "assignedUserId": opt_sid(conversation.assigned_user_id),
        "assignedInboxTeamId": opt_sid(conversation.assigned_inbox_team_id),
        "botEnabled": conversation.bot_enabled,
        "createdAt": iso(conversation.created_at),
        "updatedAt": iso(conversation.updated_at),
    })
}

/// The contact inbox fields the job payloads carry (reduced).
pub fn contact_inbox_json(contact_inbox: &ContactInbox) -> Value {
    json!({
        "id": sid(contact_inbox.id),
        "contactId": sid(contact_inbox.contact_id),
        "inboxId": sid(contact_inbox.inbox_id),
        "channel": contact_inbox.channel,
        "source": contact_inbox.source,
        "sourceId": contact_inbox.source_id,
        "sourceUserId": contact_inbox.source_user_id,
        "personaId": contact_inbox.persona_id,
        "lastMessageAt": opt_iso(contact_inbox.last_message_at),
    })
}

fn attachment_json(attachment: &Attachment) -> Value {
    json!({
        "id": sid(attachment.id),
        "createdAt": iso(attachment.created_at),
        "updatedAt": iso(attachment.updated_at),
        "workspaceId": sid(attachment.workspace_id),
        "conversationId": sid(attachment.conversation_id),
        "messageId": sid(attachment.message_id),
        "messageCreatedAt": iso(attachment.message_created_at),
        "fileType": attachment.file_type,
        "sourceId": attachment.source_id,
        "mimeType": attachment.mime_type,
        "width": attachment.width,
        "height": attachment.height,
        "size": attachment.size,
        "thumbnailPath": attachment.thumbnail_path,
        "originPath": attachment.origin_path,
        "name": attachment.name,
        "keepOriginal": attachment.keep_original,
        // ponytail: no presigning yet (`resolveMediaUrl` not ported).
        "url": Value::Null,
        "fallbackUrl": Value::Null,
    })
}

fn message_row_json(message: &Message) -> serde_json::Map<String, Value> {
    let out = json!({
        "id": sid(message.id),
        "createdAt": iso(message.created_at),
        "updatedAt": iso(message.updated_at),
        "conversationId": sid(message.conversation_id),
        "contactInboxId": sid(message.contact_inbox_id),
        "workspaceId": sid(message.workspace_id),
        "text": message.text,
        "contentAttributes": message.content_attributes.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "messageType": message.message_type,
        "contentType": message.content_type,
        "senderType": message.sender_type,
        "senderId": opt_sid(message.sender_id),
        "sourceId": message.source_id,
        "deletedAt": opt_iso(message.deleted_at),
        "type": message.r#type,
        "parentId": message.parent_id,
        "attributes": message.attributes.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "sendError": message.send_error,
    });
    match out {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

/// `ListForConversationInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListForConversation {
    pub workspace_id: i64,
    pub conversation_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
    pub cursor: Option<PaginationCursor>,
    pub limit: i64,
}

/// `ListForConversationResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagePage {
    pub data: Vec<MessageWithAttachments>,
    pub next_cursor: Option<PaginationCursor>,
}

/// `listForConversation`: newest first; without a cursor the page starts
/// at the end of the hour of the contact inbox's `lastMessageAt` (or now).
pub async fn list_for_conversation(db: &Db, input: ListForConversation) -> Result<MessagePage> {
    tx::read(db, move |conn| {
        let conversation = match input.conversation_id {
            Some(id) => conversation_in(conn, input.workspace_id, id)?,
            None => None,
        };
        let mut last_message_at = None;
        if let Some(conversation) = &conversation {
            let contact_inbox = match input.contact_inbox_id {
                Some(id) => conn
                    .query_row(
                        r#"SELECT "lastMessageAt" FROM "ContactInbox" WHERE "contactId" = ?1 AND "id" = ?2"#,
                        params![conversation.contact_id, id],
                        |r| r.get::<_, Option<i64>>(0),
                    )
                    .optional()
                    .map_err(repo_err)?
                    .flatten(),
                None => crate::conversation::find_recent_in(
                    conn,
                    input.workspace_id,
                    conversation.contact_id,
                )?
                .and_then(|ci| ci.last_message_at),
            };
            last_message_at = contact_inbox;
        }
        // `endOfHour(lastMessageAt ?? now)`.
        let anchor = last_message_at.unwrap_or_else(now_ms);
        let end_of_hour = anchor.div_euclid(HOUR_MS) * HOUR_MS + HOUR_MS - 1;
        let result = repo_message::list_by_conversation(
            conn,
            &ListMessagesQuery {
                contact_inbox_id: input.contact_inbox_id,
                conversation_id: input.conversation_id,
                pagination: Pagination {
                    cursor: Some(input.cursor.unwrap_or(PaginationCursor {
                        created_at: end_of_hour,
                        id: None,
                        shard_id: None,
                    })),
                    limit: input.limit,
                },
                since_time: get_safe_since_time(
                    conversation.as_ref().map(|c| SinceTimeInput::Date(c.created_at)),
                    None,
                ),
                workspace_id: input.workspace_id,
            },
        )?;
        Ok(MessagePage {
            data: result.data,
            next_cursor: result.next_cursor,
        })
    })
    .await
}

/// `findForContact`: one message of the conversation (404 "Message not
/// found").
pub async fn find_for_contact(
    db: &Db,
    workspace_id: i64,
    message_id: i64,
    conversation_id: i64,
) -> Result<MessageWithAttachments> {
    tx::read(db, move |conn| {
        let missing = || BusinessError::not_found("Message not found");
        let conversation =
            conversation_in(conn, workspace_id, conversation_id)?.ok_or_else(missing)?;
        let since_time =
            get_safe_since_time(Some(SinceTimeInput::Date(conversation.created_at)), None)
                .unwrap_or(conversation.created_at);
        repo_message::find_trigger_message(
            conn,
            &FindTriggerMessageOptions {
                conversation_id,
                id: message_id,
                require_complete_results: true,
                since_time,
                workspace_id,
            },
        )?
        .filter(|m| m.message.conversation_id == conversation_id)
        .ok_or_else(missing)
    })
    .await
}

/// `CreateOutgoingInput` (the ported part).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutgoingInput {
    /// The flow branch (`{ flowId, nodeId }`).
    pub flow_id: Option<i64>,
    pub node_id: Option<String>,
    /// The products branch (`SendProductsContent`).
    pub products: Option<Value>,
    pub text: Option<String>,
    pub inbox_id: Option<i64>,
    pub client_id: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub reply_to_message_created_at: Option<i64>,
    pub is_private_reply: bool,
    pub content_attributes: Option<Value>,
    pub quick_replies: Option<Value>,
}

/// One job `createOutgoing` adds (`queue.add(name, data, opts)`).
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingJob {
    pub queue: QueueName,
    pub name: &'static str,
    pub data: Value,
    /// `{ attempts: 1 }` (manual Threads comment replies).
    pub attempts: Option<u32>,
}

/// What `createOutgoing` did: the stored message (`None` for a flow), the
/// realtime `messageCreated` payload and the jobs to add, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub message: Option<MessageWithAttachments>,
    pub realtime: Option<Value>,
    pub jobs: Vec<OutgoingJob>,
}

/// `createOutgoing`. `user_id` = the inbox user (`senderType` `user`); none
/// = the token API (`api`).
pub async fn create_outgoing(
    db: &Db,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    input: OutgoingInput,
    user_id: Option<i64>,
) -> Result<Outgoing> {
    if let Some(flow_id) = input.flow_id {
        let workspace_id = conversation.workspace_id;
        let exists = tx::read(db, move |conn| {
            Ok(flow::exists_in_workspace(conn, workspace_id, flow_id)?)
        })
        .await?;
        if !exists {
            return Err(BusinessError::not_found("Flow not found"));
        }
        let mut data = json!({
            "conversationId": conversation_json(conversation),
            "contactInboxId": contact_inbox_json(contact_inbox),
            "flowId": sid(flow_id),
            "sendFrom": "inbox",
        });
        if let Some(node) = &input.node_id {
            data["nodeId"] = json!(node);
        }
        if let Some(user) = user_id {
            data["sentByUserId"] = sid(user);
        }
        return Ok(Outgoing {
            message: None,
            realtime: None,
            jobs: vec![OutgoingJob {
                queue: QueueName::Integration,
                name: "sendFlow",
                data: json!({ "type": "sendFlow", "data": data }),
                attempts: None,
            }],
        });
    }

    if input.products.is_some() {
        if input.reply_to_message_id.is_some() || input.is_private_reply {
            return Err(BusinessError::validation(
                "products",
                "Products can't be sent as a comment reply",
            ));
        }
        // `prepareOutgoing`: only WhatsApp has a catalog source; the Meta
        // catalog resolution is not ported yet.
        if contact_inbox.channel != "whatsapp" {
            return Err(product_unresolvable("channelUnsupported"));
        }
        return Err(BusinessError::exception(
            "Product messages are not ported yet",
            "notPorted",
            501,
        ));
    }

    let is_comment = input.reply_to_message_id.is_some();
    let content_attributes = input.content_attributes.clone().or_else(|| {
        input
            .is_private_reply
            .then(|| json!({ "isPrivateReply": true }))
    });
    let message_input = CreateMessageInput {
        contact_inbox_id: contact_inbox.id,
        conversation_id: conversation.id,
        workspace_id: conversation.workspace_id,
        content_attributes,
        content_type: "text".into(),
        message_type: "outgoing".into(),
        sender_type: if user_id.is_some() { "user" } else { "api" }.into(),
        sender_id: user_id,
        parent_id: input.reply_to_message_id.clone(),
        text: input.text.clone(),
        r#type: Some(if is_comment { "comment" } else { "message" }.into()),
        created_at: Some(now_ms()),
        ..CreateMessageInput::default()
    };
    let contact_inbox_id = contact_inbox.id;
    let contact_id = contact_inbox.contact_id;
    let workspace_id = conversation.workspace_id;
    let stored = tx::write(db, move |conn| {
        let message = repo_message::create(conn, &message_input)?;
        // `contactInboxService.updateTracking` (`firstInteractionAt` keeps
        // the earliest).
        conn.execute(
            r#"UPDATE "ContactInbox"
               SET "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1
                     THEN ?1 ELSE "firstInteractionAt" END,
                   "lastMessageAt" = ?1, "updatedAt" = ?2
               WHERE "id" = ?3 AND "contactId" = ?4
                 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?5)"#,
            params![message.created_at, now_ms(), contact_inbox_id, contact_id, workspace_id],
        )
        .map_err(repo_err)?;
        Ok(MessageWithAttachments {
            message,
            attachment_count: None,
            attachments: Vec::new(),
        })
    })
    .await?;

    let mut realtime = message_json(&stored);
    if let Some(client_id) = &input.client_id {
        realtime["clientId"] = json!(client_id);
    }
    let mut job_message = realtime.clone();
    job_message["parentCreatedAt"] = opt_iso(input.reply_to_message_created_at);
    let mut send = json!({
        "conversation": conversation_json(conversation),
        "contactInbox": contact_inbox_json(contact_inbox),
        "message": job_message,
        "sendFrom": "inbox",
    });
    if let Some(quick_replies) = &input.quick_replies {
        send["quickReplies"] = quick_replies.clone();
    }
    let mut jobs = vec![OutgoingJob {
        queue: QueueName::Chat,
        name: "sendChannelMessage",
        data: json!({ "type": "sendChannelMessage", "data": send }),
        attempts: (contact_inbox.channel == "threads" && is_comment).then_some(1),
    }];
    if user_id.is_some()
        && let Some(text) = stored.message.text.as_deref().filter(|t| !t.is_empty())
    {
        jobs.push(OutgoingJob {
            queue: QueueName::Chat,
            name: "checkOutboundAutomatedResponse",
            data: json!({
                "type": "checkOutboundAutomatedResponse",
                "data": {
                    "conversation": conversation_json(conversation),
                    "contactInbox": contact_inbox_json(contact_inbox),
                    "message": { "id": sid(stored.message.id), "text": text },
                },
            }),
            attempts: None,
        });
    }
    Ok(Outgoing {
        message: Some(stored),
        realtime: Some(realtime),
        jobs,
    })
}

/// `ProductMessageUnresolvableError(reason)`.
fn product_unresolvable(reason: &str) -> BusinessError {
    BusinessError::Exception {
        message: format!("Product message cannot be sent: {reason}"),
        code: "productMessageUnresolvable".into(),
        status: 422,
        field: None,
        data: Some(json!({ "reason": reason })),
    }
}

/// `messageResourceWithRelations` / the `MessageWithPresignedAttachments`
/// JSON: every `Message` column, `attachments` (`url` / `fallbackUrl`
/// `null`) and, when set, `attachmentCount`.
pub fn message_json(message: &MessageWithAttachments) -> Value {
    let mut out = message_row_json(&message.message);
    out.insert(
        "attachments".into(),
        Value::Array(message.attachments.iter().map(attachment_json).collect()),
    );
    if let Some(count) = message.attachment_count {
        out.insert("attachmentCount".into(), json!(count));
    }
    Value::Object(out)
}
