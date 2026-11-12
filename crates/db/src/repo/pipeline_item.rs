//! Port of `repositories/pipeline/item.ts` (`pipelineItemRepository`).
//!
//! SQLite form: the row lock of `lockForUpdate` and the lock waits of the batch
//! statements do not exist (one writer); each batch still repeats its scope in the outer
//! WHERE. The board first page (`JOIN LATERAL` per stage) is one page query per stage of
//! the pipeline, in stage id order. Every UPDATE sets `"updatedAt"` (Drizzle `$onUpdate`).

use std::collections::HashSet;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{PipelineItem, PipelineItemAssignee};
use super::patch::{self, Patch};
use super::pipeline_item_cursor::{
    PipelineItemSort, clamp_page_limit, cursor_for, decode_pipeline_item_cursor,
    pipeline_item_after_cursor, pipeline_item_order_by, pipeline_item_sort_value_sql,
    sort_value_text,
};
use super::pipeline_item_filter::{PipelineItemFilter, build_pipeline_item_where};
use super::pipeline_item_cursor::bind1;
use super::sql::{Sql, opt_int};
use super::{Error, Json, Model, Result, find_one, id_list, new_id, now_ms, returning};

/// `PipelineItemInsert`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineItemInsert {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub stage_id: i64,
    pub contact_id: i64,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    /// Money x 10,000.
    pub value: Option<i64>,
    pub status: String,
    pub priority: String,
    pub due_date: Option<i64>,
    pub field_values: Json,
    pub created_by_id: Option<i64>,
    pub closed_by_id: Option<i64>,
    pub stage_entered_at: i64,
    pub status_changed_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub last_comment_at: Option<i64>,
    pub idempotency_key: Option<String>,
}

/// Result of `insertIdempotent`.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertedItem {
    pub row: PipelineItem,
    pub inserted: bool,
}

/// `PipelineItemPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineItemPage {
    pub items: Vec<PipelineItem>,
    pub next_cursor: Option<String>,
}

/// `PipelineBoardStagePage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineBoardStagePage {
    pub stage_id: i64,
    pub items: Vec<PipelineItem>,
    /// Resumes right after the last item of `items`; `None` when the stage fits.
    pub next_cursor: Option<String>,
}

/// `PipelineStageTotals` (`total_value` is money x 10,000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStageTotals {
    pub stage_id: i64,
    pub count: i64,
    pub total_value: i64,
}

/// Input of `listStagePage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListStagePageInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    /// `None` pages across every stage (public list); set for a board column.
    pub stage_id: Option<i64>,
    pub filter: PipelineItemFilter,
    pub sort: PipelineItemSort,
    pub limit: i64,
    pub cursor: Option<String>,
}

/// Input of `listBoardFirstPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardFirstPageInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub filter: PipelineItemFilter,
    pub sort: PipelineItemSort,
    pub per_stage_limit: i64,
}

/// Input of `moveStageItemsBatch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveStageItemsInput {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub from_stage_id: i64,
    pub to_stage_id: i64,
    pub limit: i64,
    pub now: i64,
}

