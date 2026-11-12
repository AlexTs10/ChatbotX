//! Shared fixtures of the P3.3 slices 5+ (message, whatsapp-call, pipeline, template family,
//! order, product-category) on top of `common/repo.rs`.
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo5.rs"]
//! mod repo5;
//! ```

#![allow(dead_code)]

use chatbotx_db::repo::message::{CreateAttachmentInput, CreateMessageInput};
use chatbotx_db::repo::models::{Attachment, Message};
use chatbotx_db::repo::{self, Model};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

/// Workspace of the message fixtures (no FKs on `Message` / `Attachment`).
pub const WS: i64 = 2001;
/// Another workspace.
pub const WS_OTHER: i64 = 2002;
/// Contact inbox of the message fixtures.
pub const CI: i64 = 3101;
/// Conversation of the message fixtures.
pub const CONV: i64 = 3201;

/// `makeMessage()` of the TS test: id 1, `src-1`, incoming text from the contact at
/// `2026-01-01T00:00:00Z`.
pub fn make_message() -> CreateMessageInput {
    CreateMessageInput {
        id: Some(1),
        contact_inbox_id: CI,
        conversation_id: CONV,
        workspace_id: WS,
        message_type: "incoming".to_owned(),
        content_type: "text".to_owned(),
        sender_type: "contact".to_owned(),
        source_id: Some("src-1".to_owned()),
        created_at: Some(crate::repo_common::ms("2026-01-01T00:00:00Z")),
        ..CreateMessageInput::default()
    }
}

/// An image attachment input of `message_id` (`/uploads/img.png`).
pub fn make_attachment(
    id: Option<i64>,
    message_id: i64,
    message_created_at: i64,
) -> CreateAttachmentInput {
    CreateAttachmentInput {
        id,
        workspace_id: WS,
        conversation_id: CONV,
        message_id,
        message_created_at,
        file_type: "image".to_owned(),
        mime_type: "image/png".to_owned(),
        origin_path: "/uploads/img.png".to_owned(),
        ..CreateAttachmentInput::default()
    }
}

/// A raw `Message` row.
#[derive(Debug, Clone)]
pub struct MsgRow {
    pub id: i64,
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub contact_inbox_id: i64,
    pub source_id: Option<&'static str>,
    pub created_at: i64,
    pub message_type: &'static str,
    pub text: Option<&'static str>,
    pub content_attributes: Option<&'static str>,
    pub parent_id: Option<String>,
}

impl MsgRow {
    /// An incoming row of `WS` / `CONV` / `CI` without source id or text.
    pub fn new(id: i64, created_at: i64) -> MsgRow {
        MsgRow {
            id,
            workspace_id: WS,
            conversation_id: CONV,
            contact_inbox_id: CI,
            source_id: None,
            created_at,
            message_type: "incoming",
            text: None,
            content_attributes: None,
            parent_id: None,
        }
    }
}

/// Inserts one raw `Message` row.
pub fn put_message(tx: &Connection, row: &MsgRow) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "workspaceId", "conversationId",
               "contactInboxId", "sourceId", "messageType", "contentType", "senderType", "text",
               "contentAttributes", "parentId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'text', 'contact', ?8, ?9, ?10)"#,
        params![
            row.id,
            row.created_at,
            row.workspace_id,
            row.conversation_id,
            row.contact_inbox_id,
            row.source_id,
            row.message_type,
            row.text,
            row.content_attributes,
            row.parent_id,
        ],
    )?)
}

/// Inserts one raw `Attachment` row of `WS` / `CONV` (`createdAt` = `message_created_at`).
pub fn put_attachment(
    tx: &Connection,
    id: i64,
    message_id: i64,
    message_created_at: i64,
    origin_path: &str,
    thumbnail_path: Option<&str>,
) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "Attachment" ("id", "createdAt", "workspaceId", "conversationId", "messageId",
               "messageCreatedAt", "fileType", "mimeType", "originPath", "thumbnailPath", "sourceId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?2, 'image', 'image/png', ?6, ?7, ?8)"#,
        params![
            id,
            message_created_at,
            WS,
            CONV,
            message_id,
            origin_path,
            thumbnail_path,
            format!("source-attachment-{id}")
        ],
    )?)
}

/// The `Message` row `id`, if any.
pub async fn message_row(db: &Db, id: i64) -> Option<Message> {
    repo::read(db, move |c| {
        repo::find_one::<Message>(c, "\"id\" = ?1", &[&id])
    })
    .await
    .expect("read message")
}

/// Every `Message` row, by id.
pub async fn message_rows(db: &Db) -> Vec<Message> {
    repo::read(db, |c| {
        repo::find_many::<Message>(c, "ORDER BY \"id\"", &[])
    })
    .await
    .expect("read messages")
}

/// The `Attachment` row `id`, if any.
pub async fn attachment_row(db: &Db, id: i64) -> Option<Attachment> {
    repo::read(db, move |c| {
        repo::find_one::<Attachment>(c, "\"id\" = ?1", &[&id])
    })
    .await
    .expect("read attachment")
}

/// Every `Attachment` row, by id.
pub async fn attachment_rows(db: &Db) -> Vec<Attachment> {
    repo::read(db, |c| {
        repo::find_many::<Attachment>(c, "ORDER BY \"id\"", &[])
    })
    .await
    .expect("read attachments")
}

/// `Message::TABLE` (keeps the `Model` import used).
pub fn message_table() -> &'static str {
    Message::TABLE
}
