//! The non-message inbound webhook jobs the Meta channels queue
//! (`handlers/received-message.ts` `updateIncomingComment`,
//! `deleteIncomingComment`, `deleteIncomingMessage`,
//! `processMessageReaction`; `handlers/conversation.ts` `contactMarkAsRead`).

use chatbotx_business::message::message_json;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::message::{
    self as repo_message, CreateMessageInput, MessageWithAttachments,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use chatbotx_rt::EventType;
use chatbotx_rt::protocol::{Envelope, route_for_conversation};
use serde_json::{Value, json};

use crate::channel::InboundInbox;
use crate::{Deps, db_fail, str_of};

pub const CONTACT_MARK_AS_READ: &str = "contactMarkAsRead";
pub const MESSAGE_REACTION: &str = "messageReaction";
pub const DELETE_INCOMING_MESSAGE: &str = "deleteIncomingMessage";
pub const UPDATE_INCOMING_COMMENT: &str = "updateIncomingComment";
pub const DELETE_INCOMING_COMMENT: &str = "deleteIncomingComment";

/// `identifyInboxAndIntegrationAuthFromIdentifier` (inbox only).
async fn inbox_of(deps: &Deps, data: &Value) -> Result<InboundInbox, JobError> {
    let kind = str_of(data, "integrationType").unwrap_or("");
    let identifier = str_of(data, "integrationIdentifier").unwrap_or("");
    deps.channels
        .resolve_by_identifier(&deps.db, deps.realtime.as_ref(), kind, identifier)
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
        .map(|(_, _, inbox)| inbox)
        .ok_or_else(|| {
            JobError::unrecoverable(format!("Integration not found: {kind} {identifier}"))
        })
}

async fn conversation_by_id(
    deps: &Deps,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Conversation>, JobError> {
    deps.db
        .read(move |c| {
            Ok(c.query_row(
                &format!(
                    r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    Conversation::select_sql()
                ),
                params![id, workspace_id],
                Conversation::from_row,
            )
            .optional()?)
        })
        .await
        .map_err(db_fail)
}

/// `queueWorkspaceRealtimeEvent(..., route: routeForConversation(conversation))`.
async fn publish(
    deps: &Deps,
    inbox: &InboundInbox,
    conversation_id: i64,
    event_type: EventType,
    data: Value,
) {
    let Some(realtime) = &deps.realtime else {
        return;
    };
    let conversation = conversation_by_id(deps, inbox.workspace_id, conversation_id)
        .await
        .ok()
        .flatten();
    let route = match &conversation {
        Some(c) => crate::chat::route_of(c, inbox.inbox_id),
        None => {
            tracing::warn!(
                conversation_id,
                workspace_id = inbox.workspace_id,
                "conversation not found for realtime route"
            );
            route_for_conversation(None, None)
        }
    };
    realtime
        .queue_workspace_event(
            &inbox.workspace_id.to_string(),
            Envelope::routed(event_type, data, route),
        )
        .await;
}

/// `updateIncomingComment`.
pub async fn update_incoming_comment(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let inbox = inbox_of(deps, data).await?;
    let comment_id = str_of(data, "commentId").unwrap_or("").to_owned();
    let new_text = str_of(data, "newText").unwrap_or("").to_owned();
    let workspace_id = inbox.workspace_id;
    let (id, text) = (comment_id.clone(), new_text.clone());
    let updated = deps
        .db
        .write(move |c| {
            repo_message::update_text_by_source_id(c, &id, workspace_id, &text)
                .map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    let Some(updated) = updated else {
        tracing::warn!(comment_id, "updateIncomingComment: comment not found");
        return Ok(());
    };
    let event = json!({ "messageId": updated.id.to_string(), "newText": new_text, "removedAttachment": false });
    publish(
        deps,
        &inbox,
        updated.conversation_id,
        EventType::MessageUpdated,
        event,
    )
    .await;
    Ok(())
}

/// `deleteIncomingComment` / `deleteIncomingMessage`: soft-deletes the row
/// (and its children) keyed by the provider id.
pub async fn delete_incoming(deps: &Deps, data: &Value, key: &str) -> Result<(), JobError> {
    let inbox = inbox_of(deps, data).await?;
    let source_id = str_of(data, key).unwrap_or("").to_owned();
    let workspace_id = inbox.workspace_id;
    let id = source_id.clone();
    let deleted = deps
        .db
        .write(move |c| {
            repo_message::delete_by_source_id(c, &id, workspace_id, now_ms())
                .map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    let Some(first) = deleted.first() else {
        tracing::warn!(source_id, "{key}: message not found");
        return Ok(());
    };
    let ids: Vec<String> = deleted.iter().map(|r| r.id.to_string()).collect();
    publish(
        deps,
        &inbox,
        first.conversation_id,
        EventType::MessageDeleted,
        json!({ "messageIds": ids }),
    )
    .await;
    Ok(())
}

/// `processMessageReaction`: an `activity` message in the DM timeline; an
/// unknown contact is skipped (no contact is created).
pub async fn message_reaction(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let inbox = inbox_of(deps, data).await?;
    let contact_source_id = str_of(data, "contactSourceId").unwrap_or("").to_owned();
    let message_id = str_of(data, "messageId").unwrap_or("").to_owned();
    let action = str_of(data, "action").unwrap_or("").to_owned();
    let emoji = str_of(data, "emoji")
        .filter(|e| !e.is_empty())
        .map(str::to_owned);
    let reaction_source_id = format!("{message_id}-reaction-{action}");
    let reaction_text = if action == "react" {
        format!(
            "Reacted{}",
            emoji.map(|e| format!(" {e}")).unwrap_or_default()
        )
    } else {
        "Removed a reaction".to_owned()
    };
    let probe = inbox.clone();
    let (source, text) = (reaction_source_id.clone(), reaction_text.clone());
    let outcome = deps
        .db
        .write(move |tx| {
            let Some(contact_inbox) = tx
                .query_row(
                    &format!(
                        r#"{} WHERE "inboxId" = ?1 AND "channel" = ?2 AND "sourceId" = ?3
                           AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = "ContactInbox"."contactId") LIMIT 1"#,
                        ContactInbox::select_sql()
                    ),
                    params![probe.inbox_id, probe.channel, contact_source_id],
                    ContactInbox::from_row,
                )
                .optional()?
            else {
                return Ok(None);
            };
            let conversation =
                crate::inbound::find_or_create_conversation(tx, probe.workspace_id, contact_inbox.contact_id, None, None)?;
            let created = repo_message::create_or_update(
                tx,
                &CreateMessageInput {
                    id: None,
                    contact_inbox_id: contact_inbox.id,
                    conversation_id: conversation.id,
                    workspace_id: probe.workspace_id,
                    content_attributes: None,
                    content_type: "text".into(),
                    message_type: "activity".into(),
                    sender_type: "contact".into(),
                    sender_id: Some(contact_inbox.contact_id),
                    source_id: Some(source),
                    parent_id: None,
                    text: Some(text.clone()),
                    r#type: None,
                    created_at: Some(now_ms()),
                    updated_at: None,
                },
            )
            .map_err(crate::repo_err)?;
            let changed = if !created.is_new && created.message.text.as_deref() != Some(text.as_str()) {
                repo_message::update_message_text(
                    tx,
                    created.message.id,
                    probe.workspace_id,
                    &text,
                    created.message.created_at,
                )
                .map_err(crate::repo_err)?
                .map(|r| r.id)
            } else {
                None
            };
            Ok(Some((conversation, created, changed)))
        })
        .await
        .map_err(db_fail)?;
    let Some((conversation, created, changed)) = outcome else {
        tracing::warn!(
            message_id,
            "processMessageReaction: contact not found — skipping"
        );
        return Ok(());
    };
    if created.is_new {
        let created_at = created.message.created_at;
        let row = message_json(&MessageWithAttachments {
            message: created.message,
            attachment_count: None,
            attachments: Vec::new(),
        });
        crate::chat::queue_event(
            deps,
            &conversation,
            inbox.inbox_id,
            EventType::MessageCreated,
            row,
        )
        .await;
        crate::leaderboard::award_reaction(
            deps,
            &inbox,
            conversation.contact_id,
            &reaction_source_id,
            &action,
            created_at,
        )
        .await;
        return Ok(());
    }
    if let Some(id) = changed {
        let event = json!({ "messageId": id.to_string(), "newText": reaction_text, "removedAttachment": false });
        crate::chat::queue_event(
            deps,
            &conversation,
            inbox.inbox_id,
            EventType::MessageUpdated,
            event,
        )
        .await;
    }
    Ok(())
}

/// `normalizeEpochTimestamp`: seconds below 1e10, else milliseconds.
fn normalize_epoch(value: &Value) -> Option<i64> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    Some(if n < 10_000_000_000.0 { n * 1000.0 } else { n } as i64)
}

/// `parseReadTimestamp`: `entry[0].messaging[0].timestamp ?? timestamp`.
fn read_timestamp(payload: &Value) -> Option<i64> {
    let ts = payload
        .pointer("/entry/0/messaging/0/timestamp")
        .or_else(|| payload.get("timestamp"))?;
    normalize_epoch(ts)
}

/// `contactMarkAsRead`: advances the DM conversation's and the contact
/// inbox's `contactLastReadAt`, then emits `message:seen`.
pub async fn contact_mark_as_read(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let inbox = inbox_of(deps, data).await?;
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let source = str_of(data, "sourceConversationId")
        .unwrap_or("")
        .to_owned();
    let seen_at = data
        .get("payload")
        .and_then(read_timestamp)
        .unwrap_or_else(now_ms);
    let (probe, channel) = (inbox.clone(), integration_type.clone());
    let marked = deps
        .db
        .write(move |tx| {
            let Some(ci) = tx
                .query_row(
                    &format!(
                        r#"{} WHERE "sourceId" = ?1 AND "channel" = ?2 AND "inboxId" = ?3 LIMIT 1"#,
                        ContactInbox::select_sql()
                    ),
                    params![source, channel, probe.inbox_id],
                    ContactInbox::from_row,
                )
                .optional()?
            else {
                return Ok(Err("contactMarkAsRead: no contact inbox for this source id, skipping"));
            };
            let conversation: Option<i64> = tx
                .query_row(
                    r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL LIMIT 1"#,
                    params![probe.workspace_id, ci.contact_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(conversation_id) = conversation else {
                return Ok(Err("contactMarkAsRead: no DM conversation for contact inbox, skipping"));
            };
            // Advance-only (`GREATEST`): receipts are processed concurrently.
            tx.execute(
                r#"UPDATE "Conversation" SET "contactLastReadAt" = MAX(COALESCE("contactLastReadAt", ?1), ?1)
                   WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                params![seen_at, conversation_id, probe.workspace_id],
            )?;
            tx.execute(
                r#"UPDATE "ContactInbox" SET "contactLastReadAt" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "contactId" = ?4"#,
                params![seen_at, now_ms(), ci.id, ci.contact_id],
            )?;
            Ok(Ok((ci, conversation_id)))
        })
        .await
        .map_err(db_fail)?;
    let (ci, conversation_id) = match marked {
        Ok(found) => found,
        Err(reason) => {
            tracing::warn!(integration_type, inbox_id = inbox.inbox_id, "{reason}");
            return Ok(());
        }
    };
    let Some(buses) = deps.events.as_ref() else {
        return Ok(());
    };
    let payload = json!({
        "context": {
            "workspaceId": inbox.workspace_id.to_string(),
            "contactId": ci.contact_id.to_string(),
            "conversationId": conversation_id.to_string(),
            "contactInboxId": ci.id.to_string(),
            "channel": integration_type,
        },
        "action": {},
        "occurredAt": iso(seen_at),
    });
    if let Some(future) = chatbotx_jobs::events::emit(buses, "message:seen", payload)
        && let Err(err) = future.await
    {
        tracing::warn!(err = %err, "message:seen emit failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_timestamp_takes_the_first_event_in_seconds_or_ms() {
        let payload = json!({ "entry": [{ "messaging": [{ "timestamp": 1_700_000_000 }] }] });
        assert_eq!(read_timestamp(&payload), Some(1_700_000_000_000));
        assert_eq!(
            read_timestamp(&json!({ "timestamp": 1_700_000_000_123_i64 })),
            Some(1_700_000_000_123)
        );
        assert_eq!(read_timestamp(&json!({ "timestamp": 0 })), None);
    }
}
