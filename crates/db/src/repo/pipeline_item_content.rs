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
    let keyset = NewestFirstKeyset::new(limit, cursor)?;
    let ts = "\"PipelineItemComment\".\"createdAt\"";
    let id = "\"PipelineItemComment\".\"id\"";
    let mut sql = Sql::raw(format!("{} WHERE ", PipelineItemComment::select_sql()));
    sql.append(Sql::and([
        Some(bind1(
            "\"PipelineItemComment\".\"workspaceId\" = ?",
            workspace_id,
        )),
        Some(bind1("\"PipelineItemComment\".\"itemId\" = ?", item_id)),
        keyset.after(ts, id),
    ]));
    sql.push(&format!(
        " ORDER BY {} LIMIT ",
        NewestFirstKeyset::order_by(ts, id)
    ));
    sql.push_bind(keyset.fetch_limit());
    let rows = sql.models::<PipelineItemComment>(conn)?;
    let (items, next_cursor) = keyset.page(rows, |r| (r.created_at, r.id));
    Ok(PipelineItemCommentPage { items, next_cursor })
}

/// `countCommentsForItems`.
pub fn count_comments_for_items(
    conn: &Connection,
    workspace_id: i64,
    item_ids: &[i64],
) -> Result<Vec<ItemCommentCount>> {
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"itemId\", count(*) FROM \"PipelineItemComment\" WHERE \"workspaceId\" = ? AND \"itemId\" IN (SELECT value FROM json_each(?)) GROUP BY \"itemId\"",
        vec![workspace_id.into(), id_list(item_ids).into()],
    )
    .query_map(conn, |row| {
        Ok(ItemCommentCount {
            item_id: row.get(0)?,
            count: row.get(1)?,
        })
    })
}

/// `insertCommentIdempotent`: inserts the comment, or returns the one an earlier call with
/// the same idempotency key stored.
pub fn insert_comment_idempotent(
    conn: &Connection,
    values: &PipelineCommentInsert,
) -> Result<InsertedComment> {
    let conflict = if values.idempotency_key.is_some() {
        " ON CONFLICT (\"workspaceId\", \"itemId\", \"idempotencyKey\") WHERE \"idempotencyKey\" IS NOT NULL DO NOTHING"
    } else {
        ""
    };
    let sql = format!(
        "INSERT INTO \"PipelineItemComment\" (\"id\", \"workspaceId\", \"itemId\", \"text\", \"createdById\", \"idempotencyKey\") VALUES (?1, ?2, ?3, ?4, ?5, ?6){conflict}"
    );
    let row = returning::<PipelineItemComment>(
        conn,
        &sql,
        &[
            &new_id(),
            &values.workspace_id,
            &values.item_id,
            &values.text,
            &values.created_by_id,
            &values.idempotency_key,
        ],
    )?
    .into_iter()
    .next();
    if let Some(row) = row {
        return Ok(InsertedComment {
            row,
            inserted: true,
        });
    }
    let Some(key) = values.idempotency_key.as_deref() else {
        return Err(Error::msg("Failed to insert pipeline comment"));
    };
    let existing: PipelineItemComment = find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"itemId\" = ?2 AND \"idempotencyKey\" = ?3",
        &[&values.workspace_id, &values.item_id, &key],
    )?
    .ok_or_else(|| Error::msg("Pipeline comment idempotency conflict without a row"))?;
    Ok(InsertedComment {
        row: existing,
        inserted: false,
    })
}

/// `deleteComment`.
pub fn delete_comment(conn: &Connection, workspace_id: i64, item_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineItemComment\" WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2 AND \"id\" = ?3",
        (workspace_id, item_id, id),
    )? > 0)
}

/// `latestCommentAt`: the latest remaining comment time, or `None`.
pub fn latest_comment_at(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT max(\"createdAt\") FROM \"PipelineItemComment\" WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2",
        (workspace_id, item_id),
        |row| row.get(0),
    )?)
}

/// `listAttachments`: oldest first, each with its media library file.
pub fn list_attachments(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
) -> Result<Vec<AttachmentWithFile>> {
    let attachments: Vec<PipelineItemAttachment> = find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2 ORDER BY \"createdAt\" ASC",
        &[&workspace_id, &item_id],
    )?;
    let mut out = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let file: Option<MediaLibraryFile> =
            find_one(conn, "\"id\" = ?1", &[&attachment.media_library_file_id])?;
        out.push(AttachmentWithFile {
            attachment,
            media_library_file: file,
        });
    }
    Ok(out)
}

/// `countAttachments`.
pub fn count_attachments(conn: &Connection, workspace_id: i64, item_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"PipelineItemAttachment\" WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2",
        (workspace_id, item_id),
        |row| row.get(0),
    )?)
}

/// `insertAttachments`: `ON CONFLICT DO NOTHING`; returns how many were inserted.
pub fn insert_attachments(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    media_library_file_ids: &[i64],
) -> Result<usize> {
    let mut inserted = 0;
    for file_id in media_library_file_ids {
        inserted += conn.execute(
            "INSERT INTO \"PipelineItemAttachment\" (\"id\", \"workspaceId\", \"itemId\", \"mediaLibraryFileId\") VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            (new_id(), workspace_id, item_id, file_id),
        )?;
    }
    Ok(inserted)
}

/// `deleteAttachment`.
pub fn delete_attachment(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    media_library_file_id: i64,
) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM \"PipelineItemAttachment\" WHERE \"workspaceId\" = ?1 AND \"itemId\" = ?2 AND \"mediaLibraryFileId\" = ?3",
        (workspace_id, item_id, media_library_file_id),
    )? > 0)
}
