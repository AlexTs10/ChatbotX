//! `pipeline/item-content.service.ts` (comments, attachments) and the
//! `listComments` / `listActivity` reads of `item-query.service.ts`. Each
//! write locks the item row (one SQLite writer) and returns the realtime
//! events.

use std::collections::{HashMap, HashSet};

use chatbotx_db::Db;
use chatbotx_db::repo::models::{PipelineItemActivity, PipelineItemComment};
use chatbotx_db::repo::partials::pipeline::PIPELINE_LIMITS;
use chatbotx_db::repo::pipeline_activity::{self as activity_repo, ListActivityInput};
use chatbotx_db::repo::pipeline_item::{self as item_repo};
use chatbotx_db::repo::pipeline_item_content::{
    self as content_repo, PipelineCommentInsert, PipelineItemCommentPage,
};
use chatbotx_db::repo::pipeline_item_cursor::is_invalid_cursor;
use chatbotx_db::repo::pipeline_stage as stage_repo;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use super::access::{PipelineAccessScope, PipelineAction};
use super::item::{ITEM_NOT_FOUND, ItemMutation, assert_media_files, done, lock_item};
use super::mutation::{ActivityContext, PipelineActor, activity_row_for, item_realtime};
use super::service::{list_with_permissions, require_access_in};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `DETAIL_LIST_LIMIT`.
const DETAIL_LIST_LIMIT: i64 = 100;
/// `ACTIVITY_PAGE_SIZE`.
const ACTIVITY_PAGE_SIZE: i64 = 10;

/// The item an input names.
#[derive(Debug, Clone)]
pub struct ItemRef {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub scope: PipelineAccessScope,
}

/// `PipelineActivityView`: the row plus the names the scope may see.
#[derive(Debug, Clone)]
pub struct ActivityView {
    pub row: PipelineItemActivity,
    pub from_stage_name: Option<String>,
    pub to_stage_name: Option<String>,
    pub from_pipeline_name: Option<String>,
    pub pipeline_name: Option<String>,
}

/// `PipelineActivityPage` of views.
#[derive(Debug, Clone)]
pub struct ActivityViewPage {
    pub items: Vec<ActivityView>,
    pub next_cursor: Option<String>,
}

fn invalid_cursor(error: chatbotx_db::repo::Error) -> BusinessError {
    if is_invalid_cursor(&error) {
        BusinessError::validation("cursor", "Invalid cursor")
    } else {
        error.into()
    }
}

fn opt_int(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Integer)
}

/// `withLockedItem(input, "edit", ...)`: access, then the locked item.
fn locked_item(
    conn: &Connection,
    item: &ItemRef,
) -> Result<chatbotx_db::repo::models::PipelineItem> {
    require_access_in(
        conn,
        item.workspace_id,
        item.pipeline_id,
        &item.scope,
        PipelineAction::Edit,
    )?;
    lock_item(conn, item.workspace_id, item.pipeline_id, item.item_id)
}

fn write_activity(
    conn: &Connection,
    kind: &str,
    item: &chatbotx_db::repo::models::PipelineItem,
    actor: &PipelineActor,
    now: i64,
) -> Result<()> {
    let row = activity_row_for(
        kind,
        &ActivityContext {
            item,
            actor,
            now,
            from_pipeline_id: None,
        },
    );
    activity_repo::insert_many(conn, &[row])?;
    Ok(())
}

/// `addComment`: an idempotency key returns the first comment.
pub async fn add_comment(
    db: &Db,
    target: ItemRef,
    actor: PipelineActor,
    text: String,
    idempotency_key: Option<String>,
) -> Result<ItemMutation<PipelineItemComment>> {
    let (row, effects) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let item = locked_item(conn, &target)?;
        let inserted = content_repo::insert_comment_idempotent(
            conn,
            &PipelineCommentInsert {
                workspace_id: target.workspace_id,
                item_id: target.item_id,
                text,
                created_by_id: actor.user_id,
                idempotency_key,
            },
        )?;
        if !inserted.inserted {
            return Ok((inserted.row, Vec::new()));
        }
        let row = inserted.row;
        let latest = content_repo::latest_comment_at(conn, target.workspace_id, target.item_id)?;
        item_repo::update(
            conn,
            target.workspace_id,
            target.item_id,
            &vec![("lastCommentAt", opt_int(latest.or(Some(row.created_at))))],
        )?;
        write_activity(conn, "commentAdded", &item, &actor, row.created_at)?;
        Ok((row, vec![item_realtime(&item, "updated")]))
    })
    .await?;
    Ok(done(row, &effects))
}

/// `deleteComment`.
pub async fn delete_comment(
    db: &Db,
    target: ItemRef,
    actor: PipelineActor,
    comment_id: i64,
) -> Result<ItemMutation<()>> {
    let effects = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let item = locked_item(conn, &target)?;
        if !content_repo::delete_comment(conn, target.workspace_id, target.item_id, comment_id)? {
            return Err(BusinessError::not_found("Comment not found"));
        }
        let latest = content_repo::latest_comment_at(conn, target.workspace_id, target.item_id)?;
        item_repo::update(
            conn,
            target.workspace_id,
            target.item_id,
            &vec![("lastCommentAt", opt_int(latest))],
        )?;
        write_activity(
            conn,
            "commentDeleted",
            &item,
            &actor,
            chatbotx_db::repo::now_ms(),
        )?;
        Ok(vec![item_realtime(&item, "updated")])
    })
    .await?;
    Ok(done((), &effects))
}