/// `findById` (`pipeline_id` narrows when given).
pub fn find_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    pipeline_id: Option<i64>,
) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `findByIdempotencyKey`.
pub fn find_by_idempotency_key(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    idempotency_key: &str,
) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `lockForUpdate`: the item row (the write job is the lock).
pub fn lock_for_update(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `insertIdempotent`: inserts the item, or returns the one an earlier call with the same
/// idempotency key stored. Only that key's conflict is absorbed.
pub fn insert_idempotent(conn: &Connection, values: &PipelineItemInsert) -> Result<InsertedItem> {
    todo!("P3.3")
}

fn update_where(conn: &Connection, set: &Patch, workspace_id: i64, id: i64) -> Result<Option<PipelineItem>> {
    let mut sql = Sql::raw("UPDATE \"PipelineItem\" SET ");
    sql.append(patch::set_clause(PipelineItem::COLUMNS, set)?);
    sql.append(Sql::new(
        format!(
            " WHERE \"workspaceId\" = ? AND \"id\" = ? RETURNING {}",
            PipelineItem::columns_sql(None)
        ),
        vec![Value::Integer(workspace_id), Value::Integer(id)],
    ));
    sql.model::<PipelineItem>(conn)
}

/// `update`: sets the patch columns (and `updatedAt`).
pub fn update(conn: &Connection, workspace_id: i64, id: i64, values: &Patch) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `touch`: marks the item as changed without touching other columns.
pub fn touch(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `delete`.
pub fn delete(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    todo!("P3.3")
}

/// `findLatestOpenForContact`: the contact's most recently created open item.
pub fn find_latest_open_for_contact(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    contact_id: i64,
) -> Result<Option<PipelineItem>> {
    todo!("P3.3")
}

/// `countOpenForContact`: a contact's open items in a pipeline, optionally leaving one out.
pub fn count_open_for_contact(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    contact_id: i64,
    except_item_id: Option<i64>,
) -> Result<i64> {
    todo!("P3.3")
}

/// `listForContact`: every pipeline's items of one contact, newest first.
pub fn list_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    pipeline_ids: Option<&[i64]>,
    limit: i64,
) -> Result<Vec<PipelineItem>> {
    todo!("P3.3")
}

/// One page query: items matching `where_sql`, sorted, at most `fetch` rows, with the
/// sort value text of each row.
fn page_query(
    conn: &Connection,
    where_sql: Sql,
    sort: PipelineItemSort,
    fetch: i64,
) -> Result<Vec<(PipelineItem, Option<String>)>> {
    let n = PipelineItem::COLUMNS.len();
    let mut sql = Sql::raw(format!(
        "SELECT {}, {} FROM \"PipelineItem\" WHERE ",
        PipelineItem::columns_sql(Some("\"PipelineItem\"")),
        pipeline_item_sort_value_sql(sort)
    ));
    sql.append(where_sql);
    sql.push(&format!(
        " ORDER BY {} LIMIT ",
        pipeline_item_order_by(sort).join(", ")
    ));
    sql.push_bind(fetch);
    sql.query_map(conn, |row| {
        let item = PipelineItem::from_row(row)?;
        let raw: Option<i64> = row.get(n)?;
        Ok((item, sort_value_text(sort.key, raw)))
    })
}

fn cut_page(
    rows: Vec<(PipelineItem, Option<String>)>,
    limit: i64,
    sort: PipelineItemSort,
) -> (Vec<PipelineItem>, Option<String>) {
    let keep = usize::try_from(limit).unwrap_or(0);
    let more = rows.len() > keep;
    let mut rows = rows;
    rows.truncate(keep);
    let next = match rows.last() {
        Some((item, value)) if more => Some(cursor_for(sort, value.as_deref(), item.id)),
        _ => None,
    };
    (rows.into_iter().map(|(item, _)| item).collect(), next)
}

/// `listStagePage`: one keyset page of a stage, or of the whole pipeline when `stage_id`
/// is `None`. A malformed cursor fails with `InvalidPipelineCursorError`.
pub fn list_stage_page(conn: &Connection, input: &ListStagePageInput) -> Result<PipelineItemPage> {
    todo!("P3.3")
}

/// `listBoardFirstPage`: the first page of every stage of a pipeline (at most
/// `per_stage_limit` items each); stages with no matching item are absent.
pub fn list_board_first_page(
    conn: &Connection,
    input: &BoardFirstPageInput,
) -> Result<Vec<PipelineBoardStagePage>> {
    todo!("P3.3")
}

/// `listStageTotals`: count and summed value per stage.
pub fn list_stage_totals(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    filter: &PipelineItemFilter,
) -> Result<Vec<PipelineStageTotals>> {
    todo!("P3.3")
}

/// `countByStage`.
pub fn count_by_stage(conn: &Connection, workspace_id: i64, pipeline_id: i64, stage_id: i64) -> Result<i64> {
    todo!("P3.3")
}

const BATCH_SCOPE: &str =
    "\"workspaceId\" = ?1 AND \"pipelineId\" = ?2 AND (?3 IS NULL OR \"stageId\" = ?3)";

fn batch_ids_sql() -> String {
    format!("\"id\" IN (SELECT \"id\" FROM \"PipelineItem\" WHERE {BATCH_SCOPE} LIMIT ?4) AND {BATCH_SCOPE}")
}

/// `moveStageItemsBatch`: moves up to `limit` items of a stage to another; returns them.
pub fn move_stage_items_batch(conn: &Connection, input: &MoveStageItemsInput) -> Result<Vec<PipelineItem>> {
    todo!("P3.3")
}

/// `deleteStageItemsBatch`: deletes up to `limit` items of a stage; returns them.
pub fn delete_stage_items_batch(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    limit: i64,
) -> Result<Vec<PipelineItem>> {
    todo!("P3.3")
}

/// `deletePipelineItemsBatch`: deletes up to `limit` items of a pipeline; returns how many.
pub fn delete_pipeline_items_batch(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    limit: i64,
) -> Result<usize> {
    todo!("P3.3")
}

/// `listAssigneesForItems`: oldest first.
pub fn list_assignees_for_items(
    conn: &Connection,
    workspace_id: i64,
    item_ids: &[i64],
) -> Result<Vec<PipelineItemAssignee>> {
    todo!("P3.3")
}

/// `replaceAssignees`: makes the assignee set exactly the given targets; rows that stay
/// are untouched (their `createdAt` keeps ordering the primary assignee).
pub fn replace_assignees(
    conn: &Connection,
    workspace_id: i64,
    item_id: i64,
    workspace_member_ids: &[i64],
    inbox_team_ids: &[i64],
) -> Result<()> {
    todo!("P3.3")
}
