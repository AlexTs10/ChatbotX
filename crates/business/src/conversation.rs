//! `conversation/service.ts` (`conversationService`): the parts the public
//! contact message routes use (`findByContactWithInboxes`,
//! `resolveContactInboxForSend`) and `contactInboxService.findRecentByContactId`.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{Model, contact_inbox};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

pub(crate) fn find_with_inboxes_in(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<ConversationWithInboxes>> {
    let conversation = conn
        .query_row(
            &format!(
                r#"{} WHERE "contactId" = ?1 AND "workspaceId" = ?2
                   ORDER BY "sourceId" IS NOT NULL, "lastActivityAt" IS NULL, "lastActivityAt" DESC, "id" DESC
                   LIMIT 1"#,
                Conversation::select_sql()
            ),
            params![contact_id, workspace_id],
            Conversation::from_row,
        )
        .optional()
        .map_err(repo_err)?;
    let Some(conversation) = conversation else {
        return Ok(None);
    };
    let contact_inboxes = contact_inbox::list_by_contact_id(conn, contact_id)?;
    Ok(Some(ConversationWithInboxes {
        conversation,
        contact_inboxes,
    }))
}

pub(crate) fn find_recent_in(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<ContactInbox>> {
    conn.query_row(
        &format!(
            r#"SELECT {} FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
               WHERE ci."contactId" = ?1 AND i."workspaceId" = ?2
               ORDER BY coalesce(ci."lastMessageAt", 0) DESC, ci."id" ASC LIMIT 1"#,
            ContactInbox::columns_sql(Some("ci"))
        ),
        params![contact_id, workspace_id],
        ContactInbox::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// `ConversationWithContactInboxes`: the conversation and every contact
/// inbox of its contact (the relation is keyed by `contactId`).
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationWithInboxes {
    pub conversation: Conversation,
    pub contact_inboxes: Vec<ContactInbox>,
}

/// `findByContactWithInboxes`: the contact's conversation, DM threads
/// (`sourceId` null) first, then `lastActivityAt` desc (nulls last), then
/// `id` desc.
pub async fn find_by_contact_with_inboxes(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<ConversationWithInboxes>> {
    tx::read(db, move |conn| {
        find_with_inboxes_in(conn, workspace_id, contact_id)
    })
    .await
}

/// `contactInboxService.findRecentByContactId`: the contact inbox (of a
/// workspace inbox) with the latest `lastMessageAt` (none sorts as 0).
pub async fn find_recent_contact_inbox(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<ContactInbox>> {
    tx::read(db, move |conn| {
        find_recent_in(conn, workspace_id, contact_id)
    })
    .await
}

/// `resolveContactInboxForSend`: the conversation and the contact inbox to
/// send through (the `inbox_id` row of the relation, else the most recent
/// one). 404 "Conversation not found" when either is missing.
pub async fn resolve_contact_inbox_for_send(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    inbox_id: Option<i64>,
) -> Result<(ConversationWithInboxes, ContactInbox)> {
    tx::read(db, move |conn| {
        let missing = || BusinessError::not_found("Conversation not found");
        let found = find_with_inboxes_in(conn, workspace_id, contact_id)?.ok_or_else(missing)?;
        let contact_inbox = match inbox_id {
            Some(inbox) => found
                .contact_inboxes
                .iter()
                .find(|ci| ci.inbox_id == inbox)
                .cloned(),
            None => find_recent_in(conn, workspace_id, contact_id)?,
        }
        .ok_or_else(missing)?;
        Ok((found, contact_inbox))
    })
    .await
}
