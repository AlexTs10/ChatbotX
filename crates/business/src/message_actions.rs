//! The message actions of the public `/v1/conversations/{id}/messages/*`
//! routes: `findByIdWithUrls`, `resolveContactInboxForConversation`,
//! `deleteMessage`, `changeMessageAttributes` and `editMessage` (text and
//! `removeAttachment` only). The caller publishes the returned envelope and
//! adds the returned jobs.

use chatbotx_db::Db;
use chatbotx_db::repo::message::{
    self as repo_message, FindMessageByIdParams, MessageWithAttachments,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{Model, iso};
use chatbotx_db::rusqlite::Connection;
use chatbotx_jobs::QueueName;
use chatbotx_rt::protocol::{ConversationRoute, route_for_conversation};
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Value, json};

use crate::conversation_actions::ActionJob;
use crate::errors::{BusinessError, Result};
use crate::message::{contact_inbox_json, conversation_json};
use crate::tx;

/// `ChatbotXException(message)`: 400 `systemError`.
fn system_error(message: &str) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

fn route(conversation: &Conversation) -> chatbotx_rt::protocol::Route {
    route_for_conversation(
        Some(&ConversationRoute {
            assigned_inbox_team_id: conversation.assigned_inbox_team_id.map(|i| i.to_string()),
            assigned_user_id: conversation.assigned_user_id.map(|i| i.to_string()),
            inbox_id: None,
        }),
        None,
    )
}

fn find_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    created_at: i64,
) -> Result<Option<MessageWithAttachments>> {
    Ok(repo_message::find_by_id(
        conn,
        &FindMessageByIdParams {
            conversation_id: None,
            created_at,
            id,
            workspace_id,
        },
    )?)
}

/// `contactInboxService.findByIdForWorkspace` (`findBy({ where: { id } })`
/// for `changeAttributes`: same row, the message is already workspace
/// scoped).
fn contact_inbox_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<ContactInbox>> {
    let mut q = Sql::raw(format!(
        r#"SELECT {} FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId" WHERE ci."id" = "#,
        ContactInbox::columns_sql(Some("ci"))
    ));
    q.push_bind(id);
    q.push(r#" AND i."workspaceId" = "#);
    q.push_bind(workspace_id);
    q.push(" LIMIT 1");
    Ok(q.model::<ContactInbox>(conn)?)
}

/// `findByIdWithUrls` (404 "Message not found"); the attachment URLs stay
/// `null` (no presigner yet).
pub async fn find_by_id_with_urls(
    db: &Db,
    workspace_id: i64,
    id: i64,
    created_at: i64,
) -> Result<MessageWithAttachments> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id, created_at))
        .await?
        .ok_or_else(|| BusinessError::not_found("Message not found"))
}