/// `addAttachments`: files the item already has change nothing; over
/// `attachmentsPerItem` rolls the insert back (422).
pub async fn add_attachments(
    db: &Db,
    target: ItemRef,
    actor: PipelineActor,
    media_library_file_ids: Vec<i64>,
) -> Result<ItemMutation<()>> {
    let effects = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let item = locked_item(conn, &target)?;
        assert_media_files(conn, target.workspace_id, &media_library_file_ids)?;
        let inserted = content_repo::insert_attachments(
            conn,
            target.workspace_id,
            target.item_id,
            &media_library_file_ids,
        )?;
        if inserted == 0 {
            return Ok(Vec::new());
        }
        let limit = PIPELINE_LIMITS.attachments_per_item;
        if content_repo::count_attachments(conn, target.workspace_id, target.item_id)?
            > i64::try_from(limit).unwrap_or(i64::MAX)
        {
            return Err(BusinessError::validation(
                "mediaLibraryFileIds",
                format!("An item can have at most {limit} attachments"),
            ));
        }
        item_repo::touch(conn, target.workspace_id, target.item_id)?;
        write_activity(
            conn,
            "attachmentAdded",
            &item,
            &actor,
            chatbotx_db::repo::now_ms(),
        )?;
        Ok(vec![item_realtime(&item, "updated")])
    })
    .await?;
    Ok(done((), &effects))
}

/// `removeAttachment` (the media-library file stays).
pub async fn remove_attachment(
    db: &Db,
    target: ItemRef,
    actor: PipelineActor,
    media_library_file_id: i64,
) -> Result<ItemMutation<()>> {
    let effects = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let item = locked_item(conn, &target)?;
        if !content_repo::delete_attachment(
            conn,
            target.workspace_id,
            target.item_id,
            media_library_file_id,
        )? {
            return Err(BusinessError::not_found("Attachment not found"));
        }
        item_repo::touch(conn, target.workspace_id, target.item_id)?;
        write_activity(
            conn,
            "attachmentRemoved",
            &item,
            &actor,
            chatbotx_db::repo::now_ms(),
        )?;
        Ok(vec![item_realtime(&item, "updated")])
    })
    .await?;
    Ok(done((), &effects))
}

/// `requireItemView`.
fn require_item_view(conn: &Connection, target: &ItemRef) -> Result<()> {
    require_access_in(
        conn,
        target.workspace_id,
        target.pipeline_id,
        &target.scope,
        PipelineAction::View,
    )?;
    item_repo::find_by_id(
        conn,
        target.workspace_id,
        target.item_id,
        Some(target.pipeline_id),
    )?
    .ok_or_else(|| BusinessError::not_found(ITEM_NOT_FOUND))?;
    Ok(())
}

/// `listComments`: newest first, 100 a page.
pub async fn list_comments(
    db: &Db,
    target: ItemRef,
    cursor: Option<String>,
) -> Result<PipelineItemCommentPage> {
    tx::read(db, move |conn| {
        require_item_view(conn, &target)?;
        content_repo::list_comments(
            conn,
            target.workspace_id,
            target.item_id,
            DETAIL_LIST_LIMIT,
            cursor.as_deref(),
        )
        .map_err(invalid_cursor)
    })
    .await
}

/// `listActivity`: newest first, ten a page; `kinds` `None` = every kind.
/// Stage and pipeline names only for pipelines the scope can view.
pub async fn list_activity(
    db: &Db,
    target: ItemRef,
    cursor: Option<String>,
    kinds: Option<Vec<String>>,
) -> Result<ActivityViewPage> {
    let workspace_id = target.workspace_id;
    let scope = target.scope;
    let (page, stages) = tx::read(db, move |conn| {
        require_item_view(conn, &target)?;
        let page = activity_repo::list_for_item(
            conn,
            &ListActivityInput {
                workspace_id,
                item_id: target.item_id,
                limit: ACTIVITY_PAGE_SIZE,
                cursor,
                kinds,
            },
        )
        .map_err(invalid_cursor)?;
        let stage_ids: Vec<i64> = page
            .items
            .iter()
            .flat_map(|row| [row.from_stage_id, row.to_stage_id])
            .flatten()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let stages = stage_repo::list_names_by_ids(conn, workspace_id, &stage_ids)?;
        Ok((page, stages))
    })
    .await?;
    let pipeline_names: HashMap<i64, String> = list_with_permissions(db, workspace_id, scope)
        .await?
        .into_iter()
        .map(|entry| (entry.pipeline.pipeline.id, entry.pipeline.pipeline.name))
        .collect();
    let stage_names: HashMap<i64, String> = stages
        .into_iter()
        .filter(|stage| pipeline_names.contains_key(&stage.pipeline_id))
        .map(|stage| (stage.id, stage.name))
        .collect();
    let name_of =
        |names: &HashMap<i64, String>, id: Option<i64>| id.and_then(|id| names.get(&id).cloned());
    Ok(ActivityViewPage {
        items: page
            .items
            .into_iter()
            .map(|row| ActivityView {
                from_stage_name: name_of(&stage_names, row.from_stage_id),
                to_stage_name: name_of(&stage_names, row.to_stage_id),
                from_pipeline_name: name_of(&pipeline_names, row.from_pipeline_id),
                pipeline_name: name_of(&pipeline_names, Some(row.pipeline_id)),
                row,
            })
            .collect(),
        next_cursor: page.next_cursor,
    })
}
