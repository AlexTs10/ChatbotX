//! Port of `repositories/pipeline/item-content.ts` (`pipelineItemContentRepository`): an
//! item's comments and attachments.

use rusqlite::Connection;

use super::models::{MediaLibraryFile, PipelineItemAttachment, PipelineItemComment};
use super::pipeline_item_cursor::NewestFirstKeyset;
use super::pipeline_item_cursor::bind1;
use super::sql::Sql;
use super::{Error, Model, Result, find_many, find_one, id_list, new_id, returning};

/// `PipelineCommentInsert`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineCommentInsert {
    pub workspace_id: i64,
    pub item_id: i64,
    pub text: String,
    pub created_by_id: Option<i64>,
    pub idempotency_key: Option<String>,
}

/// Result of `insertCommentIdempotent`.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertedComment {
    pub row: PipelineItemComment,
    pub inserted: bool,
}

/// `PipelineItemCommentPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineItemCommentPage {
    pub items: Vec<PipelineItemComment>,
    pub next_cursor: Option<String>,
}

/// One row of `countCommentsForItems`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCommentCount {
    pub item_id: i64,
    pub count: i64,
}

/// One row of `listAttachments` (`with: { mediaLibraryFile: true }`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentWithFile {
    pub attachment: PipelineItemAttachment,
    pub media_library_file: Option<MediaLibraryFile>,
}

/// `listComments`: newest-first keyset page of an item's comments.
pub fn list_comments(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    limit: i64,
    cursor: Option<&str>,
) -> Result<PipelineItemCommentPage> {
    todo!("P3.3")
}

/// `countCommentsForItems`.
pub fn count_comments_for_items(
    conn: &Connection,
    workspace_id: i64,
    item_ids: &[i64],
) -> Result<Vec<ItemCommentCount>> {
    todo!("P3.3")
}

/// `insertCommentIdempotent`: inserts the comment, or returns the one an earlier call with
/// the same idempotency key stored.
pub fn insert_comment_idempotent(conn: &Connection, values: &PipelineCommentInsert) -> Result<InsertedComment> {
    todo!("P3.3")
}

/// `deleteComment`.
pub fn delete_comment(conn: &Connection, workspace_id: i64, item_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `latestCommentAt`: the latest remaining comment time, or `None`.
pub fn latest_comment_at(conn: &Connection, workspace_id: i64, item_id: i64) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `listAttachments`: oldest first, each with its media library file.
pub fn list_attachments(conn: &Connection, workspace_id: i64, item_id: i64) -> Result<Vec<AttachmentWithFile>> {
    todo!("P3.3")
}

/// `countAttachments`.
pub fn count_attachments(conn: &Connection, workspace_id: i64, item_id: i64) -> Result<i64> {
    todo!("P3.3")
}

/// `insertAttachments`: `ON CONFLICT DO NOTHING`; returns how many were inserted.
pub fn insert_attachments(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    media_library_file_ids: &[i64],
) -> Result<usize> {
    todo!("P3.3")
}

/// `deleteAttachment`.
pub fn delete_attachment(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    media_library_file_id: i64,
) -> Result<bool> {
    todo!("P3.3")
}