/// `resolveContactInboxForConversation`: the `inbox_id` contact inbox of
/// the conversation's contact, else the most recent one (404 "Inbox not
/// found").
pub async fn resolve_contact_inbox_for_conversation(
    db: &Db,
    conversation: &Conversation,
    inbox_id: Option<i64>,
) -> Result<ContactInbox> {
    let (workspace_id, contact_id) = (conversation.workspace_id, conversation.contact_id);
    tx::read(db, move |conn| match inbox_id {
        Some(inbox) => {
            let mut q = Sql::raw(format!(
                r#"SELECT {} FROM "ContactInbox" WHERE "contactId" = "#,
                ContactInbox::columns_sql(None)
            ));
            q.push_bind(contact_id);
            q.push(r#" AND "inboxId" = "#);
            q.push_bind(inbox);
            q.push(" LIMIT 1");
            Ok(q.model::<ContactInbox>(conn)?)
        }
        None => crate::conversation::find_recent_in(conn, workspace_id, contact_id),
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Inbox not found"))
}

/// What a message action did: the realtime event and the chat job.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub event: Option<Envelope>,
    pub job: Option<ActionJob>,
}

fn chat_job(name: &'static str, data: Value) -> ActionJob {
    ActionJob {
        queue: QueueName::Chat,
        name,
        data: json!({ "type": name, "data": data }),
        job_id: None,
    }
}

fn message_ref(message: &MessageWithAttachments) -> Value {
    json!({
        "id": message.message.id.to_string(),
        "createdAt": iso(message.message.created_at),
    })
}

/// `deleteMessage`: soft-deletes the message (with its children when it
/// has a `sourceId`), emits `messageDeleted` and, for a channel message,
/// queues `deleteChannelMessage`. Unknown: 400 "Comment not found".
pub async fn delete_message(
    db: &Db,
    conversation: &Conversation,
    id: i64,
    created_at: i64,
) -> Result<Done> {
    let conversation_id = conversation.id;
    let workspace_id = conversation.workspace_id;
    let (message_ids, message, contact_inbox) = tx::write(db, move |conn| {
        let message = find_in(conn, workspace_id, id, created_at)?
            .filter(|m| m.message.conversation_id == conversation_id)
            .ok_or_else(|| system_error("Comment not found"))?;
        let row = &message.message;
        let ids: Vec<i64> = match &row.source_id {
            Some(source) => {
                repo_message::delete_by_source_id(conn, source, workspace_id, row.created_at)?
                    .into_iter()
                    .map(|r| r.id)
                    .collect()
            }
            None => repo_message::delete_by_id(conn, row.id, workspace_id, row.created_at)?,
        };
        let contact_inbox = match row.source_id {
            Some(_) => contact_inbox_in(conn, workspace_id, row.contact_inbox_id)?,
            None => None,
        };
        Ok((ids, message, contact_inbox))
    })
    .await?;
    let ids: Vec<String> = message_ids.iter().map(i64::to_string).collect();
    Ok(Done {
        event: Some(Envelope::routed(
            EventType::MessageDeleted,
            json!({ "messageIds": ids }),
            route(conversation),
        )),
        job: contact_inbox.map(|ci| {
            chat_job(
                "deleteChannelMessage",
                json!({
                    "conversation": conversation_json(conversation),
                    "contactInbox": contact_inbox_json(&ci),
                    "message": message_ref(&message),
                }),
            )
        }),
    })
}

/// `changeMessageAttributes`: queues `changeChannelMessageState` (the
/// worker skips non-comments). Unknown: 400 "Message not found".
pub async fn change_attributes(
    db: &Db,
    conversation: &Conversation,
    id: i64,
    created_at: i64,
    liked: Option<bool>,
    hidden: Option<bool>,
) -> Result<ActionJob> {
    let conversation_id = conversation.id;
    let workspace_id = conversation.workspace_id;
    let (message, contact_inbox) = tx::read(db, move |conn| {
        let message = find_in(conn, workspace_id, id, created_at)?
            .filter(|m| m.message.conversation_id == conversation_id)
            .ok_or_else(|| system_error("Message not found"))?;
        let contact_inbox = contact_inbox_in(conn, workspace_id, message.message.contact_inbox_id)?
            .ok_or_else(|| system_error("Inbox not found"))?;
        Ok((message, contact_inbox))
    })
    .await?;
    let mut data = json!({
        "conversation": conversation_json(conversation),
        "contactInbox": contact_inbox_json(&contact_inbox),
        "message": message_ref(&message),
    });
    if let Some(liked) = liked {
        data["liked"] = json!(liked);
    }
    if let Some(hidden) = hidden {
        data["hidden"] = json!(hidden);
    }
    Ok(chat_job("changeChannelMessageState", data))
}

/// `editMessage` without a new attachment: updates the text (and drops the
/// attachments on `remove_attachment`), emits `messageUpdated` and queues
/// `editChannelMessage`. Returns the response body.
pub async fn edit_message(
    db: &Db,
    conversation: &Conversation,
    id: i64,
    created_at: i64,
    new_text: String,
    remove_attachment: bool,
) -> Result<(Value, Done)> {
    let conversation_id = conversation.id;
    let workspace_id = conversation.workspace_id;
    let text = new_text.clone();
    let (message, contact_inbox) = tx::write(db, move |conn| {
        let message = find_in(conn, workspace_id, id, created_at)?
            .filter(|m| m.message.conversation_id == conversation_id)
            .ok_or_else(|| system_error("Comment not found"))?;
        let row = &message.message;
        if row.message_type != "outgoing" || row.r#type != "comment" {
            return Err(system_error("Message is not an editable comment"));
        }
        let contact_inbox = contact_inbox_in(conn, workspace_id, row.contact_inbox_id)?
            .ok_or_else(|| system_error("Inbox not found"))?;
        repo_message::update_message_text(conn, row.id, workspace_id, &text, row.created_at)?
            .ok_or_else(|| system_error("Message not found"))?;
        if remove_attachment {
            repo_message::delete_attachments_by_message_id(
                conn,
                row.id,
                workspace_id,
                row.created_at,
            )?;
        }
        Ok((message, contact_inbox))
    })
    .await?;
    let body = json!({
        "success": true,
        "messageId": id.to_string(),
        "newText": new_text,
        "newAttachmentPath": null,
        "newAttachmentPublicUrl": null,
        "newAttachmentMimeType": null,
        "newAttachmentWidth": 0,
        "newAttachmentHeight": 0,
        "removedAttachment": remove_attachment,
    });
    let mut event_data = body.clone();
    if let Some(map) = event_data.as_object_mut() {
        map.remove("success");
    }
    let done = Done {
        event: Some(Envelope::routed(
            EventType::MessageUpdated,
            event_data,
            route(conversation),
        )),
        job: Some(chat_job(
            "editChannelMessage",
            json!({
                "conversation": conversation_json(conversation),
                "contactInbox": contact_inbox_json(&contact_inbox),
                "message": message_ref(&message),
                "newText": new_text,
            }),
        )),
    };
    Ok((body, done))
}

/// `getWhatsappFlowResponse(contentAttributes)` matches.
pub fn is_whatsapp_flow_response(content_attributes: Option<&Value>) -> bool {
    let Some(attrs) = content_attributes.and_then(Value::as_object) else {
        return false;
    };
    attrs.get("type").and_then(Value::as_str) == Some("whatsapp_flow_response")
        && attrs.get("name").is_none_or(|n| n.as_str() == Some("flow"))
        && attrs.get("flowResponse").is_some_and(Value::is_object)
}
